#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "$root"

# The module owns its memory. No imports, threads, host allocator or startup
# function are needed; each worker instantiates one independent session.
link="-C link-arg=--initial-memory=16777216 -C link-arg=--max-memory=536870912 -C link-arg=-zstack-size=8388608 -C link-arg=--export-memory"

# Two builds of the same source: the default module uses WebAssembly SIMD
# (simd128), and a scalar module serves engines without it. The SDK picks one
# at load time with WebAssembly.validate. Separate target directories keep
# each build incremental despite the different code-generation flags.
build() {
    local flags=$1 target_dir=$2 output=$3
    CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS="$link $flags" \
        cargo build --target wasm32-unknown-unknown --profile wasm-release \
        -p evenfall-gba-web --target-dir "$target_dir"
    cp "$target_dir/wasm32-unknown-unknown/wasm-release/evenfall_gba_web.wasm" "$output"
}

build "-C target-feature=+simd128" target/wasm-simd128 web/gba/evenfall-gba.wasm
build "" target/wasm-scalar web/gba/evenfall-gba-scalar.wasm
