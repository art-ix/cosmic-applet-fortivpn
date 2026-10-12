#!/usr/bin/env bash
# Installer for cosmic-applet-fortivpn. Works from an unpacked release tarball
# (binary next to this script) or from the repo after `cargo build --release`.
#
#   sudo ./install.sh              # install
#   sudo ./install.sh --uninstall  # uninstall (stops the service = disconnects the VPN)
set -euo pipefail

NAME=cosmic-applet-fortivpn
APPID=com.github.artur.CosmicAppletFortiVpn
BIN=/usr/local/bin/$NAME
DESKTOP=/usr/local/share/applications/$APPID.desktop
UNIT=/etc/systemd/system/cosmic-fortivpn.service
RULE=/etc/polkit-1/rules.d/50-cosmic-fortivpn.rules

if [ "$(id -u)" -ne 0 ]; then
    echo "Run as root: sudo $0 $*" >&2
    exit 1
fi

if [ "${1:-}" = "--uninstall" ]; then
    systemctl stop cosmic-fortivpn.service 2>/dev/null || true
    rm -f "$BIN" "$DESKTOP" "$UNIT" "$RULE"
    systemctl daemon-reload
    echo "Uninstalled. Remove the applet from the panel in Settings -> Desktop -> Panel (or run: just uninstall-panel)."
    exit 0
fi

here="$(cd "$(dirname "$0")" && pwd)"
if [ -x "$here/$NAME" ]; then
    src_bin="$here/$NAME"
elif [ -x "$here/target/release/$NAME" ]; then
    src_bin="$here/target/release/$NAME"
else
    echo "Binary $NAME not found (build it with: cargo build --release)." >&2
    exit 1
fi

if [ ! -e /usr/bin/openfortivpn ]; then
    echo "Warning: /usr/bin/openfortivpn is missing (sudo apt install openfortivpn)." >&2
fi
if [ ! -e /etc/openfortivpn/config ]; then
    echo "Warning: /etc/openfortivpn/config is missing - the service will not start without it." >&2
fi

install -Dm755 "$src_bin" "$BIN"
install -Dm644 "$here/data/$APPID.desktop" "$DESKTOP"
install -Dm644 "$here/data/cosmic-fortivpn.service" "$UNIT"
install -Dm644 "$here/data/50-cosmic-fortivpn.rules" "$RULE"
systemctl daemon-reload

echo "Installed. Add the 'FortiVPN' applet in Settings -> Desktop -> Panel -> Configure panel applets."
