# Changelog

All notable changes to this project are documented here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/).

## 0.5.0 - 2026-10-12

### Added
- Desktop notifications (`org.freedesktop.Notifications`, shown by `cosmic-notifications`) when the VPN
  connects, the connection drops (reconnecting / disconnected) or fails. Notifications replace each other
  instead of piling up; a disconnect from the applet does not notify. With several monitors (one applet
  process per panel) only one instance notifies. No notifications while the popup is open, and opening it
  closes the current one, so they never cover the popup. The failure notification has a
  *Retry* button.
- Plain-language failure reason in the popup and in the notification: wrong credentials, OTP required,
  untrusted gateway certificate, DNS failure, gateway unreachable, TLS error, missing or invalid config,
  PPP tunnel not up, `openfortivpn` missing, too many attempts, timeout, crash. It is read from the log of the
  last attempt and the unit's `Result`. It is shown, and notified once, as soon as the first attempt fails
  while systemd is still retrying (`Restart=on-failure`), not only after the retries run out.
- *Retry* button in the failed state. Retrying (also by switching the toggle on) clears the unit's failed
  state and systemd's start limit first, so it works right after three failed attempts.
- Coloured status dot on the panel icon: green when connected, blinking orange while connecting or
  disconnecting, red on failure.

## 0.4.0 - 2026-10-12

### Added
- `.deb` package for Pop!_OS / Ubuntu 24.04 (amd64), attached to every release next to the tarball. It installs
  the applet, the systemd unit and the polkit rule under `/usr`, depends on `openfortivpn`, `systemd` and
  `polkitd`, and removes cleanly with `apt remove`.
- Installing the package over an earlier `install.sh` installation removes the leftover files in
  `/usr/local` and `/etc` that would shadow it, without disconnecting a running VPN. Updating the package
  never stops the VPN; removing it does.
- `scripts/build-deb.sh`, `scripts/test-deb.sh` (installs the package in a clean Ubuntu 24.04 container and
  checks migration, libraries, remove and purge) and `just deb`.
- CI builds and tests the `.deb` on every push and pull request.

### Changed
- README describes the `.deb` install first; the tarball is for other distributions.

## 0.3.0 - 2026-10-12

### Changed
- With several PPP links (for example a mobile modem next to the VPN), the applet now picks the interface
  that belongs to the unit (the `pppd` PID from `/run/pppN.pid` found in the unit's cgroup) instead of
  the first `ppp*`. It falls back to the first one when that cannot be determined, which also covers a
  tunnel started outside the applet.
- The log and traffic refresh now also runs while the popup is open and the log is visible (Connecting,
  Failed, or *Show log*), not only while connected, so authentication progress and failures update live.
- The delayed refresh after reaching Connected is skipped if the VPN has already gone down.
- The connection time is read straight from systemd instead of from the property cache.
- Releases are automatic: when CI on `main` passes and the `Cargo.toml` version has no tag yet, the tag and the
  GitHub release are created, with the notes taken from this file.

### Fixed
- Log lines no longer stay stale while the VPN is connecting (regression from 0.2.0, where state updates
  that did not change the VPN state stopped refreshing the log).
- `1.0 MiB` is shown instead of `1024.0 KiB` right below a unit boundary.
- The "no speed yet" placeholder is now a localized string instead of a hardcoded one.
- `just uninstall-panel` matches the applet ID literally (dots are escaped in the pattern).
- Declared the `fs` feature of `tokio` that the code already relied on.

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

