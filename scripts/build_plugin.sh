#!/bin/sh
# Build the AstroBox plugin and package dist/band-files.abp.
set -eu
HERE=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
WORKSPACE=$(CDPATH= cd -- "$HERE/../.." && pwd)
export PATH="$WORKSPACE/tools/dl/bin:$WORKSPACE/.cargo/bin:$PATH"
export RUSTUP_HOME="$WORKSPACE/.rustup"
export CARGO_HOME="$WORKSPACE/.cargo"
export CC="$WORKSPACE/tools/dl/bin/cc"
export CXX="$WORKSPACE/tools/dl/bin/cc"
cd "$HERE/plugin"
cargo build --target wasm32-wasip2 --release
cp target/wasm32-wasip2/release/astrobox_plugin_band_files.wasm "$HERE/dist/"
cd "$HERE/dist"
rm -f band-files.abp
zip -q -9 band-files.abp manifest.json icon.png astrobox_plugin_band_files.wasm
echo "wrote $HERE/dist/band-files.abp"
