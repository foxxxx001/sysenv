#!/usr/bin/env bash
# build.sh — Linux (Ubuntu) 一键发布脚本
# 流程：cargo build --release → UPX 压缩到 dist（平台+版本命名）→ 清理临时编译产物
#
# 用法：./build.sh     （UPX 可用环境变量指定：UPX=/path/to/upx ./build.sh）
set -e
cd "$(dirname "$0")"

export PATH="$HOME/.cargo/bin:$PATH"

# WSL 下若 ~/.cargo/config.toml 指向不可达的 git:// 镜像，先临时移走（构建后恢复）
if [ -f "$HOME/.cargo/config.toml" ]; then mv "$HOME/.cargo/config.toml" "$HOME/.cargo/config.toml.orig"; fi
if [ -f "$HOME/.cargo/config" ]; then mv "$HOME/.cargo/config" "$HOME/.cargo/config.orig"; fi
export CARGO_REGISTRIES_CRATES_IO_PROTOCOL=sparse

VER=$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml)
OUT="dist/sysenv-linux-x86_64_v${VER}"

# Locate UPX; download a static build when the system has none
if [ -n "${UPX:-}" ]; then
  UPX_BIN="$UPX"
elif command -v upx >/dev/null 2>&1; then
  UPX_BIN=$(command -v upx)
else
  UPX_BIN=/tmp/upx-5.2.1-amd64_linux/upx
  if [ ! -x "$UPX_BIN" ]; then
    cd /tmp || exit 1
    curl -sL -o upx.tar.xz https://github.com/upx/upx/releases/download/v5.2.1/upx-5.2.1-amd64_linux.tar.xz
    tar -xf upx.tar.xz
    cd - >/dev/null || exit 1
  fi
fi

echo "== cargo build --release (v$VER) =="
cargo build --release

echo "== UPX compress -> $OUT =="
mkdir -p dist
"$UPX_BIN" --best --force -o "$OUT" target/release/sysenv

echo "== smoke test =="
"./$OUT" --version

echo "== cleanup temp build artifacts =="
cargo clean

if [ -f "$HOME/.cargo/config.toml.orig" ]; then mv "$HOME/.cargo/config.toml.orig" "$HOME/.cargo/config.toml"; fi
if [ -f "$HOME/.cargo/config.orig" ]; then mv "$HOME/.cargo/config.orig" "$HOME/.cargo/config"; fi

echo "OK: $OUT ($(stat -c%s "$OUT") bytes)"
