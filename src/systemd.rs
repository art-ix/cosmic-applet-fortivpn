//! Klient D-Bus do systemd: start/stop unitu i obserwacja jego stanu.

use std::time::{Duration, SystemTime, UNIX_EPOCH};
use zbus::{Connection, proxy, zvariant::OwnedObjectPath};

pub const UNIT: &str = "cosmic-fortivpn.service";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VpnState {
    Inactive,
    Connecting,
    Connected,
    Disconnecting,
    Failed,
}

impl VpnState {
    /// Mapuje `ActiveState` systemd na stan VPN.
    pub fn from_active_state(active: &str) -> Self {
        match active {
            "activating" => Self::Connecting,
            "active" | "reloading" => Self::Connected,
            "deactivating" => Self::Disconnecting,
            "failed" | "maintenance" => Self::Failed,
            _ => Self::Inactive,
        }
    }
}

#[proxy(
    interface = "org.freedesktop.systemd1.Manager",
    default_service = "org.freedesktop.systemd1",
    default_path = "/org/freedesktop/systemd1"
)]
pub trait Manager {
    fn start_unit(&self, name: &str, mode: &str) -> zbus::Result<OwnedObjectPath>;
    fn stop_unit(&self, name: &str, mode: &str) -> zbus::Result<OwnedObjectPath>;
    fn load_unit(&self, name: &str) -> zbus::Result<OwnedObjectPath>;
    fn reset_failed_unit(&self, name: &str) -> zbus::Result<()>;
    fn subscribe(&self) -> zbus::Result<()>;
}

#[proxy(
    interface = "org.freedesktop.systemd1.Unit",
    default_service = "org.freedesktop.systemd1"
)]
pub trait Unit {
    #[zbus(property)]
    fn active_state(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn sub_state(&self) -> zbus::Result<String>;
    /// Moment wejscia w stan `active` (mikrosekundy od epoki Unix, 0 = nigdy).
    // Bez cache: odczyt w chwili sygnalu ActiveState nie moze trafic na jeszcze nieodswiezona wartosc.
    #[zbus(property(emits_changed_signal = "false"))]
    fn active_enter_timestamp(&self) -> zbus::Result<u64>;
}

#[proxy(
    interface = "org.freedesktop.systemd1.Service",
    default_service = "org.freedesktop.systemd1"
)]
pub trait Service {
    /// Wynik ostatniego uruchomienia: `success`, `exit-code`, `start-limit-hit`, `timeout`, ...
    #[zbus(property(emits_changed_signal = "false"))]
    fn result(&self) -> zbus::Result<String>;
}

/// Start/stop unitu. `reset_failed` przed startem czysci stan `failed` i licznik `StartLimitBurst`,
/// zeby ponowna proba po serii bledow nie byla odrzucana przez systemd.
pub async fn set_enabled(enable: bool, reset_failed: bool) -> Result<(), String> {
    async fn inner(enable: bool, reset_failed: bool) -> zbus::Result<()> {
        let conn = Connection::system().await?;
        let manager = ManagerProxy::new(&conn).await?;
        if enable && reset_failed {
            manager.reset_failed_unit(UNIT).await?;
        }
        if enable {
            manager.start_unit(UNIT, "replace").await?;
        } else {
            manager.stop_unit(UNIT, "replace").await?;
        }
        Ok(())
    }
    inner(enable, reset_failed).await.map_err(|e| e.to_string())
}

/// Otwiera polaczenie, subskrybuje zmiany unitu i zwraca proxy wraz z polaczeniem
/// (polaczenie musi zyc, bo `Subscribe` jest zwiazane z nim).
pub async fn watch() -> zbus::Result<(Connection, UnitProxy<'static>)> {
    let conn = Connection::system().await?;
    let manager = ManagerProxy::new(&conn).await?;
    manager.subscribe().await?;
    let path = manager.load_unit(UNIT).await?;
    let unit = UnitProxy::builder(&conn).path(path)?.build().await?;
    Ok((conn, unit))
}

pub async fn current_state(unit: &UnitProxy<'_>) -> zbus::Result<VpnState> {
    Ok(VpnState::from_active_state(&unit.active_state().await?))
}

/// Pole `Result` uslugi (powod porazki dla stanu `failed`).
pub async fn result(unit: &UnitProxy<'_>) -> zbus::Result<String> {
    let service = ServiceProxy::builder(unit.inner().connection())
        .path(unit.inner().path().to_owned())?
        .build()
        .await?;
    service.result().await
}

/// Od kiedy unit jest aktywny (do liczenia czasu polaczenia).
pub async fn connected_since(unit: &UnitProxy<'_>) -> zbus::Result<Option<SystemTime>> {
    let us = unit.active_enter_timestamp().await?;
    Ok((us > 0).then(|| UNIX_EPOCH + Duration::from_micros(us)))
}

#[cfg(test)]
mod tests {
    use super::VpnState::*;
    use super::*;

    #[test]
    fn maps_active_state() {
        assert_eq!(VpnState::from_active_state("inactive"), Inactive);
        assert_eq!(VpnState::from_active_state("activating"), Connecting);
        assert_eq!(VpnState::from_active_state("active"), Connected);
        assert_eq!(VpnState::from_active_state("deactivating"), Disconnecting);
        assert_eq!(VpnState::from_active_state("failed"), Failed);
        assert_eq!(VpnState::from_active_state("???"), Inactive);
    }
}
