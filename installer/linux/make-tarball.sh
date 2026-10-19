#!/bin/sh
# Builds tex-<version>-linux-x86_64.tar.gz (with install.sh) from release binaries already built.
# Usage: sh installer/linux/make-tarball.sh <version> <cargo target dir> <output dir>
set -eu
VERSION="$1"
TARGET="$(cd "$2" && pwd)"
mkdir -p "$3"
OUT="$(cd "$3" && pwd)"
cd "$(dirname "$0")/../.."
NAME="tex-$VERSION-linux-x86_64"
STAGE="$TARGET/linux-stage"
DIR="$STAGE/$NAME"
rm -rf "$STAGE"
mkdir -p "$DIR/icons" "$DIR/gnome-extension"
cp "$TARGET/release/tex" "$TARGET/release/tex-indexer" "$DIR/"
cp installer/linux/install.sh installer/linux/tex-indexer.service \
    installer/linux/io.github.LuizFer1.Tex.desktop installer/linux/io.github.LuizFer1.Tex.autostart.desktop \
    installer/linux/io.github.LuizFer1.Tex.metainfo.xml installer/linux/io.github.LuizFer1.Tex.update.policy \
    LICENSE.md "$DIR/"
cp installer/linux/install-kind/tar "$DIR/install-kind"
cp installer/linux/icons/*.png "$DIR/icons/"
cp gnome-extension/extension.js gnome-extension/metadata.json gnome-extension/LICENSE "$DIR/gnome-extension/"
chmod 755 "$DIR/install.sh" "$DIR/tex" "$DIR/tex-indexer"
tar -czf "$OUT/$NAME.tar.gz" --owner=0 --group=0 -C "$STAGE" "$NAME"
rm -rf "$STAGE"
echo "$OUT/$NAME.tar.gz"
