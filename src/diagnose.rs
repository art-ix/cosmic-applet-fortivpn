//! Czytelny powod nieudanego polaczenia: dopasowanie komunikatow openfortivpn/systemd z logu
//! ostatniej proby oraz pola `Result` uslugi.

use crate::fl;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// Bledne haslo/certyfikat klienta albo brama odrzucila logowanie.
    Auth,
    /// Brama wymaga kodu jednorazowego (usluga nie ma terminala).
    Otp,
    /// Certyfikat bramy nie przeszedl weryfikacji i nie ma go na liscie zaufanych.
    Certificate,
    /// Nie udalo sie rozwiazac nazwy bramy.
    Dns,
    /// Brama nieosiagalna (odmowa polaczenia, brak trasy, timeout).
    Unreachable,
    /// Blad TLS przy polaczeniu z brama.
    Tls,
    /// Brak lub niepoprawny `/etc/openfortivpn/config`.
    Config,
    /// Tunel PPP nie wstal.
    Ppp,
    /// Brak programu `openfortivpn`.
    NotInstalled,
    /// systemd zablokowal kolejne proby (`StartLimitBurst`).
    StartLimit,
    /// Usluga nie zglosila gotowosci na czas.
    Timeout,
    /// openfortivpn zakonczyl sie sygnalem / zrzutem pamieci.
    Crashed,
}

impl Reason {
    pub fn text(self) -> String {
        match self {
            Self::Auth => fl!("reason-auth"),
            Self::Otp => fl!("reason-otp"),
            Self::Certificate => fl!("reason-certificate"),
            Self::Dns => fl!("reason-dns"),
            Self::Unreachable => fl!("reason-unreachable"),
            Self::Tls => fl!("reason-tls"),
            Self::Config => fl!("reason-config"),
            Self::Ppp => fl!("reason-ppp"),
            Self::NotInstalled => fl!("reason-not-installed"),
            Self::StartLimit => fl!("reason-start-limit"),
            Self::Timeout => fl!("reason-timeout"),
            Self::Crashed => fl!("reason-crashed"),
        }
    }
}

/// Wzorce sprawdzane w kolejnosci (`SSL_connect:` przed `connect:`).
const PATTERNS: &[(&str, Reason)] = &[
    ("Could not authenticate to gateway", Reason::Auth),
    ("Could not authenticate to the gateway", Reason::Auth),
    ("No password given", Reason::Auth),
    ("VPN Requires a Password", Reason::Auth),
    ("No OTP specified", Reason::Otp),
    ("Two-factor authentication token", Reason::Otp),
    ("Please enter one-time password", Reason::Otp),
    ("Gateway certificate validation failed", Reason::Certificate),
    ("getaddrinfo:", Reason::Dns),
    ("SSL_connect:", Reason::Tls),
    ("connect:", Reason::Unreachable),
    ("Could not load configuration file", Reason::Config),
    ("Specify a valid host:port couple", Reason::Config),
    ("Timed out waiting for the ppp interface", Reason::Ppp),
    ("Failed at step EXEC", Reason::NotInstalled),
    ("Unable to locate executable", Reason::NotInstalled),
    ("Failed to locate executable", Reason::NotInstalled),
];

/// Powod porazki. `logs` - linie journald od najstarszej; brane sa tylko linie ostatniej proby
/// (po ostatnim `Starting ...` systemd). `result` - wlasciwosc `Result` uslugi.
pub fn diagnose(logs: &[String], result: Option<&str>) -> Option<Reason> {
    let last_attempt = logs
        .iter()
        .rev()
        .take_while(|l| !l.starts_with("Starting "));
    for line in last_attempt {
        if let Some((_, r)) = PATTERNS.iter().find(|(p, _)| line.contains(p)) {
            return Some(*r);
        }
    }
    match result? {
        "start-limit-hit" => Some(Reason::StartLimit),
        "timeout" => Some(Reason::Timeout),
        "signal" | "core-dump" => Some(Reason::Crashed),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(s: &[&str]) -> Vec<String> {
        s.iter().map(|l| l.to_string()).collect()
    }

    #[test]
    fn finds_reason_in_last_attempt() {
        let logs = lines(&[
            "ERROR:  getaddrinfo: Name or service not known",
            "Starting cosmic-fortivpn.service - OpenFortiVPN (COSMIC applet)...",
            "INFO:   Connected to gateway.",
            "ERROR:  Could not authenticate to gateway. Please check the password, client certificate, etc.",
            "INFO:   Closed connection to gateway.",
            "cosmic-fortivpn.service: Main process exited, code=exited, status=1/FAILURE",
        ]);
        assert_eq!(diagnose(&logs, Some("exit-code")), Some(Reason::Auth));
    }

    #[test]
    fn ignores_previous_attempts() {
        let logs = lines(&[
            "ERROR:  Could not authenticate to gateway.",
            "Starting cosmic-fortivpn.service - OpenFortiVPN (COSMIC applet)...",
            "cosmic-fortivpn.service: Main process exited, code=exited, status=1/FAILURE",
        ]);
        assert_eq!(diagnose(&logs, Some("exit-code")), None);
    }

    #[test]
    fn ssl_connect_is_not_unreachable() {
        let tls = lines(&["ERROR:  SSL_connect: error:0A000126:SSL routines::unexpected eof"]);
        assert_eq!(diagnose(&tls, None), Some(Reason::Tls));
        let down = lines(&["ERROR:  connect: Connection refused"]);
        assert_eq!(diagnose(&down, None), Some(Reason::Unreachable));
    }

    #[test]
    fn newest_match_wins() {
        let logs = lines(&[
            "ERROR:  connect: Connection timed out",
            "ERROR:  Gateway certificate validation failed, and the certificate digest is not in the local whitelist.",
        ]);
        assert_eq!(diagnose(&logs, None), Some(Reason::Certificate));
    }

    #[test]
    fn falls_back_to_unit_result() {
        let logs = lines(&["cosmic-fortivpn.service: Start request repeated too quickly."]);
        assert_eq!(
            diagnose(&logs, Some("start-limit-hit")),
            Some(Reason::StartLimit)
        );
        assert_eq!(diagnose(&[], Some("core-dump")), Some(Reason::Crashed));
        assert_eq!(diagnose(&[], Some("exit-code")), None);
        assert_eq!(diagnose(&[], None), None);
    }
}
