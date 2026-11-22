# RustCode Windows Packaging Script - Creates a distribution zip bundle
param(
    [string]$Version = "0.1.0",
    [switch]$Build,
    [switch]$Desktop,
    [ValidateRange(1, 64)]
    [int]$Jobs = 2
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

$CargoBuildArgs = @("build", "--release", "-j", "$Jobs", "--bin", "rustcode")
if ($Desktop) {
    $CargoBuildArgs += @("--features", "desktop")
}

if ($Build) {
    Write-Host "==> Building release binary (cargo $($CargoBuildArgs -join ' '))..."
    Push-Location $RepoRoot
    try {
        & cargo @CargoBuildArgs
        if ($LASTEXITCODE -ne 0) {
            throw "cargo $($CargoBuildArgs -join ' ') failed with exit code $LASTEXITCODE"
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
$WebView2Arch = switch ($Arch) {
    "x86_64" { "x64" }
    "aarch64" { "arm64" }
    "x86" { "x86" }
    default { "x64" }
}
if (Test-Path $ReleaseBin) {
    Write-Host "==> Bundling release binary..."
    Copy-Item -Path $ReleaseBin -Destination (Join-Path $BundleDir "bin\rustcode.exe")
    $ReleaseDir = Split-Path -Parent $ReleaseBin
    $WebView2Dll = Join-Path $ReleaseDir "WebView2Loader.dll"
    if ((-not (Test-Path $WebView2Dll)) -and $Desktop) {
        $BuildDir = Join-Path $ReleaseDir "build"
        if (Test-Path $BuildDir) {
            $FoundDll = Get-ChildItem -Path $BuildDir -Filter "WebView2Loader.dll" -Recurse -ErrorAction SilentlyContinue |
                Where-Object { $_.FullName -match "[\\/]webview2-com-sys[^\\/]*[\\/].*[\\/]out[\\/]$WebView2Arch[\\/]WebView2Loader\.dll$" } |
                Sort-Object LastWriteTimeUtc -Descending |
                Select-Object -First 1
            if ($FoundDll) {
                $WebView2Dll = $FoundDll.FullName
            }
        }
    }
    if ($WebView2Dll -and (Test-Path $WebView2Dll)) {
        Write-Host "==> Bundling WebView2Loader.dll..."
        Copy-Item -Path $WebView2Dll -Destination (Join-Path $BundleDir "bin\WebView2Loader.dll")
    } elseif ($Desktop -and $Build) {
        throw "WebView2Loader.dll was not found alongside '$ReleaseBin' or under '$ReleaseDir\build' for -Desktop build."
    }
} elseif ($Build) {
    throw "Release binary was not produced at '$ReleaseBin' after build."
} else {
    Write-Host "==> Release binary not found locally. To compile: cargo $($CargoBuildArgs -join ' ')"
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
