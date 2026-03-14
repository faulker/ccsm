#Requires -Version 5.1
<#
.SYNOPSIS
    Install ccsm (Claude Code Session Manager) on Windows.
.DESCRIPTION
    Downloads the latest ccsm release from GitHub and installs it to a user-local bin directory.
#>

$ErrorActionPreference = 'Stop'

$Repo = 'faulker/ccsm'
$BinaryName = 'ccsm'
$InstallDir = Join-Path $env:LOCALAPPDATA 'ccsm'

# Detect architecture
$Arch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
switch ($Arch) {
    'X64'   { $Target = 'x86_64-pc-windows-msvc' }
    'Arm64' { $Target = 'aarch64-pc-windows-msvc' }
    default {
        Write-Error "Unsupported architecture: $Arch"
        exit 1
    }
}

Write-Host 'Detecting latest release...'
$Release = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest" -Headers @{ 'User-Agent' = 'ccsm-installer' }
$Tag = $Release.tag_name

if (-not $Tag) {
    Write-Error 'Could not determine latest release'
    exit 1
}

$AssetName = "$BinaryName-$Tag-$Target.zip"
$DownloadUrl = "https://github.com/$Repo/releases/download/$Tag/$AssetName"

Write-Host "Downloading $BinaryName $Tag for $Target..."

$TempDir = Join-Path ([System.IO.Path]::GetTempPath()) ([System.IO.Path]::GetRandomFileName())
New-Item -ItemType Directory -Path $TempDir | Out-Null

try {
    $ZipPath = Join-Path $TempDir $AssetName
    Invoke-WebRequest -Uri $DownloadUrl -OutFile $ZipPath -UseBasicParsing

    Write-Host 'Extracting...'
    Expand-Archive -Path $ZipPath -DestinationPath $TempDir -Force

    $ExtractedBinary = Get-ChildItem -Path $TempDir -Filter "$BinaryName.exe" -Recurse -File | Select-Object -First 1
    if (-not $ExtractedBinary) {
        Write-Error 'Binary not found in archive'
        exit 1
    }
    $ExtractedBinary = $ExtractedBinary.FullName

    New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
    Copy-Item -Path $ExtractedBinary -Destination (Join-Path $InstallDir "$BinaryName.exe") -Force

    # Add to user PATH if not already present
    $UserPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    if ($UserPath -notlike "*$InstallDir*") {
        [Environment]::SetEnvironmentVariable('Path', "$InstallDir;$UserPath", 'User')
        Write-Host ''
        Write-Host "$InstallDir has been added to your user PATH."
        Write-Host 'Restart your terminal for the change to take effect.'
    }

    Write-Host "Installed $BinaryName $Tag to $InstallDir\$BinaryName.exe"
    Write-Host "Done! Run '$BinaryName' to get started."
}
finally {
    Remove-Item -Recurse -Force $TempDir -ErrorAction SilentlyContinue
}
