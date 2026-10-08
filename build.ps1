# build.ps1 - Windows one-shot release script
# Steps: cargo build --release (sccache-cached) -> UPX compress to dist (platform+version name)
#
# Usage: powershell -File build.ps1
#
# Note: this machine keeps its Rust toolchain in D:\rust (RUSTUP_HOME / CARGO_HOME);
# adjust the lines below when building on another machine.
# Compile intermediates are cached by sccache under SCCACHE_DIR, so target/ is
# intentionally kept between builds (rebuilds hit the sccache cache).

$ErrorActionPreference = "Stop"
Set-Location (Split-Path -Parent $MyInvocation.MyCommand.Path)

# Machine-specific Rust toolchain paths (edit if needed)
$env:RUSTUP_HOME = "D:\rust\.rustup"
$env:CARGO_HOME = "D:\rust\.cargo"
$env:Path = "D:\rust\.cargo\bin;" + $env:Path

# sccache: cache compile intermediates under a fixed directory (edit if needed)
$env:SCCACHE_DIR = "D:\cdrom\sccache-cache"
$env:RUSTC_WRAPPER = "sccache"

# Read version from Cargo.toml ([package] section)
$ver = ((Get-Content Cargo.toml | Select-String '^version = ' | Select-Object -First 1).ToString() -split '"')[1]
$out = "dist\sys-windows-x86_64_v${ver}.exe"

Write-Host "== cargo build --release (v$ver, sccache) =="
cargo build --release

Write-Host "== UPX compress -> $out =="
New-Item -ItemType Directory -Force dist | Out-Null
upx --best --force -o $out "target\release\sys.exe"

Write-Host "== smoke test =="
& $out --version

Write-Host "OK: $out ($((Get-Item $out).Length) bytes)"
