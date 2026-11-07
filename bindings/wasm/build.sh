#!/usr/bin/env sh
set -eu
cd "$(dirname "$0")/../.."
cargo build --release --target wasm32-unknown-unknown --manifest-path bindings/wasm/Cargo.toml
wasm-bindgen target/wasm32-unknown-unknown/release/aerosieve_wasm.wasm \
  --out-dir site/pkg --target web --no-typescript
