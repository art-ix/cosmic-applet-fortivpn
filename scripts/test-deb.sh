#!/usr/bin/env bash
# Testuje paczke .deb w czystym kontenerze ubuntu:24.04 (wymaga dockera):
# metadane, instalacja z zaleznosciami, migracja po starym install.sh, brakujace biblioteki, unit, remove/purge.
# Uzycie: scripts/test-deb.sh [sciezka.deb]   (domyslnie najnowsza z target/debian)
set -euo pipefail
cd "$(dirname "$0")/.."

deb="${1:-$(ls -t target/debian/cosmic-applet-fortivpn_*.deb | head -n1)}"
[ -f "$deb" ] || { echo "Brak pliku .deb: $deb" >&2; exit 1; }
deb="$(realpath "$deb")"

docker run --rm -e DEBIAN_FRONTEND=noninteractive -v "$deb:/pkg.deb:ro" ubuntu:24.04 bash -euo pipefail -c '
fail() { echo "FAIL: $*" >&2; exit 1; }

echo "== metadane"
dpkg-deb -I /pkg.deb | grep -E "Package|Version|Architecture|Depends|Recommends|Maintainer"
dpkg-deb -I /pkg.deb | grep -q "openfortivpn" || fail "brak zaleznosci openfortivpn"
dpkg-deb -c /pkg.deb | tee /tmp/contents
for p in ./usr/bin/cosmic-applet-fortivpn \
         ./usr/share/applications/com.github.artur.CosmicAppletFortiVpn.desktop \
         ./usr/lib/systemd/system/cosmic-fortivpn.service \
         ./usr/share/polkit-1/rules.d/50-cosmic-fortivpn.rules \
         ./usr/share/doc/cosmic-applet-fortivpn/copyright; do
    grep -q " $p\$" /tmp/contents || fail "w paczce brakuje $p"
done
grep -q "etc/openfortivpn" /tmp/contents && fail "paczka nie moze zawierac /etc/openfortivpn"

echo "== instalacja z pozostalosciami po install.sh"
apt-get update -qq
legacy="/usr/local/bin/cosmic-applet-fortivpn
/usr/local/share/applications/com.github.artur.CosmicAppletFortiVpn.desktop
/etc/systemd/system/cosmic-fortivpn.service
/etc/polkit-1/rules.d/50-cosmic-fortivpn.rules"
for f in $legacy; do mkdir -p "$(dirname "$f")"; echo stale > "$f"; done
apt-get install -y -qq /pkg.deb >/tmp/install.log 2>&1 || { cat /tmp/install.log; fail "apt install"; }
for f in $legacy; do [ ! -e "$f" ] || fail "stary plik nie zostal usuniety: $f"; done
grep -q "removed legacy file" /tmp/install.log || fail "brak komunikatu o migracji"

echo "== pakiet zainstalowany"
test -x /usr/bin/cosmic-applet-fortivpn || fail "brak binarki"
test -x /usr/bin/openfortivpn || fail "openfortivpn nie zostal doinstalowany"
missing="$(ldd /usr/bin/cosmic-applet-fortivpn | grep "not found" || true)"
[ -z "$missing" ] || fail "brakujace biblioteki: $missing"
systemd-analyze verify /usr/lib/systemd/system/cosmic-fortivpn.service 2>&1 | grep -v "openfortivpn/config" || true
grep -q "cosmic-fortivpn.service" /usr/share/polkit-1/rules.d/50-cosmic-fortivpn.rules || fail "regula polkit"

echo "== remove / purge"
apt-get remove -y -qq cosmic-applet-fortivpn >/dev/null 2>&1 || fail "apt remove"
for f in /usr/bin/cosmic-applet-fortivpn /usr/lib/systemd/system/cosmic-fortivpn.service \
         /usr/share/polkit-1/rules.d/50-cosmic-fortivpn.rules; do
    [ ! -e "$f" ] || fail "po remove zostal $f"
done
apt-get install -y -qq /pkg.deb >/dev/null 2>&1 || fail "ponowna instalacja"
apt-get purge -y -qq cosmic-applet-fortivpn >/dev/null 2>&1 || fail "apt purge"
echo "OK"
'
