//! Powiadomienia pulpitu przez `org.freedesktop.Notifications` (szyna sesji; w COSMIC obsluguje je
//! `cosmic-notifications`).

use std::collections::HashMap;
use zbus::{Connection, fdo::DBusProxy, names::WellKnownName, proxy, zvariant::Value};

/// Klucz akcji "Polacz ponownie" w powiadomieniu o bledzie.
pub const ACTION_RETRY: &str = "retry";

#[proxy(
    interface = "org.freedesktop.Notifications",
    default_service = "org.freedesktop.Notifications",
    default_path = "/org/freedesktop/Notifications"
)]
pub trait Notifications {
    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app_name: &str,
        replaces_id: u32,
        app_icon: &str,
        summary: &str,
        body: &str,
        actions: &[&str],
        hints: HashMap<&str, Value<'_>>,
        expire_timeout: i32,
    ) -> zbus::Result<u32>;

    fn close_notification(&self, id: u32) -> zbus::Result<()>;

    #[zbus(signal)]
    fn action_invoked(&self, id: u32, action_key: &str) -> zbus::Result<()>;
}

/// Tresc jednego powiadomienia.
#[derive(Debug, Clone)]
pub struct Note {
    pub icon: &'static str,
    pub summary: String,
    pub body: String,
    /// Pary (klucz, etykieta).
    pub actions: Vec<(&'static str, String)>,
    pub critical: bool,
}

/// Nazwa na szynie sesji, o ktora ustawiaja sie w kolejce wszystkie instancje apletu.
/// cosmic-panel uruchamia osobny proces na kazdy monitor; powiadamia tylko wlasciciel nazwy,
/// a po jego zamknieciu nazwe przejmuje nastepny w kolejce.
fn leader_name(app_id: &str) -> zbus::Result<WellKnownName<'static>> {
    Ok(WellKnownName::try_from(format!("{app_id}.Notifier"))?)
}

/// Nazwe trzyma kazda instancja z otwartym popupem (na dowolnym monitorze). Popup pokazuje to samo
/// co powiadomienie, a powiadomienie by go zaslanialo - wiec wtedy powiadomien nie ma.
pub fn popup_name(app_id: &str) -> zbus::Result<WellKnownName<'static>> {
    Ok(WellKnownName::try_from(format!("{app_id}.PopupOpen"))?)
}

/// Zglasza otwarcie/zamkniecie popupu tej instancji.
pub async fn set_popup_open(
    proxy: &NotificationsProxy<'static>,
    app_id: &str,
    open: bool,
) -> zbus::Result<()> {
    let conn = proxy.inner().connection();
    if open {
        conn.request_name_with_flags(popup_name(app_id)?, Default::default())
            .await?;
    } else {
        conn.release_name(popup_name(app_id)?).await?;
    }
    Ok(())
}

/// Polaczenie z szyna sesji; sygnaly akcji przychodza na to samo polaczenie, ktore wyslalo `Notify`.
pub async fn connect(app_id: &str) -> zbus::Result<NotificationsProxy<'static>> {
    let conn = Connection::session().await?;
    // puste flagi (bez DoNotQueue): druga instancja czeka w kolejce zamiast dostac blad
    conn.request_name_with_flags(leader_name(app_id)?, Default::default())
        .await?;
    NotificationsProxy::new(&conn).await
}

/// Wysyla powiadomienie, zastepujac poprzednie (`replaces_id`), zeby sie nie mnozyly.
/// `None`: nic nie wyslano - ta instancja nie jest wlascicielem nazwy (powiadamia inna) albo
/// ktoras instancja ma otwarty popup.
pub async fn send(
    proxy: &NotificationsProxy<'static>,
    replaces_id: u32,
    app_id: &str,
    note: &Note,
) -> zbus::Result<Option<u32>> {
    let conn = proxy.inner().connection();
    let dbus = DBusProxy::new(conn).await?;
    let owner = dbus.get_name_owner(leader_name(app_id)?.into()).await?;
    if conn.unique_name() != Some(&owner) || dbus.name_has_owner(popup_name(app_id)?.into()).await?
    {
        return Ok(None);
    }
    let actions: Vec<&str> = note
        .actions
        .iter()
        .flat_map(|(k, l)| [*k, l.as_str()])
        .collect();
    let hints = HashMap::from([
        ("desktop-entry", Value::from(app_id)),
        ("urgency", Value::U8(if note.critical { 2 } else { 1 })),
    ]);
    proxy
        .notify(
            "FortiVPN",
            replaces_id,
            note.icon,
            &note.summary,
            &note.body,
            &actions,
            hints,
            -1,
        )
        .await
        .map(Some)
}
