#!/bin/sh
# 构建 AstroBox 插件 wasm 并打包 dist/band-files.abp。
# 只依赖 Rust 工具链：rustup target add wasm32-wasip2
set -eu
HERE=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)

if ! command -v cargo >/dev/null 2>&1; then
    echo "找不到 cargo，请先安装 Rust 工具链： https://rustup.rs" >&2
    exit 1
fi
if command -v rustup >/dev/null 2>&1 && ! rustup target list --installed 2>/dev/null | grep -qx wasm32-wasip2; then
    echo "缺少 wasm32-wasip2 target，先执行： rustup target add wasm32-wasip2" >&2
    exit 1
fi

cd "$HERE/plugin"
cargo build --target wasm32-wasip2 --release

cp target/wasm32-wasip2/release/astrobox_plugin_band_files.wasm "$HERE/dist/"

cd "$HERE/dist"
rm -f band-files.abp
zip -q -9 band-files.abp manifest.json icon.png astrobox_plugin_band_files.wasm
echo "wrote $HERE/dist/band-files.abp"
