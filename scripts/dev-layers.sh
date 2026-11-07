#!/bin/bash
# Builds the layer for 64-bit and 32-bit games and writes manifests for both into target/layers, pointing at the builds:
#   scripts/dev-layers.sh
#   VK_ADD_IMPLICIT_LAYER_PATH=$PWD/target/layers ENABLE_KAIROS=1 game
# (in Steam's launch options: VK_ADD_IMPLICIT_LAYER_PATH=/full/path/target/layers ENABLE_KAIROS=1 %command%)
set -eu
cd "$(dirname "$0")/.."
cargo build --release
cargo build --release --target i686-unknown-linux-gnu
mkdir -p target/layers
sed "s#/usr/lib/libkairos.so#$PWD/target/release/libkairos.so#" data/kairos.x86_64.json > target/layers/kairos.x86_64.json
sed "s#/usr/lib32/libkairos.so#$PWD/target/i686-unknown-linux-gnu/release/libkairos.so#" data/kairos.x86.json > target/layers/kairos.x86.json
echo "layers in $PWD/target/layers"
