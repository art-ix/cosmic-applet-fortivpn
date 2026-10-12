//! Informacje pomocnicze: ostatnie linie logu z journald i adres IP tunelu ppp*.

use crate::systemd::UNIT;
use std::time::Duration;
use tokio::process::Command;

#[derive(Debug, Clone, Default)]
pub struct Info {
    pub logs: Vec<String>,
    /// Nazwa interfejsu ppp* i jego adres IPv4, jesli istnieje (niezaleznie od unitu).
    pub ppp: Option<Ppp>,
    /// Liczniki (rx, tx) interfejsu `ppp` w bajtach.
    pub traffic: Option<(u64, u64)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ppp {
    pub ifname: String,
    pub ip: String,
}

pub async fn fetch() -> Info {
    let (logs, ppp) = tokio::join!(logs(), ppp());
    let traffic = match &ppp {
        Some(p) => traffic(&p.ifname).await,
        None => None,
    };
    Info { logs, ppp, traffic }
}

/// Liczniki (rx, tx) z sysfs - dostepne dla zwyklego uzytkownika, liczone od utworzenia interfejsu.
pub async fn traffic(ifname: &str) -> Option<(u64, u64)> {
    let read = |name: &str| {
        let path = format!("/sys/class/net/{ifname}/statistics/{name}");
        async move {
            tokio::fs::read_to_string(path)
                .await
                .ok()?
                .trim()
                .parse()
                .ok()
        }
    };
    let (rx, tx) = tokio::join!(read("rx_bytes"), read("tx_bytes"));
    Some((rx?, tx?))
}

/// Ile linii logu pokazuje popup (pobieranych jest wiecej, na potrzeby diagnozy bledu).
pub const LOG_LINES_SHOWN: usize = 6;

pub async fn logs() -> Vec<String> {
    let out = Command::new("journalctl")
        .args(["-u", UNIT, "-n", "40", "-o", "cat", "--no-pager"])
        .output()
        .await;
    match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout)
            .lines()
            .map(str::to_owned)
            .collect(),
        // np. brak uprawnien (grupa adm / systemd-journal): pokaz powod zamiast pustego logu
        Ok(o) => String::from_utf8_lossy(&o.stderr)
            .lines()
            .map(|l| format!("journalctl: {l}"))
            .collect(),
        Err(e) => vec![format!("journalctl: {e}")],
    }
}

/// Interfejs `ppp*` tunelu: ten, ktorego pppd (`/run/<if>.pid`) dziala w cgroupie naszego unitu;
/// gdy sie nie da ustalic (albo tunel dziala poza apletem) - pierwszy `ppp*` z listy.
async fn ppp() -> Option<Ppp> {
    let out = Command::new("ip")
        .args(["-4", "-o", "addr", "show"])
        .output()
        .await
        .ok()?;
    let all = parse_ppp(&String::from_utf8_lossy(&out.stdout));
    let own = unit_ifname().await;
    pick_ppp(all, own.as_deref())
}

fn pick_ppp(all: Vec<Ppp>, own: Option<&str>) -> Option<Ppp> {
    let i = own
        .and_then(|o| all.iter().position(|p| p.ifname == o))
        .unwrap_or(0);
    all.into_iter().nth(i)
}

/// Nazwa interfejsu pppd uruchomionego przez nasz unit (PID z `/run/pppN.pid` w jego cgroupie).
async fn unit_ifname() -> Option<String> {
    let procs =
        tokio::fs::read_to_string(format!("/sys/fs/cgroup/system.slice/{UNIT}/cgroup.procs"))
            .await
            .ok()?;
    let mut dir = tokio::fs::read_dir("/run").await.ok()?;
    while let Ok(Some(entry)) = dir.next_entry().await {
        let name = entry.file_name();
        let Some(ifname) = name
            .to_str()
            .and_then(|n| n.strip_suffix(".pid"))
            .filter(|n| n.starts_with("ppp"))
        else {
            continue;
        };
        let Ok(pid) = tokio::fs::read_to_string(entry.path()).await else {
            continue;
        };
        if procs.split_whitespace().any(|p| p == pid.trim()) {
            return Some(ifname.to_owned());
        }
    }
    None
}

/// Interfejsy `ppp*` z adresem IPv4 w wyjsciu `ip -4 -o addr show` (w kolejnosci wyjscia).
fn parse_ppp(s: &str) -> Vec<Ppp> {
    s.lines()
        .filter_map(|line| {
            let mut it = line.split_whitespace();
            // format: "7: ppp0    inet 10.0.0.1 peer ..."
            let ifname = it.nth(1)?;
            if !ifname.starts_with("ppp") {
                return None;
            }
            let addr = it.skip_while(|t| *t != "inet").nth(1)?;
            Some(Ppp {
                ifname: ifname.to_owned(),
                ip: addr.split('/').next().unwrap_or(addr).to_owned(),
            })
        })
        .collect()
}

/// Rozmiar w jednostkach binarnych, np. `12.3 MiB`.
pub fn format_bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut v = n as f64;
    let mut i = 0;
    // porownanie po zaokragleniu do 0.1, zeby 1023.96 KiB nie wyszlo jako "1024.0 KiB"
    while (v * 10.0).round() / 10.0 >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", UNITS[i])
    }
}

/// Czas trwania w dwoch najwiekszych jednostkach, np. `1h 23m`, `4m 05s`, `45s`.
pub fn format_duration(d: Duration) -> String {
    let s = d.as_secs();
    let (days, h, m, sec) = (s / 86_400, s / 3600 % 24, s / 60 % 60, s % 60);
    if days > 0 {
        format!("{days}d {h}h")
    } else if h > 0 {
        format!("{h}h {m:02}m")
    } else if m > 0 {
        format!("{m}m {sec:02}s")
    } else {
        format!("{sec}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ppp(ifname: &str, ip: &str) -> Ppp {
        Ppp {
            ifname: ifname.to_owned(),
            ip: ip.to_owned(),
        }
    }

    #[test]
    fn parses_ppp_output() {
        let s = "7: ppp0    inet 10.212.134.204 peer 169.254.2.1/32 scope global ppp0\\ valid_lft forever";
        assert_eq!(parse_ppp(s), vec![ppp("ppp0", "10.212.134.204")]);
        assert_eq!(parse_ppp(""), vec![]);

        let multi = "2: eth0    inet 192.168.1.5/24 brd 192.168.1.255 scope global eth0\n\
                     8: ppp0    inet 10.9.9.9 peer 10.9.9.1/32 scope global ppp0\n\
                     9: ppp1    inet 10.1.2.3 peer 169.254.2.1/32 scope global ppp1";
        assert_eq!(
            parse_ppp(multi),
            vec![ppp("ppp0", "10.9.9.9"), ppp("ppp1", "10.1.2.3")]
        );
        assert_eq!(
            parse_ppp("2: eth0    inet 192.168.1.5/24 scope global eth0"),
            vec![]
        );
    }

    #[test]
    fn picks_the_units_interface() {
        let all = || vec![ppp("ppp0", "10.9.9.9"), ppp("ppp1", "10.1.2.3")];
        assert_eq!(pick_ppp(all(), Some("ppp1")), Some(ppp("ppp1", "10.1.2.3")));
        // nieznany lub niewidoczny interfejs unitu -> pierwszy
        assert_eq!(pick_ppp(all(), Some("ppp7")), Some(ppp("ppp0", "10.9.9.9")));
        assert_eq!(pick_ppp(all(), None), Some(ppp("ppp0", "10.9.9.9")));
        assert_eq!(pick_ppp(vec![], Some("ppp0")), None);
    }

    #[test]
    fn formats_bytes() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1023), "1023 B");
        assert_eq!(format_bytes(1024), "1.0 KiB");
        assert_eq!(format_bytes(12_897_484), "12.3 MiB");
        assert_eq!(format_bytes(1_048_575), "1.0 MiB");
        assert_eq!(format_bytes(1_048_000), "1023.4 KiB");
        assert_eq!(format_bytes(5 * 1024 * 1024 * 1024), "5.0 GiB");
    }

    #[test]
    fn formats_duration() {
        assert_eq!(format_duration(Duration::from_secs(45)), "45s");
        assert_eq!(format_duration(Duration::from_secs(245)), "4m 05s");
        assert_eq!(format_duration(Duration::from_secs(4980)), "1h 23m");
        assert_eq!(format_duration(Duration::from_secs(90_000)), "1d 1h");
    }
}
