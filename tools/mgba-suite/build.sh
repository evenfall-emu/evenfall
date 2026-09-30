#!/usr/bin/env bash
# Builds the pinned mGBA suite ROM with a pinned devkitARM container image.
# usage: tools/mgba-suite/build.sh [SUITE_CHECKOUT]   (default: refs/suite)
#
# The suite source is exported from the pinned commit with `git archive`, so
# the checkout is only read. Outputs (gitignored): tools/mgba-suite/build/
# suite.gba and suite.elf. CONTAINER_ENGINE selects podman or docker.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
suite="${1:-$repo_root/refs/suite}"
commit=e6942030d25ffe3ba76c72b73a86da073ec857cc
# devkitpro/devkitarm:20260610
image=docker.io/devkitpro/devkitarm@sha256:116afba8df8453961de2936ffab20dd441edf4d682856c1ec8b0e53d7ed0bbf5
out="$repo_root/tools/mgba-suite/build"

engine="${CONTAINER_ENGINE:-}"
if [[ -z "$engine" ]]; then
    if command -v podman >/dev/null; then engine=podman; else engine=docker; fi
fi
user_args=()
if [[ "$engine" == docker ]]; then
    user_args=(--user "$(id -u):$(id -g)")
fi

rm -rf "$out/src"
mkdir -p "$out/src"
git -C "$suite" archive "$commit" | tar -x -C "$out/src"
# SOURCE_DATE_EPOCH keeps any embedded timestamps fixed.
"$engine" run --rm "${user_args[@]}" -e SOURCE_DATE_EPOCH=0 \
    -v "$out/src:/suite:Z" -w /suite "$image" make
cp "$out/src/suite.gba" "$out/src/suite.elf" "$out/"
sha256sum "$out/suite.gba"
