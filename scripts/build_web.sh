#!/usr/bin/env bash
# Build the TPT Nova web/PWA bundle into crates/tpt-nova-app/web.
#
# Needs: the wasm32-unknown-unknown target (rustup target add wasm32-unknown-unknown)
# and the wasm-bindgen CLI at the same version as the wasm-bindgen crate in Cargo.lock.
#
# Then serve the folder over HTTP and open it in a WebGPU browser:
#   python -m http.server 8080 --directory crates/tpt-nova-app/web
#   http://localhost:8080
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$ROOT/crates/tpt-nova-app/web"
WASM="$ROOT/target/wasm32-unknown-unknown/release/tpt-nova-app.wasm"

cargo build -p tpt-nova-app --target wasm32-unknown-unknown --release --manifest-path "$ROOT/Cargo.toml"
wasm-bindgen --target web --out-dir "$OUT" "$WASM"
echo "web bundle written to $OUT"
