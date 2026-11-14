#!/bin/sh
# 构建 AstroBox 插件 wasm 并打包 dist/chaos-connect.abp。
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

cp target/wasm32-wasip2/release/astrobox_plugin_chaos_connect.wasm "$HERE/dist/"

cd "$HERE/dist"
rm -f chaos-connect.abp
zip -q -9 chaos-connect.abp manifest.json icon.png astrobox_plugin_chaos_connect.wasm
echo "wrote $HERE/dist/chaos-connect.abp"
