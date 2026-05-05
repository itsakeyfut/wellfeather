#Requires -Version 5.1
<#
.SYNOPSIS
    Build a Windows MSIX package for wellfeather.

.DESCRIPTION
    Standalone script that builds the release binary, assembles the MSIX
    package layout, and calls MakeAppx.exe. Optionally signs the package
    with SignTool.exe.

.PARAMETER Sign
    Enable code signing.

.PARAMETER CertificatePath
    Path to the .pfx certificate file used for signing.

.PARAMETER CertificatePassword
    Password for the .pfx certificate.

.EXAMPLE
    .\build-windows.ps1
    .\build-windows.ps1 -Sign -CertificatePath cert.pfx -CertificatePassword mypass
#>
param(
    [switch]$Sign,
    [string]$CertificatePath = "",
    [string]$CertificatePassword = ""
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Find-SdkTool {
    param([string]$Name)
    $cmd = Get-Command $Name -ErrorAction SilentlyContinue
    if ($cmd) { return $cmd.Path }
    $sdkBase = "C:\Program Files (x86)\Windows Kits\10\bin"
    if (Test-Path $sdkBase) {
        $versions = Get-ChildItem $sdkBase -Directory | Sort-Object Name -Descending
        foreach ($v in $versions) {
            $candidate = Join-Path $v.FullName "x64\$Name.exe"
            if (Test-Path $candidate) { return $candidate }
        }
    }
    return $null
}

function Get-WorkspaceVersion {
    $content = Get-Content (Join-Path $PSScriptRoot "..\..\Cargo.toml") -Raw
    if ($content -match 'version\s*=\s*"([^"]+)"') { return $Matches[1] }
    throw "Cannot parse version from Cargo.toml"
}

$repoRoot = Resolve-Path (Join-Path $PSScriptRoot "..\..") | Select-Object -ExpandProperty Path
Push-Location $repoRoot

try {
    Write-Host "=== Windows MSIX Package ===" -ForegroundColor Blue

    # Build release binary
    Write-Host "Building release binary..." -ForegroundColor Cyan
    cargo build --release
    if ($LASTEXITCODE -ne 0) { throw "cargo build --release failed" }

    $version = Get-WorkspaceVersion
    Write-Host "Version: $version"

    $tempDir = Join-Path $repoRoot "packaging\temp\windows\package"
    $assetsDir = Join-Path $tempDir "Assets"
    $outputDir = Join-Path $repoRoot "packaging\output"
    New-Item -ItemType Directory -Force -Path $assetsDir | Out-Null
    New-Item -ItemType Directory -Force -Path $outputDir | Out-Null

    # Inject version into manifest
    $manifestTemplate = Get-Content (Join-Path $repoRoot "packaging\windows\AppxManifest.xml") -Raw
    $manifest = $manifestTemplate -replace '\{VERSION\}', $version
    $manifest | Set-Content (Join-Path $tempDir "AppxManifest.xml") -Encoding utf8

    # Copy executable
    $exe = Join-Path $repoRoot "target\release\wellfeather.exe"
    if (-not (Test-Path $exe)) { throw "Release binary not found: $exe" }
    Copy-Item $exe (Join-Path $tempDir "wellfeather.exe") -Force

    # Generate icon assets
    $iconSrc = Join-Path $repoRoot "app\assets\icon.png"
    if (-not (Test-Path $iconSrc)) {
        throw "Icon not found at $iconSrc`nPlace a 1024x1024 PNG at app/assets/icon.png"
    }

    $assets = @(
        @{ Name = "Square44x44Logo.png";   W = 44;  H = 44  }
        @{ Name = "Square150x150Logo.png"; W = 150; H = 150 }
        @{ Name = "Wide310x150Logo.png";   W = 310; H = 150 }
        @{ Name = "StoreLogo.png";         W = 50;  H = 50  }
        @{ Name = "SplashScreen.png";      W = 620; H = 300 }
    )

    $hasMagick = $null -ne (Get-Command magick -ErrorAction SilentlyContinue)
    foreach ($a in $assets) {
        $dest = Join-Path $assetsDir $a.Name
        if ($hasMagick) {
            magick $iconSrc -resize "$($a.W)x$($a.H)!" $dest
            if ($LASTEXITCODE -ne 0) { throw "magick failed for $($a.Name)" }
        } else {
            Write-Warning "ImageMagick not found; copying source PNG for $($a.Name)"
            Copy-Item $iconSrc $dest -Force
        }
    }

    # Run MakeAppx
    $makeAppx = Find-SdkTool "MakeAppx"
    if (-not $makeAppx) {
        throw "MakeAppx.exe not found. Install the Windows SDK or add it to PATH."
    }
    $msixPath = Join-Path $outputDir "wellfeather-$version-x86_64-windows.msix"
    Write-Host "Running MakeAppx..." -ForegroundColor Cyan
    & $makeAppx pack /d $tempDir /p $msixPath /nv /o
    if ($LASTEXITCODE -ne 0) { throw "MakeAppx failed" }

    # Optional signing
    if ($Sign) {
        if (-not $CertificatePath) { throw "--Sign requires -CertificatePath" }
        $signTool = Find-SdkTool "signtool"
        if (-not $signTool) { throw "signtool.exe not found." }
        Write-Host "Signing MSIX..." -ForegroundColor Cyan
        $signArgs = @("sign", "/fd", "SHA256", "/f", $CertificatePath)
        if ($CertificatePassword) { $signArgs += @("/p", $CertificatePassword) }
        $signArgs += $msixPath
        & $signTool @signArgs
        if ($LASTEXITCODE -ne 0) { throw "SignTool failed" }
    }

    $sizeMB = [math]::Round((Get-Item $msixPath).Length / 1MB, 1)
    Write-Host "`n✓ Package ready: $msixPath ($sizeMB MB)" -ForegroundColor Green
} finally {
    Pop-Location
}
