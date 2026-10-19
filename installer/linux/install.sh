#!/bin/sh
# Installs Tex from this folder, as root: sudo ./install.sh [--prefix DIR]  (default /usr/local)
# Removes it: sudo ./install.sh --uninstall [--prefix DIR]. The search index in /var/lib/tex stays.
# With DESTDIR set, stages the files under DESTDIR without root, systemctl or icon caches.
set -eu

PREFIX=/usr/local
ACTION=install
while [ $# -gt 0 ]; do
    case "$1" in
        --prefix)
            PREFIX="$2"
            shift 2
            ;;
        --uninstall)
            ACTION=uninstall
            shift
            ;;
        *)
            echo "usage: install.sh [--prefix DIR] [--uninstall]" >&2
            exit 2
            ;;
    esac
done

DESTDIR="${DESTDIR:-}"
HERE="$(cd "$(dirname "$0")" && pwd)"
APP_ID=io.github.LuizFer1.Tex
LIB="$PREFIX/lib/tex"
SHARE="$DESTDIR$PREFIX/share"
EXTENSION="$SHARE/gnome-shell/extensions/tex@luizfer1.github.io"
UNIT="$DESTDIR/etc/systemd/system/tex-indexer.service"
POLICY="$DESTDIR/usr/share/polkit-1/actions/$APP_ID.update.policy"
AUTOSTART="$DESTDIR/etc/xdg/autostart/$APP_ID.desktop"
SIZES="16 24 32 48 64 128 256"

if [ -z "$DESTDIR" ] && [ "$(id -u)" -ne 0 ]; then
    echo "install.sh: run it as root, for example: sudo ./install.sh" >&2
    exit 1
fi

manage_service() {
    [ -z "$DESTDIR" ] && [ -d /run/systemd/system ]
}

with_prefix() {
    sed "s|/usr/lib/tex|$LIB|g" "$1" > "$2"
    chmod 644 "$2"
}

uninstall() {
    if manage_service; then
        systemctl disable --now tex-indexer.service 2>/dev/null || true
    fi
    rm -f "$UNIT" "$POLICY" "$AUTOSTART" "$DESTDIR$PREFIX/bin/tex"
    rm -f "$SHARE/applications/$APP_ID.desktop" "$SHARE/metainfo/$APP_ID.metainfo.xml"
    for size in $SIZES; do
        rm -f "$SHARE/icons/hicolor/${size}x${size}/apps/$APP_ID.png"
    done
    rm -rf "$DESTDIR$LIB" "$EXTENSION"
    if manage_service; then
        systemctl daemon-reload
    fi
}

install_files() {
    install -d -m 755 "$DESTDIR$LIB" "$DESTDIR$PREFIX/bin" "$SHARE/applications" "$SHARE/metainfo" \
        "$EXTENSION" "$(dirname "$UNIT")" "$(dirname "$POLICY")" "$(dirname "$AUTOSTART")"
    install -m 755 "$HERE/tex" "$HERE/tex-indexer" "$DESTDIR$LIB/"
    install -m 644 "$HERE/install-kind" "$DESTDIR$LIB/install-kind"
    ln -sf "$LIB/tex" "$DESTDIR$PREFIX/bin/tex"
    with_prefix "$HERE/tex-indexer.service" "$UNIT"
    with_prefix "$HERE/$APP_ID.update.policy" "$POLICY"
    with_prefix "$HERE/$APP_ID.autostart.desktop" "$AUTOSTART"
    with_prefix "$HERE/$APP_ID.desktop" "$SHARE/applications/$APP_ID.desktop"
    install -m 644 "$HERE/$APP_ID.metainfo.xml" "$SHARE/metainfo/$APP_ID.metainfo.xml"
    for size in $SIZES; do
        install -D -m 644 "$HERE/icons/$size.png" "$SHARE/icons/hicolor/${size}x${size}/apps/$APP_ID.png"
    done
    install -m 644 "$HERE/gnome-extension/extension.js" "$HERE/gnome-extension/metadata.json" \
        "$HERE/gnome-extension/LICENSE" "$EXTENSION/"
    if [ -z "$DESTDIR" ] && command -v gtk-update-icon-cache >/dev/null 2>&1; then
        gtk-update-icon-cache -q -t "$SHARE/icons/hicolor" || true
    fi
    if manage_service; then
        systemctl daemon-reload
        systemctl enable tex-indexer.service
        systemctl restart tex-indexer.service
    fi
}

if [ "$ACTION" = uninstall ]; then
    uninstall
    echo "Tex removed. The search index stays in /var/lib/tex."
else
    install_files
    echo "Tex installed in $LIB. It starts at the next login; or open it now from the app menu."
fi
