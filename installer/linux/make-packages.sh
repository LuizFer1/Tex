#!/bin/sh
# Builds the .deb and the .rpm of Tex from release binaries already built.
# Usage: sh installer/linux/make-packages.sh <version> <cargo target dir> <output dir>
set -eu
VERSION="$1"
mkdir -p "$2" "$3"
CARGO_TARGET_DIR="$(cd "$2" && pwd)"
export CARGO_TARGET_DIR
OUT="$(cd "$3" && pwd)"
cd "$(dirname "$0")/../.."
cargo deb -p tex-app --no-build --output "$OUT/tex_${VERSION}_amd64.deb"
cargo generate-rpm -p crates/tex-app --target-dir "$CARGO_TARGET_DIR" --output "$OUT/tex-${VERSION}.x86_64.rpm"
ls -l "$OUT"
