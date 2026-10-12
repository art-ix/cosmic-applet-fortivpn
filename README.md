# cosmic-applet-fortivpn

A small [COSMIC](https://github.com/pop-os/cosmic-epoch) panel applet (Pop!_OS 24.04 / COSMIC 1.10)
that turns [`openfortivpn`](https://github.com/adrienverge/openfortivpn) on and off from the panel,
next to the Network applet.

- One toggle in the panel popup, with live status (connecting / connected / disconnecting / failed).
- Shows the tunnel IP (`ppp0`) and the last lines of the service log. Click the status line to copy the IP.
- While connected and the popup is open: data sent/received since the tunnel came up, live speed, and connection time.
- Reacts to changes made elsewhere (e.g. `systemctl stop` in a terminal) without polling.
- Uses your existing `/etc/openfortivpn/config` — the applet never reads it, so credentials stay root-only.

> The UI follows your session language. English is the default and is used whenever your
> language has no translation. Available: English (`en`), Polish (`pl`), German (`de`), Spanish (`es`),
> Italian (`it`), French (`fr`).

## How it works

```
panel ── applet (your user) ──D-Bus──> systemd ──> cosmic-fortivpn.service ──> /usr/bin/openfortivpn
                │                                   (reads /etc/openfortivpn/config)
                └─ polkit rule: your user may manage ONLY this one unit without a password
```

The applet does not run `openfortivpn` itself. It starts/stops a dedicated systemd unit
(`cosmic-fortivpn.service`) and watches its `ActiveState` over D-Bus. A polkit rule allows
local, active users in the `sudo` group to manage that single unit without a password;
every other unit still requires authentication.

## Requirements

- Pop!_OS 24.04 with COSMIC (other COSMIC distributions should work, but are untested)
- `openfortivpn` with a working, **non-interactive** `/etc/openfortivpn/config`
  (no OTP/SAML prompts — the service has no terminal)
- NetworkManager is not required

## Install from a release

```sh
tar xzf cosmic-applet-fortivpn-vX.Y.Z-linux-x86_64.tar.gz
cd cosmic-applet-fortivpn-vX.Y.Z-linux-x86_64
sudo ./install.sh
```

Then add the applet: **Settings → Desktop → Panel → Configure panel applets → FortiVPN**.

Installed files:

| File | Purpose |
|---|---|
| `/usr/local/bin/cosmic-applet-fortivpn` | the applet |
| `/usr/local/share/applications/com.github.artur.CosmicAppletFortiVpn.desktop` | lets the panel find it |
| `/etc/systemd/system/cosmic-fortivpn.service` | runs `openfortivpn` |
| `/etc/polkit-1/rules.d/50-cosmic-fortivpn.rules` | passwordless start/stop of that unit only |

## Build from source

```sh
sudo apt install build-essential pkg-config just \
    libxkbcommon-dev libwayland-dev libinput-dev libudev-dev
cargo build --release
sudo ./install.sh        # or: just build && sudo just install
```

`cargo` is usually not in root's `PATH`, so build as your user first and only install with `sudo`.

`libcosmic` is pinned to the revision used by `cosmic-applets` `epoch-1.10.0` (see `Cargo.toml`).
Its API is unstable; when COSMIC updates, bump the `rev` to the matching `Cargo.lock` entry of
`cosmic-applets`.

## Configuration

There is no applet configuration; everything comes from `/etc/openfortivpn/config`.
Changes take effect on the next start (toggle off and on, or `systemctl restart cosmic-fortivpn`).

To pass extra options to `openfortivpn`, edit `ExecStart` in
`/etc/systemd/system/cosmic-fortivpn.service` and run `sudo systemctl daemon-reload`.

If you are not in the `sudo` group, change `subject.isInGroup("sudo")` in
`data/50-cosmic-fortivpn.rules` to your group or user before installing.

## Uninstall

```sh
just uninstall-all         # panel entry + system files (run WITHOUT sudo)
# or: sudo ./install.sh --uninstall   and remove the applet in panel settings
```

Stopping the service disconnects the VPN. The `openfortivpn` package and your
`/etc/openfortivpn/config` are left untouched.

## Troubleshooting

- **Toggle asks for a password / fails** — check that `/etc/polkit-1/rules.d/50-cosmic-fortivpn.rules`
  exists and that you are in the `sudo` group.
- **Stays in "connecting" then fails** — open the popup → *Show log*, or run
  `journalctl -u cosmic-fortivpn -e`.
- **"tunnel ppp0 running outside the applet"** — an `openfortivpn` started by hand is still running;
  stop it before using the toggle.
- **Applet missing in the panel list** — make sure the `.desktop` file is in `/usr/local/share/applications`
  and restart the panel session.

## Translations

Strings live in `i18n/<lang>/cosmic_applet_fortivpn.ftl` ([Fluent](https://projectfluent.org/)).
To add a language, copy `i18n/en/cosmic_applet_fortivpn.ftl` to `i18n/<lang>/`, translate the
values (keep the keys and `{ $placeholders }`), rebuild. Missing keys fall back to English.

## Releases

Pushing a tag `vX.Y.Z` (must match `version` in `Cargo.toml`) builds the applet in GitHub Actions
and publishes a release with a `.tar.gz` and its SHA-256:

```sh
git tag v0.1.0 && git push origin v0.1.0
```

## License

MIT — see [LICENSE](LICENSE).
