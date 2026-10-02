# Build Flasher for Windows and its installer:
#   target\dist\Flasher-<version>-windows-x64-setup.exe   (Inno Setup)
#   target\dist\Flasher-<version>-windows-x64-portable.zip
#
# Needs Rust and Inno Setup 6 (https://jrsoftware.org/isinfo.php, or
# `winget install JRSoftware.InnoSetup`). Run from any directory:
#   powershell -ExecutionPolicy Bypass -File packaging\windows\build.ps1
# Extra cargo flags (CI uses --locked) go in $env:CARGO_FLAGS.

param(
    # ISCC.exe, if it is not in one of the usual places.
    [string]$Iscc
)

$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..\..')

$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.*)"$' |
    Select-Object -First 1).Matches[0].Groups[1].Value

# The C runtime goes inside flasher.exe, so it runs on PCs without the
# Visual C++ Redistributable (otherwise: "VCRUNTIME140.dll was not found").
$env:RUSTFLAGS = "$env:RUSTFLAGS -C target-feature=+crt-static".Trim()

$cargoFlags = @()
if ($env:CARGO_FLAGS) { $cargoFlags = $env:CARGO_FLAGS -split '\s+' }
cargo build --release @cargoFlags -p flasher --bin flasher
if ($LASTEXITCODE) { exit $LASTEXITCODE }

$dist = 'target\dist'
New-Item -ItemType Directory -Force $dist | Out-Null

# The portable zip: the exe and the font's licence.
$zip = Join-Path $dist "Flasher-$version-windows-x64-portable.zip"
Compress-Archive -Force -Path target\release\flasher.exe, assets\Inter-OFL.txt -DestinationPath $zip

# The installer.
if (-not $Iscc) {
    $candidates = @(
        (Get-Command iscc.exe -ErrorAction SilentlyContinue).Source,
        "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
        "$env:ProgramFiles\Inno Setup 6\ISCC.exe",
        "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe"
    )
    $Iscc = $candidates | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
}
if (-not $Iscc) {
    throw 'Inno Setup 6 not found: install it (winget install JRSoftware.InnoSetup) or pass -Iscc <path to ISCC.exe>'
}
& $Iscc /Qp "/DAppVersion=$version" "/DSourceExe=..\..\target\release\flasher.exe" packaging\windows\flasher.iss
if ($LASTEXITCODE) { exit $LASTEXITCODE }

Get-ChildItem $dist "Flasher-$version-windows-*" | ForEach-Object { $_.FullName }
