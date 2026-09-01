# Alloy installer - downloads latest release and sets up PATH
# Usage: irm https://raw.githubusercontent.com/plinthlol/alloy/HEAD/assets/install.ps1 | iex
param(
    [string]$Version = "latest",
    [string]$Variant = ""
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"
$repo = "plinthlol/alloy"

# Variant selection: -Variant param > ALLOY_VARIANT env > interactive picker > default to tui
if (-not $Variant) { $Variant = $env:ALLOY_VARIANT }

if (-not $Variant) {
    if ([Environment]::UserInteractive -and -not ([Console]::IsInputRedirected)) {
        Write-Host "Select Alloy variant:"
        Write-Host "  1) alloysh (TUI, recommended)"
        Write-Host "  2) alloyctl (CLI)"
        $choice = Read-Host "Enter choice [1-2]"
        switch ($choice) {
            { $_ -in "1", "" } { $Variant = "tui" }
            "2" { $Variant = "cli" }
            default { throw "Invalid choice: '$choice' (expected 1 or 2)" }
        }
    } else {
        Write-Host "No terminal detected, defaulting to alloysh (TUI). Set `$env:ALLOY_VARIANT = 'tui'|'cli'` to choose explicitly."
        $Variant = "tui"
    }
}

switch ($Variant.ToLower()) {
    "tui" { $binary = "alloysh.exe"; $artifact = "alloysh-windows-x86_64.exe" }
    "cli" { $binary = "alloyctl.exe"; $artifact = "alloyctl-windows-x86_64.exe" }
    default { throw "Unknown variant: '$Variant' (use tui or cli)" }
}

# Setup download URLs and paths
$base = if ($Version -eq "latest") {
    "https://github.com/$repo/releases/latest/download"
} else {
    "https://github.com/$repo/releases/download/$Version"
}

$binDir = Join-Path $env:LOCALAPPDATA "alloy\bin"
$null = New-Item -ItemType Directory -Force -Path $binDir
$target = Join-Path $binDir $binary

# Download binary
Write-Host "Downloading $artifact..."
try {
    Invoke-WebRequest -Uri "$base/$artifact" -OutFile $target -UseBasicParsing
} catch {
    throw "error: download failed: $($_.Exception.Message)"
}

# Verify checksum (skippable only if the checksum file can't be fetched)
$expected = $null
try {
    $checksumContent = (Invoke-WebRequest -Uri "$base/$artifact.sha256" -UseBasicParsing).Content
    $expected = ($checksumContent -split '\s+' | Select-Object -First 1).Trim().ToLower()
} catch {
    Write-Host "warning: checksum verification skipped" -ForegroundColor Yellow
}

if (-not [string]::IsNullOrWhiteSpace($expected)) {
    $actual = (Get-FileHash -Algorithm SHA256 -Path $target).Hash.ToLower()
    if ($expected -ne $actual) {
        throw "error: checksum mismatch"
    }
}

# Update PATH if needed
$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
if ($userPath -notlike "*$binDir*") {
    $newPath = "$binDir;$userPath"
    [Environment]::SetEnvironmentVariable("Path", $newPath, "User")
    $env:Path = $newPath
    Write-Host "Updated PATH - restart terminal or start a new one"
}

Write-Host "✓ Installed $binary to $target"
