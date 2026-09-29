#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "$root"

# The module owns its memory. No imports, threads, host allocator or startup
# function are needed; each worker instantiates one independent session.
export CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS="-C link-arg=--initial-memory=16777216 -C link-arg=--max-memory=536870912 -C link-arg=-zstack-size=8388608 -C link-arg=--export-memory"
cargo build --target wasm32-unknown-unknown --profile wasm-release -p evenfall-gba-web
cp target/wasm32-unknown-unknown/wasm-release/evenfall_gba_web.wasm web/gba/evenfall-gba.wasm
