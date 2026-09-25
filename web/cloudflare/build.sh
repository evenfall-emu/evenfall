#!/usr/bin/env bash
set -euo pipefail

# Run from the repository root. Deployment builds need only Zig and Node; do
# not run research-environment setup here.
root="$(git rev-parse --show-toplevel)"
if [[ "$PWD" != "$root" ]]; then
    echo "Run tools/build-cloudflare.sh from the repository root." >&2
    exit 1
fi

mkdir -p .dist
temporary="$(mktemp -d "$root/.dist/cloudflare-build.XXXXXX")"
trap 'rm -rf "$temporary"' EXIT

zig_version=0.16.0
if command -v zig >/dev/null && [[ "$(zig version)" == "$zig_version" ]]; then
    zig_bin="$(command -v zig)"
else
    if [[ "$(uname -s)-$(uname -m)" != Linux-x86_64 ]]; then
        echo "Install Zig $zig_version before building on this platform." >&2
        exit 1
    fi
    archive="$temporary/zig.tar.xz"
    curl --fail --location --retry 3 --silent --show-error \
        "https://ziglang.org/download/$zig_version/zig-x86_64-linux-$zig_version.tar.xz" \
        --output "$archive"
    echo "70e49664a74374b48b51e6f3fdfbf437f6395d42509050588bd49abe52ba3d00  $archive" | sha256sum --check
    tar -xJf "$archive" -C "$temporary"
    zig_bin="$temporary/zig-x86_64-linux-$zig_version/zig"
fi

# One root install serves every npm workspace (SDK, Pokémon plugin, player,
# server). The player build below type-checks the frontend.
npm ci
npm run check -w server
npm test -w server
npm run check -w web -w games/pokemon
npm test -w games/pokemon -w frontend

# Instance settings for the official play.evenfall-emu.org deployment. These are
# public values, not secrets. A self-hosted deployment exports its own values
# before running this script; an empty value disables the feature. Analytics
# are sent only from pages served on VITE_POSTHOG_HOSTNAME, so previews and
# other hosts of this build stay silent. See frontend/README.md#analytics.
export VITE_POSTHOG_KEY="${VITE_POSTHOG_KEY-phc_qRjgGwT22cQ2yYu8Zkgx2hc44NtrgRBeubAyXm9u9vCi}"
export VITE_POSTHOG_HOST="${VITE_POSTHOG_HOST-https://eu.i.posthog.com}"
export VITE_POSTHOG_HOSTNAME="${VITE_POSTHOG_HOSTNAME-play.evenfall-emu.org}"
# The player links this privacy notice; ./privacy.txt publishes PRIVACY.md.
export VITE_PRIVACY_URL="${VITE_PRIVACY_URL-./privacy.txt}"
"$zig_bin" build web -Doptimize=ReleaseSafe --prefix "$temporary/output"

# Never publish the ordinary zig-out tree: it can contain stale build files.
# Reject game media even if somebody accidentally puts it in the SDK directory.
private_files="$(find "$temporary/output/web" -type f \( \
    -iname '*.gba' -o -iname '*.sav' -o -iname '*.state' -o \
    -iname '*.bin' -o -iname '*.zip' -o -iname '*.tar*' \))"
if [[ -n "$private_files" ]]; then
    echo "Refusing to publish game media/archive files:" >&2
    printf '%s\n' "$private_files" >&2
    exit 1
fi
test -s "$temporary/output/web/index.html"
test -s "$temporary/output/web/service-worker.js"
# The player bundles the SDK: its worker and WASM are hashed assets.
compgen -G "$temporary/output/web/assets/worker-*.js" >/dev/null
compgen -G "$temporary/output/web/assets/zigba-*.wasm" >/dev/null
cp tools/cloudflare-headers "$temporary/output/web/_headers"
mkdir -p "$temporary/output/web/licenses"
cp LICENSE "$temporary/output/web/licenses/ZiGBA.txt"
cp frontend/src/assets/VT323-LICENSE.txt "$temporary/output/web/licenses/VT323.txt"
cp frontend/src/assets/fonts/PressStart2P-OFL.txt "$temporary/output/web/licenses/PressStart2P.txt"
if [[ "$VITE_PRIVACY_URL" == ./privacy.txt ]]; then
    cp PRIVACY.md "$temporary/output/web/privacy.txt"
fi
rm -rf "$root/.dist/cloudflare"
mv "$temporary/output/web" "$root/.dist/cloudflare"
echo "Static deployment ready in .dist/cloudflare (ReleaseSafe, Zig $zig_version)."
