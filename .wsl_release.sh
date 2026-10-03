#!/usr/bin/env bash
# Build Linux release v0.2.1 in WSL, compress with UPX, name per convention.
set -u
export PATH="$HOME/.cargo/bin:$PATH"
cargo --version 2>&1 || true

if [ -f "$HOME/.cargo/config.toml" ]; then mv "$HOME/.cargo/config.toml" "$HOME/.cargo/config.toml.orig"; fi
if [ -f "$HOME/.cargo/config" ]; then mv "$HOME/.cargo/config" "$HOME/.cargo/config.orig"; fi
export CARGO_REGISTRIES_CRATES_IO_PROTOCOL=sparse

# Get UPX (static linux-amd64) if not present
UPX=/tmp/upx-5.2.1-amd64_linux/upx
if [ ! -x "$UPX" ]; then
  cd /tmp || exit 1
  curl -sL -o upx.tar.xz https://github.com/upx/upx/releases/download/v5.2.1/upx-5.2.1-amd64_linux.tar.xz
  tar -xf upx.tar.xz
fi
"$UPX" --version | head -n 1

cd /mnt/d/work/ai/systool/sysenv || exit 1

echo "== cargo test =="
cargo test > /tmp/ltest.log 2>&1; echo "test rc=$?"; grep -E "test result|FAILED|error" /tmp/ltest.log | head -n 5

echo "== cargo build --release =="
cargo build --release > /tmp/lrel.log 2>&1; echo "rel rc=$?"; grep -E "^error|warning:|Finished" /tmp/lrel.log | head -n 5

echo "== UPX compress =="
mkdir -p dist
"$UPX" --best -o dist/sysenv-linux-x86_64_v0.2.1 target/release/sysenv
ls -l dist/
echo "== smoke =="
./dist/sysenv-linux-x86_64_v0.2.1 --version
./dist/sysenv-linux-x86_64_v0.2.1 ai model gpt-4.1 -o csv | tail -n 1 | cut -c1-80
./dist/sysenv-linux-x86_64_v0.2.1 short --dir /tmp/shx --temporary --force > /dev/null 2>&1 && echo "short ok"

if [ -f "$HOME/.cargo/config.toml.orig" ]; then mv "$HOME/.cargo/config.toml.orig" "$HOME/.cargo/config.toml"; fi
if [ -f "$HOME/.cargo/config.orig" ]; then mv "$HOME/.cargo/config.orig" "$HOME/.cargo/config"; fi
echo "ALL_DONE"
