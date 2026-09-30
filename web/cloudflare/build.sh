#!/usr/bin/env bash
set -euo pipefail

# Builds the player and the WebAssembly core into .dist/cloudflare, the static
# assets of the `zigba` Worker (web/cloudflare/wrangler.jsonc). It needs only
# Node.js, npm, curl and a C toolchain for build scripts: when rustup is
# missing, it installs a pinned, checksummed rustup, and rust-toolchain.toml
# chooses the compiler. It never migrates or seeds a database.
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"

mkdir -p .dist
temporary="$(mktemp -d "$root/.dist/cloudflare-build.XXXXXX")"
trap 'rm -rf "$temporary"' EXIT

rustup_version=1.29.1
rustup_sha256=dda7234360b7f578ca8b0ddcb80145646fa61a67c1720a5abc7051b35c9fcb71
if ! command -v rustup >/dev/null; then
    if [[ "$(uname -s)-$(uname -m)" != Linux-x86_64 ]]; then
        echo "Install rustup before building on this platform." >&2
        exit 1
    fi
    installer="$temporary/rustup-init"
    curl --fail --location --retry 3 --silent --show-error --proto '=https' --tlsv1.2 \
        "https://static.rust-lang.org/rustup/archive/$rustup_version/x86_64-unknown-linux-gnu/rustup-init" \
        --output "$installer"
    echo "$rustup_sha256  $installer" | sha256sum --check
    chmod +x "$installer"
    # Kept inside the build: nothing is installed into the user's home.
    export RUSTUP_HOME="$temporary/rustup" CARGO_HOME="$temporary/cargo"
    "$installer" -y --quiet --no-modify-path --profile minimal --default-toolchain none
    export PATH="$CARGO_HOME/bin:$PATH"
fi
# Installs the channel, components and targets that rust-toolchain.toml names.
rustup toolchain install
rustc --version

# One root install serves every npm workspace (SDK, Pokémon plugin, player,
# server). Workers Builds' own install is skipped (SKIP_DEPENDENCY_INSTALL).
npm ci --no-audit --no-fund
npm run build:wasm -w @evenfall/gba
npx vp check
npx vp test --project player --project gba-pokemon --project server

# Instance settings for the official play.evenfall-emu.org deployment. These are
# public values, not secrets. A self-hosted deployment exports its own values
# before running this script; an empty value disables the feature. Analytics
# are sent only from pages served on VITE_POSTHOG_HOSTNAME, so previews and
# other hosts of this build stay silent. See docs/player.md#analytics.
export VITE_POSTHOG_KEY="${VITE_POSTHOG_KEY-phc_qRjgGwT22cQ2yYu8Zkgx2hc44NtrgRBeubAyXm9u9vCi}"
export VITE_POSTHOG_HOST="${VITE_POSTHOG_HOST-https://eu.i.posthog.com}"
export VITE_POSTHOG_HOSTNAME="${VITE_POSTHOG_HOSTNAME-play.evenfall-emu.org}"
# The player links this privacy notice; ./privacy.txt publishes PRIVACY.md.
export VITE_PRIVACY_URL="${VITE_PRIVACY_URL-./privacy.txt}"
output="$temporary/output"
EVENFALL_PLAYER_DIR="$output" npm run build -w @evenfall/player

# Publish only this fresh build. Reject game media even if somebody
# accidentally puts it in a source directory.
private_files="$(find "$output" -type f \( \
    -iname '*.gba' -o -iname '*.sav' -o -iname '*.state' -o \
    -iname '*.bin' -o -iname '*.zip' -o -iname '*.tar*' \))"
if [[ -n "$private_files" ]]; then
    echo "Refusing to publish game media/archive files:" >&2
    printf '%s\n' "$private_files" >&2
    exit 1
fi
test -s "$output/index.html"
test -s "$output/service-worker.js"
# The player bundles the SDK: its worker and both WASM cores are hashed assets.
ls "$output/assets" | grep -Eq '^worker-[A-Za-z0-9_-]{8}\.js$'
ls "$output/assets" | grep -Eq '^evenfall-gba-[A-Za-z0-9_-]{8}\.wasm$'
ls "$output/assets" | grep -Eq '^evenfall-gba-scalar-[A-Za-z0-9_-]{8}\.wasm$'
cp web/cloudflare/headers "$output/_headers"
mkdir -p "$output/licenses"
cp LICENSE "$output/licenses/Evenfall.txt"
cp web/player/src/assets/VT323-LICENSE.txt "$output/licenses/VT323.txt"
cp web/player/src/assets/fonts/PressStart2P-OFL.txt "$output/licenses/PressStart2P.txt"
if [[ "$VITE_PRIVACY_URL" == ./privacy.txt ]]; then
    cp web/cloudflare/PRIVACY.md "$output/privacy.txt"
fi
rm -rf "$root/.dist/cloudflare"
mv "$output" "$root/.dist/cloudflare"
echo "Static deployment ready in .dist/cloudflare ($(rustc --version))."
