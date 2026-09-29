#Requires -Version 5.1
<#
.SYNOPSIS
    Alfred installer for Windows — the two-part product: the `alfred` binary
    plus Pi (https://github.com/earendil-works/pi), which Alfred runs as a
    subprocess.
.DESCRIPTION
    Five idempotent steps:
      1. Ensure Node >= 22.19, or fetch the standalone Pi binary.
      2. Install Pi if `pi --version` fails.
      3. Install the `alfred` release binary.
      4. Seed %USERPROFILE%\.alfred\config from the release's examples.
      5. Print the next steps.
    Each step re-checks its own precondition, so a second run is a no-op.
.PARAMETER Version
    Alfred release to install (default: latest).
.PARAMETER Token
    GitHub personal access token for a private repo (optional).
.EXAMPLE
    .\install.ps1
    .\install.ps1 -Version 2026.09.04
#>
param(
    [string]$Version = "latest",
    [string]$Token = ""
)

$ErrorActionPreference = "Stop"

$NodeMinMajor = 22
$NodeMinMinor = 19
$PiPackage = "@earendil-works/pi-coding-agent"
$PiReleaseBase = "https://github.com/earendil-works/pi/releases/latest/download"

$GitHubRepo = "ataidecarlos/alfred"
$GitHubApi = "https://api.github.com/repos/$GitHubRepo"
$AlfredHome = "$env:USERPROFILE\.alfred"
$ConfigDir = "$AlfredHome\config"
$InstallDir = "$env:LOCALAPPDATA\bin"
$PiStandaloneDir = "$AlfredHome\pi-standalone"
$AlfredVersionFile = "$AlfredHome\installed-version"

# Only windows-x64 is published today; ARM64 runs it under emulation.
$AlfredPlatform = "windows-x64"

function Write-Info { param([string]$Message) Write-Host -ForegroundColor Green "[INFO] $Message" }
function Write-Warn { param([string]$Message) Write-Host -ForegroundColor Yellow "[WARN] $Message" }
function Write-Fail { param([string]$Message) Write-Host -ForegroundColor Red "[ERROR] $Message"; exit 1 }

function Get-GitHubHeaders {
    $headers = @{ "Accept" = "application/vnd.github.v3+json" }
    if ($Token) {
        $headers["Authorization"] = "token $Token"
    } elseif ($env:GH_PAT) {
        $headers["Authorization"] = "token $env:GH_PAT"
    }
    return $headers
}

function Get-Arch {
    if ($env:PROCESSOR_ARCHITECTURE -eq "ARM64" -or $env:PROCESSOR_ARCHITEW6432 -eq "ARM64") {
        return "arm64"
    }
    return "x64"
}

# ── Step 1: ensure a way to run Pi ────────────────────────────────────

function Test-NodeVersion {
    if (-not (Get-Command node -ErrorAction SilentlyContinue)) { return $false }
    if (-not (Get-Command npm -ErrorAction SilentlyContinue)) { return $false }
    try { $raw = (& node --version).Trim() } catch { return $false }
    if ($raw -notmatch '^v?(\d+)\.(\d+)') { return $false }
    $major = [int]$Matches[1]
    $minor = [int]$Matches[2]
    if ($major -gt $NodeMinMajor) { return $true }
    return ($major -eq $NodeMinMajor -and $minor -ge $NodeMinMinor)
}

function Test-PiOnPath {
    if (-not (Get-Command pi -ErrorAction SilentlyContinue)) { return $false }
    try { & pi --version *> $null; return ($LASTEXITCODE -eq 0) } catch { return $false }
}

function Test-StandalonePi {
    $exe = Join-Path $PiStandaloneDir "pi.exe"
    if (-not (Test-Path $exe)) { return $false }
    try { & $exe --version *> $null; return ($LASTEXITCODE -eq 0) } catch { return $false }
}

function Install-PiPrerequisite {
    if (Test-PiOnPath) {
        Write-Info "Pi already installed: $((& pi --version) -join ' ')"
        $script:PiMethod = "skip"
        return
    }
    if (Test-StandalonePi) {
        Write-Info "Standalone Pi already present: $PiStandaloneDir\pi.exe"
        $script:PiMethod = "binary"
        return
    }
    if (Test-NodeVersion) {
        Write-Info "Node $((& node --version).Trim()) meets the >= $NodeMinMajor.$NodeMinMinor requirement"
        $script:PiMethod = "npm"
        return
    }

    Write-Warn "Node >= $NodeMinMajor.$NodeMinMinor with npm not found; using the standalone Pi binary"
    $script:PiMethod = "binary"
    $script:PiArchive = "pi-windows-$(Get-Arch).zip"
    $script:PiTmp = Join-Path $env:TEMP "pi-install-$(Get-Random)"
    New-Item -ItemType Directory -Path $script:PiTmp -Force | Out-Null

    Write-Info "Downloading $($script:PiArchive)..."
    try {
        Invoke-WebRequest -Uri "$PiReleaseBase/$($script:PiArchive)" -OutFile "$($script:PiTmp)\$($script:PiArchive)" -UseBasicParsing -ErrorAction Stop
    } catch {
        Write-Fail "failed to download $($script:PiArchive) from $PiReleaseBase : $_"
    }

    try {
        Invoke-WebRequest -Uri "$PiReleaseBase/SHA256SUMS" -OutFile "$($script:PiTmp)\SHA256SUMS" -UseBasicParsing -ErrorAction Stop
        $line = Get-Content "$($script:PiTmp)\SHA256SUMS" | Where-Object { $_ -like "*$($script:PiArchive)" } | Select-Object -First 1
        if ($line) {
            $expected = ($line -split '\s+')[0].Trim().ToLower()
            $actual = (Get-FileHash "$($script:PiTmp)\$($script:PiArchive)" -Algorithm SHA256).Hash.ToLower()
            if ($expected -ne $actual) { Write-Fail "checksum verification failed for $($script:PiArchive)" }
        } else {
            Write-Warn "No checksum published for $($script:PiArchive); skipping verification"
        }
    } catch {
        Write-Warn "Pi SHA256SUMS unavailable; skipping checksum verification"
    }
}

# ── Step 2: install Pi ────────────────────────────────────────────────

function Install-Pi {
    if (Test-PiOnPath) { return }

    switch ($script:PiMethod) {
        "npm" {
            Write-Info "Installing $PiPackage with npm..."
            & npm install -g --ignore-scripts --no-fund --no-audit $PiPackage
            if ($LASTEXITCODE -ne 0) { Write-Fail "npm install of $PiPackage failed" }
        }
        "binary" {
            if (-not (Test-StandalonePi)) {
                Write-Info "Unpacking $($script:PiArchive) into $PiStandaloneDir..."
                New-Item -ItemType Directory -Path $PiStandaloneDir -Force | Out-Null
                Expand-Archive -Path "$($script:PiTmp)\$($script:PiArchive)" -DestinationPath $PiStandaloneDir -Force
            }
            if (-not (Test-Path "$PiStandaloneDir\pi.exe")) {
                Write-Fail "unexpected Pi archive layout: $PiStandaloneDir\pi.exe is missing"
            }
        }
        default {
            Write-Fail "internal error: the Pi install method was not resolved in step 1"
        }
    }

    if (Get-Command pi -ErrorAction SilentlyContinue) {
        & pi --version *> $null
        if ($LASTEXITCODE -ne 0) { Write-Fail "installed pi at $((Get-Command pi).Source) does not run" }
        Write-Info "Pi installed: $((& pi --version) -join ' ')"
    } elseif (Test-StandalonePi) {
        Write-Warn "Pi installed at $PiStandaloneDir, which is not on PATH; add it to PATH"
    } else {
        Write-Fail "Pi installation did not produce a runnable 'pi' binary"
    }
}

# ── Step 3: install the alfred binary ─────────────────────────────────

function Get-LatestAlfredVersion {
    try {
        $headers = Get-GitHubHeaders
        $release = Invoke-RestMethod -Uri "$GitHubApi/releases/latest" -Headers $headers
        return ($release.tag_name -replace '^v', '')
    } catch {
        Write-Fail "could not resolve the latest Alfred release: $_"
    }
}

function Get-AlfredRelease {
    param([string]$ResolvedVersion)

    $archiveName = "alfred-v$ResolvedVersion-$AlfredPlatform.zip"
    $archiveUrl = "https://github.com/$GitHubRepo/releases/download/v$ResolvedVersion/$archiveName"

    $script:AlfredTmp = Join-Path $env:TEMP "alfred-install-$(Get-Random)"
    New-Item -ItemType Directory -Path $script:AlfredTmp -Force | Out-Null
    $archivePath = Join-Path $script:AlfredTmp $archiveName

    Write-Info "Downloading $archiveName..."
    $downloaded = $false
    try {
        Invoke-WebRequest -Uri $archiveUrl -OutFile $archivePath -UseBasicParsing -ErrorAction Stop
        $downloaded = $true
    } catch {
        Write-Warn "Direct download failed; trying the GitHub API..."
    }

    if (-not $downloaded) {
        try {
            $headers = Get-GitHubHeaders
            $release = Invoke-RestMethod -Uri "$GitHubApi/releases/tags/v$ResolvedVersion" -Headers $headers
            $asset = $release.assets | Where-Object { $_.name -eq $archiveName }
            if (-not $asset) { Write-Fail "asset $archiveName not found in release v$ResolvedVersion" }
            $downloadHeaders = @{ "Accept" = "application/octet-stream" }
            if ($Token) { $downloadHeaders["Authorization"] = "token $Token" }
            elseif ($env:GH_PAT) { $downloadHeaders["Authorization"] = "token $env:GH_PAT" }
            Invoke-WebRequest -Uri $asset.url -OutFile $archivePath -Headers $downloadHeaders -UseBasicParsing
        } catch {
            Write-Fail "failed to download ${archiveName}: $_"
        }
    }

    try {
        Invoke-WebRequest -Uri "$archiveUrl.sha256" -OutFile "$archivePath.sha256" -UseBasicParsing -ErrorAction Stop
        $expected = ((Get-Content "$archivePath.sha256" -Raw).Split(' ')[0]).Trim().ToLower()
        $actual = (Get-FileHash -Path $archivePath -Algorithm SHA256).Hash.ToLower()
        if ($expected -ne $actual) { Write-Fail "checksum verification failed for $archiveName" }
    } catch {
        Write-Warn "No checksum published for $archiveName; skipping verification"
    }

    Expand-Archive -Path $archivePath -DestinationPath $script:AlfredTmp -Force
    $script:AlfredExtractedDir = Join-Path $script:AlfredTmp "alfred-v$ResolvedVersion-$AlfredPlatform"
    if (-not (Test-Path $script:AlfredExtractedDir)) {
        Write-Fail "unexpected archive layout in $archiveName"
    }
}

function Test-AlfredInstalled {
    param([string]$Target)
    if (-not (Test-Path (Join-Path $InstallDir "alfred.exe"))) { return $false }
    if (-not (Test-Path $AlfredVersionFile)) { return $false }
    if ((Get-Content $AlfredVersionFile -Raw).Trim() -ne $Target) { return $false }
    return (Test-Path (Join-Path $ConfigDir "config.toml")) -and
        (Test-Path (Join-Path $ConfigDir "prompts\system.md")) -and
        (Test-Path (Join-Path $ConfigDir "prompts\user.md"))
}

function Install-AlfredBinary {
    $target = $Version
    if ($target -eq "latest") { $target = Get-LatestAlfredVersion }

    $installed = Join-Path $InstallDir "alfred.exe"
    if (Test-AlfredInstalled -Target $target) {
        Write-Info "Alfred $target already installed, skipping"
        return
    }

    Get-AlfredRelease -ResolvedVersion $target
    New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
    Copy-Item -Path (Join-Path $script:AlfredExtractedDir "alfred.exe") -Destination $installed -Force

    & $installed --version *> $null
    if ($LASTEXITCODE -ne 0) { Write-Fail "the installed alfred binary does not run ($installed)" }

    New-Item -ItemType Directory -Path $AlfredHome -Force | Out-Null
    Set-Content -Path $AlfredVersionFile -Value $target
    Write-Info "Alfred $target installed: $installed"
}

# ── Step 4: seed the config directory ─────────────────────────────────

function Copy-SeedFile {
    param([string]$From, [string]$To)
    if (Test-Path $To) {
        Write-Info "keeping existing $To"
        return $true
    }
    if (-not (Test-Path $From)) { return $false }
    Copy-Item -Path $From -Destination $To
    Write-Info "created $To"
    return $true
}

function Install-AlfredConfig {
    $configFile = Join-Path $ConfigDir "config.toml"
    $systemFile = Join-Path $ConfigDir "prompts\system.md"
    $userFile = Join-Path $ConfigDir "prompts\user.md"
    New-Item -ItemType Directory -Path "$ConfigDir\prompts" -Force | Out-Null

    if ((Test-Path $configFile) -and (Test-Path $systemFile) -and (Test-Path $userFile)) {
        Write-Info "Configuration already seeded, skipping"
        return
    }
    if (-not $script:AlfredExtractedDir) {
        Write-Fail "cannot seed configuration: the Alfred release was not downloaded"
    }

    Write-Info "Seeding configuration in $ConfigDir..."
    if (-not (Copy-SeedFile -From (Join-Path $script:AlfredExtractedDir "config.toml.example") -To $configFile)) {
        Write-Fail "the Alfred release is missing config.toml.example"
    }
    if (-not (Copy-SeedFile -From (Join-Path $script:AlfredExtractedDir "prompts\system.md.example") -To $systemFile)) {
        Write-Fail "the Alfred release is missing prompts\system.md.example"
    }
    if (-not (Copy-SeedFile -From (Join-Path $script:AlfredExtractedDir "prompts\user.md.example") -To $userFile)) {
        Write-Fail "the Alfred release is missing prompts\user.md.example"
    }
}

# ── Step 5: next steps ────────────────────────────────────────────────

function Add-ToUserPath {
    param([string]$Dir)
    $current = [Environment]::GetEnvironmentVariable("Path", "User")
    if ($current -and ($current.Split(';') -contains $Dir)) { return }
    $newPath = if ($current) { "$Dir;$current" } else { $Dir }
    [Environment]::SetEnvironmentVariable("Path", $newPath, "User")
    Write-Info "Added $Dir to the user PATH (restart your terminal)"
}

function Show-NextSteps {
    Write-Host ""
    Write-Info "Alfred and Pi are installed."
    Write-Host ""
    Write-Host "Next steps:"
    Write-Host "  1. Restart your terminal so the updated PATH is picked up."
    Write-Host "  2. Set the Pi provider key named by [pi].api_key_env (default PI_API_KEY):"
    Write-Host "       `$env:PI_API_KEY = '...'"
    Write-Host "  3. Review $ConfigDir\config.toml ([pi].provider, [pi].model, [pi].binary)."
    Write-Host "  4. Start the server with 'alfred'."
    Write-Host ""

    Add-ToUserPath -Dir $InstallDir
    if ($script:PiMethod -eq "binary") { Add-ToUserPath -Dir $PiStandaloneDir }
}

function Install-Alfred {
    Write-Info "Installing Alfred (with Pi)..."
    Install-PiPrerequisite
    Install-Pi
    Install-AlfredBinary
    Install-AlfredConfig
    if ($script:AlfredTmp) { Remove-Item -Path $script:AlfredTmp -Recurse -Force -ErrorAction SilentlyContinue }
    if ($script:PiTmp) { Remove-Item -Path $script:PiTmp -Recurse -Force -ErrorAction SilentlyContinue }
    Show-NextSteps
}

Install-Alfred
