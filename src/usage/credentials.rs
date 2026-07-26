//! Reads Claude Code's OAuth access token for the api usage source.
//!
//! Three places are tried in order, cheapest and least intrusive first: an env
//! var, the credentials file Claude Code writes on Linux (and on macOS when the
//! Keychain is disabled), and finally the macOS Keychain. Only the last one can
//! raise a password prompt, which is why it is last, and why the `auto` source
//! avoids this path entirely whenever local history is fresh.
//!
//! The Keychain is read by shelling out to `/usr/bin/security` rather than
//! linking `security-framework`, so the five cross-compiled release targets
//! don't need a macOS-only dependency. The call is array-based, per the repo's
//! shell-safety convention.

use anyhow::{anyhow, bail, Context, Result};
use std::path::PathBuf;

/// Env vars supplying the token directly, bypassing every file and the
/// Keychain. `CLAUDE_CODE_OAUTH_TOKEN` is Claude Code's own variable.
const TOKEN_ENVS: [&str; 2] = ["CCSM_USAGE_TOKEN", "CLAUDE_CODE_OAUTH_TOKEN"];

/// Keychain service name Claude Code stores its credentials under.
const KEYCHAIN_SERVICE: &str = "Claude Code-credentials";

/// Reads the OAuth access token, or explains every place that was tried.
pub fn oauth_token() -> Result<String> {
    if let Some(token) = from_env() {
        return Ok(token);
    }
    match from_credentials_file() {
        Ok(token) => Ok(token),
        Err(file_err) => from_keychain().map_err(|keychain_err| {
            anyhow!("no Claude credentials found ({file_err:#}; {keychain_err:#})")
        }),
    }
}

/// First non-empty token env var, if any.
fn from_env() -> Option<String> {
    TOKEN_ENVS
        .iter()
        .filter_map(|name| std::env::var(name).ok())
        .find(|token| !token.is_empty())
}

/// Path to Claude Code's credentials file.
fn credentials_path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".claude").join(".credentials.json"))
}

/// Reads the token from `~/.claude/.credentials.json`.
fn from_credentials_file() -> Result<String> {
    let path = credentials_path().ok_or_else(|| anyhow!("no home directory"))?;
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("could not read {}", path.display()))?;
    token_from_credentials_json(&text)
}

/// Reads the token from the macOS Keychain. Non-macOS platforms have no
/// Keychain, so this is the end of the line there.
fn from_keychain() -> Result<String> {
    if !cfg!(target_os = "macos") {
        bail!("no Keychain on this platform");
    }

    let user = std::env::var("USER").unwrap_or_else(|_| "unknown".to_string());
    let output = std::process::Command::new("/usr/bin/security")
        .args(["find-generic-password", "-s", KEYCHAIN_SERVICE, "-a", &user, "-w"])
        .output()
        .context("could not run /usr/bin/security")?;

    if !output.status.success() {
        // stderr is the only detail `security` gives, and it never echoes the
        // secret itself, so it is safe to surface.
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("Keychain lookup failed: {}", stderr.trim());
    }

    token_from_credentials_json(String::from_utf8_lossy(&output.stdout).trim())
}

/// Extracts `claudeAiOauth.accessToken` from a credentials blob. Pure, so the
/// shape is testable without a Keychain or a Claude install. The error never
/// includes the input, which is a secret.
fn token_from_credentials_json(text: &str) -> Result<String> {
    let parsed: serde_json::Value =
        serde_json::from_str(text).map_err(|e| anyhow!("credentials are not valid JSON: {e}"))?;

    parsed["claudeAiOauth"]["accessToken"]
        .as_str()
        .filter(|token| !token.is_empty())
        .map(|token| token.to_string())
        .ok_or_else(|| anyhow!("credentials have no claudeAiOauth.accessToken"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_access_token() {
        let json = r#"{"claudeAiOauth":{"accessToken":"sk-ant-oat-test","expiresAt":1}}"#;
        assert_eq!(token_from_credentials_json(json).unwrap(), "sk-ant-oat-test");
    }

    #[test]
    fn rejects_a_missing_token() {
        let err = token_from_credentials_json(r#"{"claudeAiOauth":{}}"#).expect_err("should fail");
        assert!(err.to_string().contains("accessToken"), "got: {err}");
    }

    #[test]
    fn rejects_an_empty_token() {
        assert!(token_from_credentials_json(r#"{"claudeAiOauth":{"accessToken":""}}"#).is_err());
    }

    #[test]
    fn rejects_malformed_json() {
        assert!(token_from_credentials_json("not json").is_err());
    }

    #[test]
    fn the_error_never_echoes_the_credentials() {
        let err = token_from_credentials_json(r#"{"secret":"sk-ant-oat-leak"}"#)
            .expect_err("should fail");
        assert!(
            !format!("{err:#}").contains("sk-ant-oat-leak"),
            "credentials must not appear in errors: {err:#}"
        );
    }
}
