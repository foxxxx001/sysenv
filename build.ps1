# build.ps1 - Windows one-shot release script
# Steps: cargo build --release -> UPX compress to dist (platform+version name) -> cleanup temp build artifacts
#
# Usage: powershell -File build.ps1
#
# Note: this machine keeps its Rust toolchain in D:\rust (RUSTUP_HOME / CARGO_HOME);
# adjust the three lines below when building on another machine.

$ErrorActionPreference = "Stop"
Set-Location (Split-Path -Parent $MyInvocation.MyCommand.Path)

# Machine-specific Rust toolchain paths (edit if needed)
$env:RUSTUP_HOME = "D:\rust\.rustup"
$env:CARGO_HOME = "D:\rust\.cargo"
$env:Path = "D:\rust\.cargo\bin;" + $env:Path

# Read version from Cargo.toml ([package] section)
$ver = ((Get-Content Cargo.toml | Select-String '^version = ' | Select-Object -First 1).ToString() -split '"')[1]
$out = "dist\sysenv-windows-x86_64_v${ver}.exe"

Write-Host "== cargo build --release (v$ver) =="
cargo build --release

Write-Host "== UPX compress -> $out =="
New-Item -ItemType Directory -Force dist | Out-Null
upx --best --force -o $out "target\release\sysenv.exe"

Write-Host "== smoke test =="
& $out --version

Write-Host "== cleanup temp build artifacts =="
cargo clean

Write-Host "OK: $out ($((Get-Item $out).Length) bytes)"
