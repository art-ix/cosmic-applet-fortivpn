# Changelog

All notable changes to this project are documented here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/).

## 0.2.0 - 2026-10-12

### Added
- Traffic statistics in the popup while connected: data received and sent since the tunnel came up,
  read from `/sys/class/net/ppp*/statistics` (no root needed).
- Live transfer speed, sampled every 2 s. The sampling runs only while the popup is open and the VPN
  is connected.
- Connection time, taken from the unit's `ActiveEnterTimestamp`, so it is correct even if the applet
  started after the tunnel.
- Copy the tunnel IP to the clipboard with the button next to the address.
- A `title` string for the toggle label, translated in all languages.
- Restart throttling in the systemd unit (`RestartSec=10`, `StartLimitBurst=3` within 120 s), so wrong
  credentials or an unreachable gateway no longer hammer the VPN gateway.
- polkit rule also matches the `wheel` group (Fedora, Arch), not only `sudo`.
- Own SVG icons (download, upload, copy, check) embedded in the binary; no icon-set dependency.

### Changed
- Redesigned popup: state with a coloured dot (green, yellow, red, grey) and connection time, the tunnel
  IP as the main value, and two tiles for received and sent data, using the theme's colours. Text now
  uses only two sizes (14 px and 20 px), plus 12 px for log lines.
- The popup has a fixed width (320 px) so it does not resize as values change.
- The tunnel is detected on any `ppp*` interface, not only `ppp0`.
- State updates that do not change the VPN state no longer spawn `journalctl` and `ip`. After a switch to
  Connected, the info is refreshed once more after 2 s to pick up the IP and log lines.
- `just uninstall-panel` removes only a line that is exactly the applet ID and keeps a `.bak` copy of the
  panel config. If the ID is not on its own line it warns instead of editing.

### Fixed
- D-Bus connection errors are no longer shown after the connection recovers; they are cleared by the
  next state update.
- The D-Bus watch loop stops when the UI is gone, and logs when the signal stream ends instead of
  retrying silently.
- An empty log view when `journalctl` fails (for example missing `adm`/`systemd-journal` group) now shows
  the reason.
- Declared the `time` feature of `tokio` that the code already relied on, and removed the unused
  `futures` dependency.

## 0.1.0

### Added
- Initial release: COSMIC panel applet with one toggle that starts and stops `openfortivpn` through the
  `cosmic-fortivpn.service` systemd unit.
- Live status (connecting, connected, disconnecting, failed) from D-Bus signals, without polling.
- Tunnel IP and the last lines of the service log.
- Passwordless start and stop for that single unit through a scoped polkit rule.
- Translations: English, Polish, German, Spanish, Italian, French.
- `install.sh` and `justfile` for installation and uninstallation; CI and release workflows.

