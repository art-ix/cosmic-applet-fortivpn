name := 'cosmic-applet-fortivpn'
appid := 'com.github.artur.CosmicAppletFortiVpn'
bin := '/usr/local/bin' / name
desktop := '/usr/local/share/applications' / appid + '.desktop'
unit := '/etc/systemd/system/cosmic-fortivpn.service'
rule := '/etc/polkit-1/rules.d/50-cosmic-fortivpn.rules'

default: build

build:
    cargo build --release

# Instalacja binarki, .desktop, unitu systemd i reguly polkit (wymaga sudo).
# Najpierw `just build` jako zwykly uzytkownik (cargo nie jest w PATH roota).
install:
    sudo install -Dm755 target/release/{{name}} {{bin}}
    sudo install -Dm644 data/{{appid}}.desktop {{desktop}}
    sudo install -Dm644 data/cosmic-fortivpn.service {{unit}}
    sudo install -Dm644 data/50-cosmic-fortivpn.rules {{rule}}
    sudo systemctl daemon-reload

# Odinstalowanie czesci systemowej (zatrzymuje usluge = rozlacza VPN).
# Pakiet openfortivpn i /etc/openfortivpn/config zostaja nietkniete.
# Wpis w panelu usun osobno: `just uninstall-panel` (jako zwykly uzytkownik).
uninstall:
    -sudo systemctl stop cosmic-fortivpn.service
    sudo rm -f {{bin}} {{desktop}} {{unit}} {{rule}}
    sudo systemctl daemon-reload

# Zdejmuje aplet z konfiguracji panelu COSMIC (uruchom BEZ sudo)
uninstall-panel:
    #!/usr/bin/env bash
    set -euo pipefail
    f="$HOME/.config/cosmic/com.system76.CosmicPanel.Panel/v1/plugins_wings"
    [ -f "$f" ] || exit 0
    grep -q '{{appid}}' "$f" || { echo "brak wpisu w $f"; exit 0; }
    # Usuwamy tylko linie zawierajaca wylacznie ID apletu (opcjonalnie w cudzyslowie, z przecinkiem).
    id='{{appid}}'
    re="^[[:space:]]*\"?${id//./\\.}\"?,?[[:space:]]*$"
    if grep -Eq "$re" "$f"; then
        cp "$f" "$f.bak"
        sed -i -E "/$re/d" "$f"
        echo "usunieto wpis z $f (kopia: $f.bak)"
    else
        echo "wpis w $f nie jest w osobnej linii - usun {{appid}} recznie (Ustawienia -> Panel)" >&2
    fi

# Pelne odinstalowanie: panel + czesc systemowa (uruchom BEZ sudo, sudo wywola sie samo)
uninstall-all: uninstall-panel uninstall
