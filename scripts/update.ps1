#Requires -Version 5.1
<#
.SYNOPSIS
    Alfred Update Script for Windows
.DESCRIPTION
    Checks for newer version and updates the installation
#>

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

# Get current installed version
function Get-InstalledVersion {
    $binaryPath = Join-Path $InstallDir "alfred.exe"
    if (Test-Path $binaryPath) {
        try {
            $output = & $binaryPath --version 2>$null
            if ($output -match '(\d{8})') {
                return $Matches[1]
            }
        } catch {
            # Ignore errors
        }
    }
    return "none"
}

# Get latest version from GitHub
function Get-LatestVersion {
    $release = Invoke-RestMethod -Uri "$GitHubApi/releases/latest"
    $version = $release.tag_name -replace '^v', ''
    return $version
}

# Detect platform
function Get-Platform {
    return "windows-x64"
}

# Download and install update
function Install-Update {
    param([string]$Version, [string]$Platform)

    $archiveName = "alfred-v$Version-$Platform.zip"
    $downloadUrl = "https://github.com/$GitHubRepo/releases/download/v$Version/$archiveName"
    $checksumUrl = "$downloadUrl.sha256"

    $tempDir = Join-Path $env:TEMP "alfred-update-$(Get-Random)"
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
        Write-Error "Failed to extract update"
        Remove-Item -Path $tempDir -Recurse -Force
        exit 1
    }

    # Stop running server if any
    $process = Get-Process -Name "alfred" -ErrorAction SilentlyContinue
    if ($process) {
        Write-Info "Stopping running Alfred server..."
        Stop-Process -Name "alfred" -Force -ErrorAction SilentlyContinue
        Start-Sleep -Seconds 2
    }

    # Update binary
    Write-Info "Updating binary..."
    $destBinary = Join-Path $InstallDir "alfred.exe"
    Copy-Item -Path (Join-Path $extractedDir.FullName "alfred.exe") -Destination $destBinary -Force

    # Check for new config templates
    Write-Info "Checking for config updates..."
    $configFile = Join-Path $ConfigDir "config.toml"
    if (Test-Path (Join-Path $extractedDir.FullName "config.toml.example")) {
        if (Test-Path $configFile) {
            Write-Warn "Config file exists, skipping template update"
            Write-Info "Your config: $configFile"
        } else {
            New-Item -ItemType Directory -Path $ConfigDir -Force | Out-Null
            Copy-Item -Path (Join-Path $extractedDir.FullName "config.toml.example") -Destination $configFile
            Write-Info "Created new config: $configFile"
        }
    }

    # Check for prompt updates
    New-Item -ItemType Directory -Path "$ConfigDir\prompts" -Force | Out-Null
    foreach ($file in @("system.md", "user.md")) {
        $sourceFile = Join-Path $extractedDir.FullName "prompts\$file.example"
        $destFile = Join-Path "$ConfigDir\prompts" $file
        if (Test-Path $sourceFile) {
            if (Test-Path $destFile) {
                Write-Warn "Prompt file $file exists, skipping"
            } else {
                Copy-Item -Path $sourceFile -Destination $destFile
            }
        }
    }

    # Cleanup
    Remove-Item -Path $tempDir -Recurse -Force

    Write-Info "Update complete!"
}

# Main update process
function Update-Alfred {
    Write-Info "Checking for Alfred updates..."
    Write-Host ""

    $installedVersion = Get-InstalledVersion
    $latestVersion = Get-LatestVersion

    Write-Info "Installed version: $installedVersion"
    Write-Info "Latest version: $latestVersion"

    if ($installedVersion -eq $latestVersion) {
        Write-Info "Already up to date!"
        return
    }

    Write-Host ""
    Write-Warn "New version available: $latestVersion"
    $confirm = Read-Host "Do you want to update? (y/N)"

    if ($confirm -notmatch '^[Yy]$') {
        Write-Info "Update cancelled"
        return
    }

    $platform = Get-Platform
    Install-Update -Version $latestVersion -Platform $platform

    Write-Host ""
    Write-Info "Restarting Alfred server..."
    $binaryPath = Join-Path $InstallDir "alfred.exe"
    if (Test-Path $binaryPath) {
        Start-Process -FilePath $binaryPath -WindowStyle Hidden
        Write-Info "Alfred server started"
    }
}

# Run update
Update-Alfred
