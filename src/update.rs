use anyhow::{Context, Result};
use serde::Deserialize;
use std::cmp::Ordering;
use std::io::Read;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        self.major
            .cmp(&other.major)
            .then(self.minor.cmp(&other.minor))
            .then(self.patch.cmp(&other.patch))
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct UpdateInfo {
    pub current: Version,
    pub latest: Version,
    pub download_url: String,
    pub tag: String,
}

#[derive(Debug, Clone)]
pub enum UpdateStatus {
    None,
    Available(UpdateInfo),
    Downloading,
    Done(String),
    Failed(String),
}

#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: String,
    assets: Vec<ReleaseAsset>,
}

#[derive(Deserialize)]
struct ReleaseAsset {
    name: String,
    browser_download_url: String,
}

pub fn parse_version(s: &str) -> Option<Version> {
    let s = s.strip_prefix('v').unwrap_or(s);
    let parts: Vec<&str> = s.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    Some(Version {
        major: parts[0].parse().ok()?,
        minor: parts[1].parse().ok()?,
        patch: parts[2].parse().ok()?,
    })
}

pub fn asset_name_for_current_platform(tag: &str) -> Option<String> {
    let (target, ext) = match (std::env::consts::ARCH, std::env::consts::OS) {
        ("aarch64", "macos") => ("aarch64-apple-darwin", "tar.gz"),
        ("x86_64", "macos") => ("x86_64-apple-darwin", "tar.gz"),
        ("x86_64", "linux") => ("x86_64-unknown-linux-gnu", "tar.gz"),
        ("aarch64", "linux") => ("aarch64-unknown-linux-gnu", "tar.gz"),
        ("x86_64", "windows") => ("x86_64-pc-windows-msvc", "zip"),
        _ => return None,
    };
    Some(format!("ccsm-{}-{}.{}", tag, target, ext))
}

pub fn check_for_update() -> Option<UpdateInfo> {
    let current = parse_version(env!("CARGO_PKG_VERSION"))?;

    let resp = ureq::get("https://api.github.com/repos/faulker/ccsm/releases/latest")
        .set("User-Agent", "ccsm-update-checker")
        .timeout(std::time::Duration::from_secs(3))
        .call()
        .ok()?;

    let release: GitHubRelease = resp.into_json().ok()?;
    let latest = parse_version(&release.tag_name)?;

    if latest <= current {
        return None;
    }

    let asset_name = asset_name_for_current_platform(&release.tag_name)?;
    let asset = release.assets.iter().find(|a| a.name == asset_name)?;

    Some(UpdateInfo {
        current,
        latest,
        download_url: asset.browser_download_url.clone(),
        tag: release.tag_name,
    })
}

pub fn perform_update(info: &UpdateInfo) -> Result<()> {
    const MAX_DOWNLOAD_SIZE: u64 = 50 * 1024 * 1024;

    let resp = ureq::get(&info.download_url)
        .set("User-Agent", "ccsm-update-checker")
        .timeout(std::time::Duration::from_secs(60))
        .call()
        .context("Failed to download update")?;

    let mut bytes = Vec::new();
    resp.into_reader()
        .take(MAX_DOWNLOAD_SIZE)
        .read_to_end(&mut bytes)
        .context("Failed to read update response")?;
    if bytes.len() as u64 >= MAX_DOWNLOAD_SIZE {
        anyhow::bail!("Download exceeded maximum allowed size");
    }

    let temp_dir = tempfile::tempdir().context("Failed to create temp directory")?;

    if info.download_url.ends_with(".zip") {
        extract_zip(&bytes, temp_dir.path())?;
    } else {
        extract_tar_gz(&bytes, temp_dir.path())?;
    }

    // Find the binary in the extracted archive
    let new_binary = find_binary(temp_dir.path()).context("Could not find ccsm binary in archive")?;

    let current_exe = std::env::current_exe().context("Failed to determine current executable")?;

    // Replace the current binary: rename old, copy new, remove old
    let backup = current_exe.with_extension("old");
    std::fs::rename(&current_exe, &backup).context("Failed to back up current binary")?;

    if let Err(e) = std::fs::copy(&new_binary, &current_exe) {
        // Restore backup on failure
        if let Err(restore_err) = std::fs::rename(&backup, &current_exe) {
            return Err(e).context(format!(
                "Failed to install new binary AND failed to restore backup: {}",
                restore_err
            ));
        }
        return Err(e).context("Failed to install new binary (backup restored)");
    }

    // Set executable permissions
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&current_exe, std::fs::Permissions::from_mode(0o755))
            .context("Failed to set executable permissions")?;
    }

    let _ = std::fs::remove_file(&backup);
    // temp_dir is cleaned up automatically on drop

    Ok(())
}

fn extract_tar_gz(bytes: &[u8], dest: &std::path::Path) -> Result<()> {
    let decoder = flate2::read::GzDecoder::new(bytes);
    let mut archive = tar::Archive::new(decoder);

    for entry in archive.entries().context("Failed to read archive entries")? {
        let mut entry = entry.context("Failed to read archive entry")?;
        let path = entry.path().context("Failed to get entry path")?;
        if path.is_absolute()
            || path.components().any(|c| {
                matches!(
                    c,
                    std::path::Component::ParentDir | std::path::Component::RootDir
                )
            })
        {
            anyhow::bail!("Archive contains potentially malicious path: {:?}", path);
        }
        entry.unpack_in(dest).context("Failed to extract entry")?;
    }
    Ok(())
}

fn extract_zip(bytes: &[u8], dest: &std::path::Path) -> Result<()> {
    let cursor = std::io::Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(cursor).context("Failed to read zip archive")?;

    for i in 0..archive.len() {
        let mut file = archive.by_index(i).context("Failed to read zip entry")?;
        let Some(path) = file.enclosed_name() else {
            anyhow::bail!("Zip contains potentially malicious path");
        };
        let out_path = dest.join(path);

        if file.is_dir() {
            std::fs::create_dir_all(&out_path).context("Failed to create directory")?;
        } else {
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent).context("Failed to create parent directory")?;
            }
            let mut out_file =
                std::fs::File::create(&out_path).context("Failed to create extracted file")?;
            std::io::copy(&mut file, &mut out_file).context("Failed to write extracted file")?;
        }
    }
    Ok(())
}

fn find_binary(dir: &std::path::Path) -> Option<std::path::PathBuf> {
    let binary_name = if cfg!(windows) { "ccsm.exe" } else { "ccsm" };

    // Check directly in the temp dir
    let direct = dir.join(binary_name);
    if direct.exists() {
        return Some(direct);
    }
    // Check one level deep (archives often have a subdirectory)
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let nested = path.join(binary_name);
                if nested.exists() {
                    return Some(nested);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_version_basic() {
        assert_eq!(
            parse_version("1.2.3"),
            Some(Version {
                major: 1,
                minor: 2,
                patch: 3
            })
        );
    }

    #[test]
    fn test_parse_version_with_v_prefix() {
        assert_eq!(
            parse_version("v0.1.0"),
            Some(Version {
                major: 0,
                minor: 1,
                patch: 0
            })
        );
    }

    #[test]
    fn test_parse_version_invalid() {
        assert_eq!(parse_version(""), None);
        assert_eq!(parse_version("1.2"), None);
        assert_eq!(parse_version("not.a.version"), None);
        assert_eq!(parse_version("1.2.3.4"), None);
    }

    #[test]
    fn test_version_ordering() {
        let v010 = Version {
            major: 0,
            minor: 1,
            patch: 0,
        };
        let v011 = Version {
            major: 0,
            minor: 1,
            patch: 1,
        };
        let v020 = Version {
            major: 0,
            minor: 2,
            patch: 0,
        };
        let v100 = Version {
            major: 1,
            minor: 0,
            patch: 0,
        };

        assert!(v010 < v011);
        assert!(v011 < v020);
        assert!(v020 < v100);
        assert!(v010 == v010);
    }

    #[test]
    fn test_version_display() {
        let v = Version {
            major: 1,
            minor: 2,
            patch: 3,
        };
        assert_eq!(format!("{}", v), "1.2.3");
    }

    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
    fn test_asset_name_for_current_platform() {
        let name = asset_name_for_current_platform("v0.2.0");
        let name = name.expect("Expected Some on supported platform");
        assert!(name.starts_with("ccsm-v0.2.0-"));
        assert!(name.ends_with(".tar.gz") || name.ends_with(".zip"));
    }

    #[test]
    fn test_find_binary_direct() {
        let dir = tempfile::tempdir().unwrap();
        let binary_name = if cfg!(windows) { "ccsm.exe" } else { "ccsm" };
        std::fs::write(dir.path().join(binary_name), b"fake").unwrap();
        let found = find_binary(dir.path());
        assert!(found.is_some());
        assert!(found.unwrap().ends_with(binary_name));
    }

    #[test]
    fn test_find_binary_nested() {
        let dir = tempfile::tempdir().unwrap();
        let binary_name = if cfg!(windows) { "ccsm.exe" } else { "ccsm" };
        let sub = dir.path().join("subdir");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join(binary_name), b"fake").unwrap();
        let found = find_binary(dir.path());
        assert!(found.is_some());
        assert!(found.unwrap().ends_with(binary_name));
    }

    #[test]
    fn test_find_binary_missing() {
        let dir = tempfile::tempdir().unwrap();
        assert!(find_binary(dir.path()).is_none());
    }

    #[test]
    fn test_extract_tar_gz() {
        let dir = tempfile::tempdir().unwrap();
        // Create a tar.gz with a single file
        let mut buf = Vec::new();
        {
            let encoder = flate2::write::GzEncoder::new(&mut buf, flate2::Compression::default());
            let mut tar_builder = tar::Builder::new(encoder);
            let data = b"hello";
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            tar_builder.append_data(&mut header, "testfile", &data[..]).unwrap();
            tar_builder.into_inner().unwrap().finish().unwrap();
        }
        extract_tar_gz(&buf, dir.path()).unwrap();
        assert!(dir.path().join("testfile").exists());
    }

    #[test]
    fn test_extract_zip() {
        let dir = tempfile::tempdir().unwrap();
        let mut buf = Vec::new();
        {
            let cursor = std::io::Cursor::new(&mut buf);
            let mut writer = zip::ZipWriter::new(cursor);
            let options = zip::write::SimpleFileOptions::default();
            writer.start_file("testfile.txt", options).unwrap();
            std::io::Write::write_all(&mut writer, b"hello zip").unwrap();
            writer.finish().unwrap();
        }
        extract_zip(&buf, dir.path()).unwrap();
        assert!(dir.path().join("testfile.txt").exists());
    }
}
