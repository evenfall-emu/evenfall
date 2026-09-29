#!/usr/bin/env bash
# Measure a committed core revision in an isolated checkout: fresh target per
# profile, then rebuild after changing one inlined Bus::now body line.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
mkdir "$scratch/source"
git -C "$root" archive HEAD | tar -x -C "$scratch/source"
cd "$scratch/source"

unset RUSTC_WRAPPER CARGO_BUILD_RUSTC_WRAPPER
cargo="$HOME/.cargo/bin/cargo" # bypass host output caches/wrappers
"$cargo" fetch --locked -q
file=systems/gba/src/bus/mod.rs

measure() {
    local label=$1
    shift
    local start end
    start=$(date +%s%N)
    if ! "$@" >"$scratch/build.log" 2>&1; then
        cat "$scratch/build.log" >&2
        echo "FAILED: $label" >&2
        return 1
    fi
    end=$(date +%s%N)
    awk -v label="$label" -v ns="$((end - start))" \
        'BEGIN { printf "%-42s %6.2f s\n", label, ns / 1000000000 }'
}

edit() {
    # The body of this inlined method is shared by many codegen units.
    if ! grep -Fq 'pub fn now(&self) -> Cycle {' "$file"; then
        echo "Bus::now body changed; update the build-time edit" >&2
        return 1
    fi
    sed -i "0,/pub fn now(&self) -> Cycle {/s//pub fn now(\&self) -> Cycle { core::hint::black_box(${1}u32);/" "$file"
}

echo "Evenfall revision: $(git -C "$root" rev-parse HEAD)"
for profile in dev release; do
    export CARGO_TARGET_DIR="$scratch/target-$profile"
    measure "$profile cold (core and dependencies)" "$cargo" build --locked --profile "$profile" -p evenfall-gba
    edit 1
    measure "$profile one-line edit" "$cargo" build --locked --profile "$profile" -p evenfall-gba
    # Remove only our scratch edit; never reset a source file in the caller's tree.
    git -C "$root" show HEAD:"$file" >"$file"
done

# Measure the shipped WASM consumer, not merely the core rlib.
export CARGO_TARGET_DIR="$scratch/target-wasm-release"
export CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS="-C link-arg=--initial-memory=16777216 -C link-arg=--max-memory=536870912 -C link-arg=-zstack-size=8388608 -C link-arg=--export-memory"
measure "wasm-release cold (GBA web module)" "$cargo" build --locked --target wasm32-unknown-unknown --profile wasm-release -p evenfall-gba-web
edit 1
measure "wasm-release one-line edit" "$cargo" build --locked --target wasm32-unknown-unknown --profile wasm-release -p evenfall-gba-web
