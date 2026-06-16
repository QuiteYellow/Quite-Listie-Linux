#!/bin/sh
# Regenerate build-aux/cargo-sources.json from Cargo.lock so the flatpak build
# can fetch every crate offline. Run this whenever Cargo.lock changes.
#
# Needs: python3 + aiohttp + toml, and flatpak-cargo-generator.py from
#   https://github.com/flatpak/flatpak-builder-tools/tree/master/cargo
set -e

here=$(dirname "$0")
root=$(cd "$here/.." && pwd)
gen="$here/flatpak-cargo-generator.py"

if [ ! -f "$gen" ]; then
    echo "Fetching flatpak-cargo-generator.py..."
    curl -fsSL \
        https://raw.githubusercontent.com/flatpak/flatpak-builder-tools/master/cargo/flatpak-cargo-generator.py \
        -o "$gen"
fi

python3 "$gen" "$root/Cargo.lock" -o "$here/cargo-sources.json"
echo "Wrote $here/cargo-sources.json"
