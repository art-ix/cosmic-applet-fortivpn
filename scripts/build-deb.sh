#!/usr/bin/env bash
# Buduje paczke .deb (wymaga: `cargo install cargo-deb --locked`, dpkg-dev).
# Uzywane lokalnie (`just deb`), w CI i przy wydaniu. Wynik: target/debian/*.deb + .sha256.
set -euo pipefail
cd "$(dirname "$0")/.."

cargo build --release --locked
cargo deb --no-build --locked

deb="$(ls -t target/debian/cosmic-applet-fortivpn_*.deb | head -n1)"
(cd "$(dirname "$deb")" && sha256sum "$(basename "$deb")" > "$(basename "$deb").sha256")
echo "$deb"
