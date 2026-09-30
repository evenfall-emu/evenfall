#!/usr/bin/env bash
# Fetches the pinned references recorded in refs/README.md.
# usage: tools/setup-refs.sh [name...]   (default: all but the optional ones)
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

install_ref() {
    local name=$1 url=$2 commit=$3 destination="$repo_root/refs/$1"
    if ((${#selected[@]})) && [[ " ${selected[*]} " != *" $name "* ]]; then
        return
    fi
    # An optional ref is fetched only when named.
    if [[ ${4:-} == optional ]] && [[ " ${selected[*]} " != *" $name "* ]]; then
        return
    fi

    if [[ -d "$destination/.git" ]]; then
        if [[ "$(git -C "$destination" rev-parse HEAD)" == "$commit" ]]; then
            echo "[setup] refs/$name is already at $commit"
            return
        fi
        if [[ -n "$(git -C "$destination" status --porcelain)" ]]; then
            echo "[setup] refs/$name has local changes; refusing to replace them" >&2
            return 1
        fi
    elif [[ -e "$destination" ]]; then
        echo "[setup] refs/$name exists but is not a Git checkout" >&2
        return 1
    else
        mkdir -p "$destination"
        git -C "$destination" init --quiet
        git -C "$destination" remote add origin "$url"
    fi

    echo "[setup] Fetching refs/$name at $commit"
    git -C "$destination" fetch --quiet --depth 1 origin "$commit"
    git -C "$destination" checkout --quiet --detach FETCH_HEAD
}

selected=("$@")
mkdir -p "$repo_root/refs"
install_ref gba-tests https://github.com/jsmolka/gba-tests a7113b67e63f83a9b321696ddd7042ccfad6c881
install_ref cult-gba-bios https://github.com/Cult-of-GBA/BIOS a30e9a96df083628b650724b7d4d7112b4070b98
install_ref suite https://github.com/mgba-emu/suite e6942030d25ffe3ba76c72b73a86da073ec857cc
install_ref mgba https://github.com/mgba-emu/mgba 26b7884bc25a5933960f3cdcd98bac1ae14d42e2
install_ref nanoboyadvance https://github.com/nba-emu/NanoBoyAdvance 55b5cf0ae3d929582ac5bfd486558173502b8354
install_ref delta https://github.com/rileytestut/Delta c1d3d068e019e6493eed45654569db3cc5beb86a
install_ref gbadeltacore https://github.com/rileytestut/GBADeltaCore 869c34aeca9dd2b3fbd329092bb2743b0ae80c98
install_ref vbam https://github.com/visualboyadvance-m/visualboyadvance-m 453fa0decf179360926fb417725a794aa496d6e3
# Optional, fetched only when named: only the benchmark's zigba adapter uses it.
install_ref zigba https://github.com/evenfall-emu/zigba add59e5957b75f29d084813336ee20e300fc0b6e optional
