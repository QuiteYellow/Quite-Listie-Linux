#!/bin/sh
# Wrapper invoked by meson's cargo-build custom_target.
# Args: <cargo> <manifest-path> <cargo-target-dir> <cargo-out-dir> <meson-output> [extra cargo args...]
set -e

CARGO="$1"
MANIFEST="$2"
TARGET_DIR="$3"
OUT_DIR="$4"
MESON_OUTPUT="$5"
shift 5

export CARGO_TARGET_DIR="$TARGET_DIR"
"$CARGO" build --manifest-path "$MANIFEST" -p quite-listie "$@"
cp "$OUT_DIR/quite-listie" "$MESON_OUTPUT"
