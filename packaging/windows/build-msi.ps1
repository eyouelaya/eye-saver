<#
.SYNOPSIS
  Builds an EyeSaver MSI.

.DESCRIPTION
  One architecture per installer, because an MSI cannot hold two. Run it twice to produce
  both:

    ./packaging/windows/build-msi.ps1                      # x64
    ./packaging/windows/build-msi.ps1 -Arch arm64          # ARM64

  Requires the WiX v4 command line, which is a .NET tool. Pin the version: v7 needs its
  "Open Source Maintenance Fee" EULA accepted before it will run at all, and targets a
  different Package.wxs schema besides.

    dotnet tool install --global wix --version 4.0.6

  The Util extension Package.wxs needs is fetched by this script, at the same version.

  Set WINDOWS_CERT (and optionally WINDOWS_CERT_PASSWORD) to sign the result; without it
  the MSI is unsigned and SmartScreen will warn on first run.
#>
[CmdletBinding()]
param(
    [ValidateSet('x64', 'arm64')]
    [string]$Arch = 'x64'
)

$ErrorActionPreference = 'Stop'
$root = Resolve-Path (Join-Path $PSScriptRoot '..' '..')
Push-Location $root
try {
    $target = switch ($Arch) {
        'x64'   { 'x86_64-pc-windows-msvc' }
        'arm64' { 'aarch64-pc-windows-msvc' }
    }

    $version = (Select-String -Path 'Cargo.toml' -Pattern '^version = "(.*)"' |
        Select-Object -First 1).Matches[0].Groups[1].Value

    Write-Host "==> generating icons"
    cargo run -p xtask --quiet -- icons | Out-Null

    Write-Host "==> building eye-saver for $target"
    rustup target add $target | Out-Null
    cargo build --release -p eye-saver --target $target

    $exe = Join-Path $root "target/$target/release/eye-saver.exe"
    $icon = Join-Path $root 'packaging/generated/EyeSaver.ico'
    if (-not (Test-Path $exe)) { throw "missing $exe" }

    if ($env:WINDOWS_CERT) {
        Write-Host "==> signing eye-saver.exe"
        $signArgs = @('sign', '/fd', 'SHA256', '/f', $env:WINDOWS_CERT)
        if ($env:WINDOWS_CERT_PASSWORD) { $signArgs += @('/p', $env:WINDOWS_CERT_PASSWORD) }
        & signtool @signArgs $exe
    }

    $dist = Join-Path $root 'dist'
    New-Item -ItemType Directory -Force -Path $dist | Out-Null
    $msi = Join-Path $dist "EyeSaver-$version-$Arch.msi"

    Write-Host "==> building $msi"
    # Keep in step with the wix version above: an extension from another major version
    # targets a different schema.
    & wix extension add --global WixToolset.Util.wixext/4.0.6
    if ($LASTEXITCODE -ne 0) { throw "could not add the WiX Util extension" }
    & wix build (Join-Path $PSScriptRoot 'Package.wxs') `
        -ext WixToolset.Util.wixext `
        -arch $Arch `
        -d "Version=$version" `
        -d "ExePath=$exe" `
        -d "IconPath=$icon" `
        -out $msi
    if ($LASTEXITCODE -ne 0) { throw "wix build failed" }

    if ($env:WINDOWS_CERT) {
        Write-Host "==> signing the installer"
        $signArgs = @('sign', '/fd', 'SHA256', '/f', $env:WINDOWS_CERT)
        if ($env:WINDOWS_CERT_PASSWORD) { $signArgs += @('/p', $env:WINDOWS_CERT_PASSWORD) }
        & signtool @signArgs $msi
    } else {
        Write-Host "==> not signed: SmartScreen will warn on first run"
    }

    Write-Host "==> built $msi"
}
finally {
    Pop-Location
}
