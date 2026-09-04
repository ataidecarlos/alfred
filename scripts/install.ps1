#Requires -Version 5.1
<#
.SYNOPSIS
    Alfred Installer for Windows
.DESCRIPTION
    Downloads and installs Alfred AI agent server
.PARAMETER Version
    Version to install (default: latest)
.EXAMPLE
    .\install.ps1
    .\install.ps1 -Version 20260904
#>

param(
    [string]$Version = "latest"
)

$ErrorActionPreference = "Stop"

# Configuration
$GitHubRepo = "ataidecarlos/alfred"
$GitHubApi = "https://api.github.com/repos/$GitHubRepo"
$InstallDir = "$env:LOCALAPPDATA\bin"
$ConfigDir = "$env:APPDATA\alfred"
$DataDir = "$env:LOCALAPPDATA\alfred"
$CacheDir = "$env:LOCALAPPDATA\alfred\cache"

# Colors
function Write-Info { Write-Host -ForegroundColor Green "[INFO] $args" }
function Write-Warn { Write-Host -ForegroundColor Yellow "[WARN] $args" }
function Write-Error { Write-Host -ForegroundColor Red "[ERROR] $args" }

# Detect architecture
function Get-Platform {
    $arch = [System.Environment]::Is64BitOperatingSystem
    if ($arch) {
        return "windows-x64"
    } else {
        Write-Error "Unsupported architecture: x86"
        exit 1
    }
}

# Get latest version from GitHub
function Get-LatestVersion {
    if ($Version -eq "latest") {
        $release = Invoke-RestMethod -Uri "$GitHubApi/releases/latest"
        $Version = $release.tag_name -replace '^v', ''
    }
    Write-Info "Version: $Version"
    return $Version
}

# Download release
function Get-Release {
    param([string]$Version, [string]$Platform)

    $archiveName = "alfred-v$Version-$Platform.zip"
    $downloadUrl = "https://github.com/$GitHubRepo/releases/download/v$Version/$archiveName"
    $checksumUrl = "$downloadUrl.sha256"

    $tempDir = Join-Path $env:TEMP "alfred-install-$(Get-Random)"
    New-Item -ItemType Directory -Path $tempDir -Force | Out-Null

    Write-Info "Downloading $downloadUrl..."

    # Download archive
    $archivePath = Join-Path $tempDir $archiveName
    Invoke-WebRequest -Uri $downloadUrl -OutFile $archivePath

    # Download and verify checksum
    $checksumPath = "$archivePath.sha256"
    Invoke-WebRequest -Uri $checksumUrl -OutFile $checksumPath

    $expectedHash = (Get-Content $checksumPath -Raw).Split(' ')[0].Trim()
    $actualHash = (Get-FileHash -Path $archivePath -Algorithm SHA256).Hash.ToLower()

    if ($expectedHash -ne $actualHash) {
        Write-Error "Checksum verification failed!"
        Remove-Item -Path $tempDir -Recurse -Force
        exit 1
    }

    Write-Info "Checksum verified"

    # Extract archive
    Write-Info "Extracting..."
    Expand-Archive -Path $archivePath -DestinationPath $tempDir -Force

    # Find extracted directory
    $extractedDir = Get-ChildItem -Path $tempDir -Directory -Filter "alfred-*" | Select-Object -First 1

    if (-not $extractedDir) {
        Write-Error "Failed to extract release"
        Remove-Item -Path $tempDir -Recurse -Force
        exit 1
    }

    return $extractedDir.FullName
}

# Install binary
function Install-Binary {
    param([string]$SourceDir)

    Write-Info "Installing binary to $InstallDir..."

    New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null

    # Copy binary (renamed from alfred-v* to alfred.exe)
    $sourceBinary = Join-Path $SourceDir "alfred.exe"
    $destBinary = Join-Path $InstallDir "alfred.exe"

    Copy-Item -Path $sourceBinary -Destination $destBinary -Force

    Write-Info "Binary installed: $destBinary"
}

# Install config files
function Install-Config {
    param([string]$SourceDir)

    Write-Info "Installing config files to $ConfigDir..."

    New-Item -ItemType Directory -Path $ConfigDir -Force | Out-Null
    New-Item -ItemType Directory -Path "$ConfigDir\prompts" -Force | Out-Null

    # Copy config template (skip if exists)
    $configFile = Join-Path $ConfigDir "config.toml"
    if (-not (Test-Path $configFile)) {
        Copy-Item -Path (Join-Path $SourceDir "config.toml.example") -Destination $configFile
        Write-Info "Created config: $configFile"
        Write-Warn "Please edit $configFile and add your API keys"
    } else {
        Write-Info "Config already exists, skipping"
    }

    # Copy prompt templates (skip if exists)
    foreach ($file in @("system.md", "user.md")) {
        $destFile = Join-Path "$ConfigDir\prompts" $file
        if (-not (Test-Path $destFile)) {
            $sourceFile = Join-Path $SourceDir "prompts\$file.example"
            if (Test-Path $sourceFile) {
                Copy-Item -Path $sourceFile -Destination $destFile
            }
        }
    }
}

# Create data directories
function New-DataDirs {
    Write-Info "Creating data directories..."

    New-Item -ItemType Directory -Path $DataDir -Force | Out-Null
    New-Item -ItemType Directory -Path $CacheDir -Force | Out-Null

    Write-Info "Data directory: $DataDir"
    Write-Info "Cache directory: $CacheDir"
}

# Check PATH
function Test-PathEnv {
    $currentPath = [Environment]::GetEnvironmentVariable("Path", "User")
    if ($currentPath -notlike "*$InstallDir*") {
        Write-Warn "$InstallDir is not in your PATH"
        Write-Info "Adding to user PATH..."

        $newPath = "$InstallDir;$currentPath"
        [Environment]::SetEnvironmentVariable("Path", $newPath, "User")
        Write-Info "PATH updated. Restart your terminal to use 'alfred'."
    }
}

# Cleanup
function Remove-Temp {
    param([string]$Path)
    Remove-Item -Path $Path -Recurse -Force -ErrorAction SilentlyContinue
}

# Main installation
function Install-Alfred {
    Write-Info "Installing Alfred..."
    Write-Host ""

    $platform = Get-Platform
    $version = Get-LatestVersion

    $extractedDir = Get-Release -Version $version -Platform $platform

    Install-Binary -SourceDir $extractedDir
    Install-Config -SourceDir $extractedDir
    New-DataDirs

    # Cleanup
    $tempParent = Split-Path $extractedDir -Parent
    Remove-Temp -Path $tempParent

    Write-Host ""
    Write-Info "Installation complete!"
    Write-Host ""
    Write-Info "Next steps:"
    Write-Info "  1. Edit $ConfigDir\config.toml with your API keys"
    Write-Info "  2. Run 'alfred' to start the server"
    Write-Info "  3. Run 'alfred --tui' to start the terminal UI"
    Write-Host ""

    Test-PathEnv
}

# Run installation
Install-Alfred
