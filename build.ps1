# build.ps1 — Windows 一键发布脚本
# 流程：cargo build --release → UPX 压缩到 dist（平台+版本命名）→ 清理临时编译产物
#
# 用法：powershell -File build.ps1
#
# 注：本机 Rust 工具链位于 D:\rust（RUSTUP_HOME / CARGO_HOME）；换机器时调整下方三行。

$ErrorActionPreference = "Stop"
Set-Location (Split-Path -Parent $MyInvocation.MyCommand.Path)

# 本机 Rust 工具链路径（按需修改）
$env:RUSTUP_HOME = "D:\rust\.rustup"
$env:CARGO_HOME = "D:\rust\.cargo"
$env:Path = "D:\rust\.cargo\bin;" + $env:Path

# 从 Cargo.toml 读取版本号（[package] 段的 version）
$ver = ((Get-Content Cargo.toml | Select-String '^version = ' | Select-Object -First 1).ToString() -split '"')[1]
$out = "dist\sysenv-windows-x86_64_v${ver}.exe"

Write-Host "== cargo build --release (v$ver) =="
cargo build --release

Write-Host "== UPX compress -> $out =="
New-Item -ItemType Directory -Force dist | Out-Null
upx --best -o $out "target\release\sysenv.exe"

Write-Host "== smoke test =="
& $out --version

Write-Host "== cleanup temp build artifacts =="
cargo clean

Write-Host "OK: $out ($((Get-Item $out).Length) bytes)"
