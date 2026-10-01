#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "$root"

# wasm-bindgen's command-line tool must be exactly the version of the
# wasm-bindgen crate in Cargo.lock, so read it from there. A matching tool on
# PATH is used as is. Otherwise the pinned version is built once into target/.
version="$(awk '/^name = "wasm-bindgen"$/ { getline; gsub(/version = |"/, ""); print; exit }' Cargo.lock)"
if [[ -z "$version" ]]; then
    echo "Cargo.lock names no wasm-bindgen version." >&2
    exit 1
fi
bindgen=wasm-bindgen
if [[ "$(wasm-bindgen --version 2>/dev/null)" != "wasm-bindgen $version" ]]; then
    tools="$root/target/tools/wasm-bindgen-$version"
    bindgen="$tools/bin/wasm-bindgen"
    if [[ ! -x "$bindgen" ]]; then
        cargo install wasm-bindgen-cli --version "=$version" --locked --root "$tools"
    fi
fi

# The same profile as the emulator core's module. A separate target directory
# keeps the core's builds incremental.
cargo build --target wasm32-unknown-unknown --profile wasm-release \
    -p evenfall-present-web --target-dir target/wasm-present
rm -rf web/present/pkg
"$bindgen" --target web --no-typescript --out-dir web/present/pkg \
    target/wasm-present/wasm32-unknown-unknown/wasm-release/evenfall_present_web.wasm
