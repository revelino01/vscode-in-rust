# RustCode Windows Packaging Script - Creates a distribution zip bundle
param(
    [string]$Version = "0.1.0",
    [switch]$Build
)

$ErrorActionPreference = "Stop"

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$RepoRoot = Split-Path -Parent $ScriptDir
$DistDir = Join-Path $RepoRoot "dist"
$TargetName = "rustcode"
$Arch = switch ($env:PROCESSOR_ARCHITECTURE) {
    "AMD64" { "x86_64" }
    "ARM64" { "aarch64" }
    default { if ($env:PROCESSOR_ARCHITECTURE) { $env:PROCESSOR_ARCHITECTURE.ToLowerInvariant() } else { "x86_64" } }
}
$PackageName = "${TargetName}-${Version}-windows-${Arch}"
$BundleDir = Join-Path $DistDir $PackageName

if ($Build) {
    Write-Host "==> Building release binary..."
    Push-Location $RepoRoot
    try {
        cargo build --release --bin rustcode
        if ($LASTEXITCODE -ne 0) {
            throw "cargo build --release --bin rustcode failed with exit code $LASTEXITCODE"
        }
    } finally {
        Pop-Location
    }
}

Write-Host "==> Packaging RustCode v${Version} for windows-${Arch}..."

if (Test-Path $BundleDir) {
    Remove-Item -Recurse -Force $BundleDir
}
$ZipPath = Join-Path $DistDir "${PackageName}.zip"
if (Test-Path $ZipPath) {
    Remove-Item -Force $ZipPath
}

New-Item -ItemType Directory -Force -Path (Join-Path $BundleDir "bin") | Out-Null
New-Item -ItemType Directory -Force -Path (Join-Path $BundleDir "resources") | Out-Null
New-Item -ItemType Directory -Force -Path (Join-Path $BundleDir "ext-host-js") | Out-Null

$TargetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $RepoRoot "target" }
$ReleaseBin = Join-Path $TargetDir "release\rustcode.exe"
if (-not (Test-Path $ReleaseBin)) {
    $ReleaseBin = Join-Path $RepoRoot "target\release\rustcode.exe"
}
if (Test-Path $ReleaseBin) {
    Write-Host "==> Bundling release binary..."
    Copy-Item -Path $ReleaseBin -Destination (Join-Path $BundleDir "bin\rustcode.exe")
} else {
    Write-Host "==> Release binary not found locally. To compile: cargo build --release --bin rustcode"
    New-Item -ItemType File -Force -Path (Join-Path $BundleDir "bin\.build_required") | Out-Null
}

Write-Host "==> Bundling resources and extension host shims..."
$ResourcesDir = Join-Path $RepoRoot "resources"
if (Test-Path $ResourcesDir) {
    Copy-Item -Path (Join-Path $ResourcesDir "*") -Destination (Join-Path $BundleDir "resources") -Recurse -Force
}

$ExtHostJsDir = Join-Path $RepoRoot "ext-host-js"
if (Test-Path $ExtHostJsDir) {
    Copy-Item -Path (Join-Path $ExtHostJsDir "*") -Destination (Join-Path $BundleDir "ext-host-js") -Recurse -Force
}

Compress-Archive -Path $BundleDir -DestinationPath $ZipPath -Force
Write-Host "==> Distribution archive created at: $ZipPath"
