use crate::{
    diagnose::{Reason, diagnose},
    fl, icons,
    info::{self, Info, LOG_LINES_SHOWN, format_bytes, format_duration},
    notify::{self, ACTION_RETRY, Note, NotificationsProxy},
    systemd::{self, UnitStatus, VpnState},
    transition::{Event, Tracker},
};
use cosmic::{
    Element, Task, app,
    applet::{menu_button, padded_control},
    cosmic_theme::Spacing,
    iced::{
        Alignment, Background, Border, Color, Length, Subscription,
        futures::{SinkExt, StreamExt, channel::mpsc, stream::select},
        stream,
        widget::{stack, svg},
        window,
    },
    theme,
    widget::{Column, Row, Space, button, container, divider, icon, text, toggler},
};
use std::{
    any::TypeId,
    rc::Rc,
    time::{Duration, Instant, SystemTime},
};

pub fn run() -> cosmic::iced::Result {
    cosmic::applet::run::<FortiVpnApplet>(())
}

struct FortiVpnApplet {
    core: cosmic::app::Core,
    popup: Option<window::Id>,
    state: VpnState,
    /// Blad ostatniej operacji start/stop (np. odmowa polkit) lub bledu D-Bus.
    error: Option<String>,
    /// Blad polaczenia z D-Bus; znika po pierwszej udanej zmianie stanu.
    watch_error: Option<String>,
    info: Info,
    show_log: bool,
    /// Od kiedy unit jest aktywny (z `ActiveEnterTimestamp`).
    connected_since: Option<SystemTime>,
    /// Ostatni odczyt licznikow: (kiedy, rx, tx) - baza do liczenia predkosci.
    last_sample: Option<(Instant, u64, u64)>,
    /// Predkosc (rx, tx) w B/s.
    speed: Option<(u64, u64)>,
    /// Adres IP skopiowany do schowka (do czasu kolejnego ticku).
    copied: bool,
    /// Przyszla pierwsza informacja o stanie (bez niej nie ma z czym porownac - brak powiadomien).
    state_known: bool,
    /// Pole `Result` uslugi (powod stanu `failed`).
    unit_result: Option<String>,
    /// Ktore zmiany stanu powiadamiaja.
    tracker: Tracker,
    notifier: Option<NotificationsProxy<'static>>,
    /// Id ostatniego powiadomienia - kolejne je zastepuje.
    notify_id: u32,
    /// Proba sie nie udala i systemd ponowi ja za chwile (`SubState=auto-restart`, stan Connecting).
    retrying: bool,
    /// Aplet przerwal ponawianie po trwalym bledzie (zle haslo, certyfikat, config...). Unit jest
    /// wtedy `inactive`, ale pokazywany jako Failed - do kolejnego startu.
    aborted: bool,
    /// Faza migania kropki na ikonie panelu w stanach przejsciowych.
    blink: bool,
}

#[derive(Debug, Clone)]
enum Message {
    TogglePopup,
    CloseRequested(window::Id),
    Toggle(bool),
    ToggleLog,
    StateChanged(UnitStatus),
    ActionDone(Result<(), String>),
    WatchError(String),
    InfoLoaded(Info),
    LogsLoaded(Vec<String>),
    DelayedRefresh,
    Tick,
    Traffic(Option<(u64, u64)>),
    CopyIp,
    /// Swiezy log po nieudanej probie; `bool` - czy wyslac powiadomienie o bledzie.
    FailureLogs(Vec<String>, bool),
    Retry,
    NotifierReady(NotificationsProxy<'static>),
    /// Polaczenie z serwerem powiadomien zerwane (nasluch akcji juz nie dziala).
    NotifierLost,
    Notified(Result<Option<u32>, String>),
    NotificationAction(u32, String),
    Blink,
    /// Ktoras instancja otworzyla popup - zamknij widoczne powiadomienie.
    PopupShown,
    Noop,
}

impl FortiVpnApplet {
    fn refresh_info() -> app::Task<Message> {
        cosmic::task::future(async { Message::InfoLoaded(info::fetch().await) })
    }

    /// Zapomina probki ruchu (po zmianie stanu / zamknieciu popupu), zeby nie liczyc predkosci z przerwy.
    fn reset_sampling(&mut self) {
        self.last_sample = None;
        self.speed = None;
        self.copied = false;
    }

    /// Linie logu sa widoczne (wlaczone przez uzytkownika albo wymuszone przez stan Failed).
    fn log_visible(&self) -> bool {
        self.show_log || self.state == VpnState::Failed
    }

    /// Powod porazki (w stanie Failed albo przy ponawianiu).
    fn reason(&self) -> Option<Reason> {
        (self.state == VpnState::Failed || self.retrying)
            .then(|| diagnose(&self.info.logs, self.unit_result.as_deref()))
            .flatten()
    }

    /// Kolor stanu (kropka w popupie i na ikonie panelu); `None` dla Inactive.
    fn state_color(&self, cosmic: &cosmic::cosmic_theme::Theme) -> Option<Color> {
        match self.state {
            VpnState::Connected => Some(cosmic.success_color().into()),
            VpnState::Connecting | VpnState::Disconnecting => Some(cosmic.warning_color().into()),
            VpnState::Failed => Some(cosmic.destructive_color().into()),
            VpnState::Inactive => None,
        }
    }

    /// Start (przelacznik, *Sprobuj ponownie* w popupie i w powiadomieniu).
    fn start(&mut self) -> app::Task<Message> {
        self.error = None;
        self.aborted = false;
        let reset = self.state == VpnState::Failed;
        cosmic::task::future(
            async move { Message::ActionDone(systemd::set_enabled(true, reset).await) },
        )
    }

    fn stop() -> app::Task<Message> {
        cosmic::task::future(async {
            Message::ActionDone(systemd::set_enabled(false, false).await)
        })
    }

    /// Log czytany dopiero po zmianie stanu, wiec zawiera juz wpisy nieudanej proby.
    fn failure_logs(notify: bool) -> app::Task<Message> {
        cosmic::task::future(async move { Message::FailureLogs(info::logs().await, notify) })
    }

    /// Powiadomienie dla zdarzenia innego niz `Failure` (to wymaga logu - `failure_note`).
    fn event_note(event: Event) -> Note {
        let (icon, summary) = match event {
            Event::Connected => ("network-vpn-symbolic", fl!("notify-connected")),
            Event::Reconnecting => ("network-vpn-acquiring-symbolic", fl!("notify-reconnecting")),
            Event::Disconnected | Event::Failure => {
                ("network-vpn-disabled-symbolic", fl!("notify-disconnected"))
            }
        };
        Note {
            icon,
            summary,
            body: String::new(),
            actions: Vec::new(),
            critical: false,
        }
    }

    /// Powiadomienie o bledzie; przy ponawianiu bez przycisku (systemd i tak sprobuje ponownie).
    fn failure_note(&self) -> Note {
        let failed = self.state == VpnState::Failed;
        Note {
            icon: "network-vpn-no-route-symbolic",
            summary: if failed {
                fl!("notify-failed")
            } else {
                fl!("notify-retrying")
            },
            body: self.reason().map(Reason::text).unwrap_or_default(),
            actions: if failed {
                vec![(ACTION_RETRY, fl!("retry"))]
            } else {
                Vec::new()
            },
            critical: failed,
        }
    }

    fn notify(&self, note: Note) -> app::Task<Message> {
        let Some(proxy) = self.notifier.clone() else {
            return Task::none();
        };
        let replaces = self.notify_id;
        let app_name = fl!("title");
        cosmic::task::future(async move {
            Message::Notified(
                notify::send(
                    &proxy,
                    replaces,
                    <FortiVpnApplet as cosmic::Application>::APP_ID,
                    &app_name,
                    &note,
                )
                .await
                .map_err(|e| e.to_string()),
            )
        })
    }

    /// Zglasza (przez szyne sesji) otwarcie/zamkniecie popupu tej instancji.
    fn popup_changed(&self, open: bool) -> app::Task<Message> {
        let Some(proxy) = self.notifier.clone() else {
            return Task::none();
        };
        cosmic::task::future(async move {
            let app_id = <FortiVpnApplet as cosmic::Application>::APP_ID;
            if let Err(e) = notify::set_popup_open(&proxy, app_id, open).await {
                tracing::warn!("popup name: {e}");
            }
            Message::Noop
        })
    }

    fn icon(&self) -> &'static str {
        match self.state {
            VpnState::Connected => "network-vpn-symbolic",
            VpnState::Connecting | VpnState::Disconnecting => "network-vpn-acquiring-symbolic",
            VpnState::Failed => "network-vpn-no-route-symbolic",
            VpnState::Inactive => "network-vpn-disabled-symbolic",
        }
    }

    fn status_text(&self) -> String {
        let ip = self.info.ppp.as_ref().map(|p| p.ip.as_str());
        match (self.state, ip) {
            (VpnState::Connected, _) => fl!("connected"),
            (VpnState::Connecting, _) if self.retrying => fl!("retrying"),
            (VpnState::Connecting, _) => fl!("connecting"),
            (VpnState::Disconnecting, _) => fl!("disconnecting"),
            (VpnState::Failed, _) => fl!("failed"),
            (VpnState::Inactive, Some(ip)) => fl!("disabled-external", ip = ip),
            (VpnState::Inactive, None) => fl!("disabled"),
        }
    }
}

impl cosmic::Application for FortiVpnApplet {
    type Message = Message;
    type Executor = cosmic::SingleThreadExecutor;
    type Flags = ();
    const APP_ID: &'static str = "com.github.artur.CosmicAppletFortiVpn";

    fn init(core: cosmic::app::Core, _flags: Self::Flags) -> (Self, app::Task<Self::Message>) {
        (
            Self {
                core,
                popup: None,
                state: VpnState::Inactive,
                error: None,
                watch_error: None,
                info: Info::default(),
                show_log: false,
                connected_since: None,
                last_sample: None,
                speed: None,
                copied: false,
                state_known: false,
                unit_result: None,
                tracker: Tracker::default(),
                notifier: None,
                notify_id: 0,
                blink: false,
                retrying: false,
                aborted: false,
            },
            Self::refresh_info(),
        )
    }

    fn core(&self) -> &cosmic::app::Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut cosmic::app::Core {
        &mut self.core
    }

    fn update(&mut self, message: Self::Message) -> app::Task<Self::Message> {
        match message {
            Message::TogglePopup => {
                if let Some(p) = self.popup.take() {
                    self.reset_sampling();
                    return Task::batch([
                        cosmic::surface::surface_task(cosmic::surface::action::destroy_popup(p)),
                        self.popup_changed(false),
                    ]);
                }
                let open = cosmic::surface::surface_task(cosmic::surface::action::app_popup(
                    |_| Default::default(),
                    |app: &mut FortiVpnApplet| {
                        let new_id = window::Id::unique();
                        app.popup.replace(new_id);
                        app.core.applet.get_popup_settings(
                            app.core.main_window_id().unwrap(),
                            new_id,
                            Some((1, 1)),
                            None,
                            None,
                        )
                    },
                    None,
                ));
                return Task::batch([open, Self::refresh_info(), self.popup_changed(true)]);
            }
            Message::CloseRequested(id) => {
                if Some(id) == self.popup {
                    self.popup = None;
                    self.reset_sampling();
                    return self.popup_changed(false);
                }
            }
            Message::Toggle(enable) => {
                if self.state == VpnState::Disconnecting {
                    return Task::none();
                }
                if enable {
                    return self.start();
                }
                self.error = None;
                self.tracker.stop_requested();
                return Self::stop();
            }
            Message::Retry => return self.start(),
            Message::ToggleLog => self.show_log = !self.show_log,
            Message::StateChanged(status) => {
                self.watch_error = None;
                self.connected_since = status.since;
                self.unit_result = status.result;
                let mut state = status.state;
                // ponawianie przerwane przez aplet: unit jest inactive, ale to porazka
                if self.aborted && state == VpnState::Inactive {
                    state = VpnState::Failed;
                }
                let first = !std::mem::replace(&mut self.state_known, true);
                let retrying = status.retrying && state == VpnState::Connecting;
                // wejscie w auto-restart: nowy powod w logu, odswiez go
                let was_retrying = std::mem::replace(&mut self.retrying, retrying);
                let retry_started = retrying && !was_retrying;
                // sub_state potrafi zmieniac sie bez zmiany VpnState - nie odpalaj wtedy journalctl/ip
                if state == self.state && !first && !retry_started {
                    return Task::none();
                }
                let prev = std::mem::replace(&mut self.state, state);
                self.reset_sampling();
                if state != VpnState::Failed {
                    self.error = None;
                }
                let mut tasks = vec![Self::refresh_info()];
                // bez powiadomien o stanie zastanym przy starcie apletu
                let event = if first {
                    None
                } else {
                    self.tracker.transition(prev, state, retry_started)
                };
                // Failure czeka na swiezy log (z niego jest powod); kazde wejscie w auto-restart tez
                // czyta log - trwaly blad przerywa ponawianie (patrz FailureLogs)
                match event {
                    Some(Event::Failure) => tasks.push(Self::failure_logs(true)),
                    Some(e) => {
                        tasks.push(self.notify(Self::event_note(e)));
                        if retry_started {
                            tasks.push(Self::failure_logs(false));
                        }
                    }
                    None if retry_started => tasks.push(Self::failure_logs(false)),
                    None => {}
                }
                if state == VpnState::Connected {
                    // ppp0 / log moga pojawic sie chwile po READY=1 - odswiez jeszcze raz
                    tasks.push(cosmic::task::future(async {
                        tokio::time::sleep(Duration::from_secs(2)).await;
                        Message::DelayedRefresh
                    }));
                }
                return Task::batch(tasks);
            }
            Message::FailureLogs(logs, notify) => {
                self.info.logs = logs;
                if self.retrying && !self.aborted && self.reason().is_some_and(Reason::is_permanent)
                {
                    // kolejne proby nic nie dadza; powiadomienie przyjdzie z (mapowanym) stanem Failed
                    tracing::info!("permanent failure, stopping retries");
                    self.aborted = true;
                    return Self::stop();
                }
                if notify && (self.state == VpnState::Failed || self.retrying) {
                    return self.notify(self.failure_note());
                }
            }
            Message::ActionDone(res) => {
                if let Err(e) = res {
                    tracing::warn!("systemd action failed: {e}");
                    self.error = Some(e);
                    self.tracker.stop_failed();
                    self.aborted = false;
                }
            }
            Message::WatchError(e) => {
                tracing::warn!("watch error: {e}");
                self.watch_error = Some(e);
            }
            Message::InfoLoaded(info) => {
                // pierwsza probka dla liczenia predkosci (kolejne przychodza z Tick)
                self.last_sample = info.traffic.map(|(rx, tx)| (Instant::now(), rx, tx));
                self.info = info;
            }
            Message::LogsLoaded(logs) => self.info.logs = logs,
            // spozniony odczyt po Connected - tylko jesli tunel nadal dziala
            Message::DelayedRefresh => {
                if self.state == VpnState::Connected {
                    return Self::refresh_info();
                }
            }
            Message::Tick => {
                self.copied = false;
                let mut tasks = Vec::new();
                if self.state == VpnState::Connected
                    && let Some(p) = &self.info.ppp
                {
                    let ifname = p.ifname.clone();
                    tasks.push(cosmic::task::future(async move {
                        Message::Traffic(info::traffic(&ifname).await)
                    }));
                }
                if self.log_visible() {
                    tasks.push(cosmic::task::future(async {
                        Message::LogsLoaded(info::logs().await)
                    }));
                }
                return Task::batch(tasks);
            }
            Message::Traffic(Some((rx, tx))) => {
                let now = Instant::now();
                if let Some((t0, rx0, tx0)) = self.last_sample {
                    let dt = now.duration_since(t0).as_secs_f64();
                    if dt > 0.0 {
                        let rate =
                            |new: u64, old: u64| (new.saturating_sub(old) as f64 / dt) as u64;
                        self.speed = Some((rate(rx, rx0), rate(tx, tx0)));
                    }
                }
                self.last_sample = Some((now, rx, tx));
                self.info.traffic = Some((rx, tx));
            }
            Message::Traffic(None) => {}
            Message::CopyIp => {
                if let Some(p) = &self.info.ppp {
                    self.copied = true;
                    return cosmic::iced::clipboard::write(p.ip.clone());
                }
            }
            Message::NotifierReady(proxy) => {
                self.notifier = Some(proxy);
                // popup otwarty zanim polaczenie bylo gotowe - zglos go teraz
                if self.popup.is_some() {
                    return self.popup_changed(true);
                }
            }
            Message::NotifierLost => {
                self.notifier = None;
                self.notify_id = 0;
            }
            Message::Notified(Ok(Some(id))) => self.notify_id = id,
            Message::Notified(Ok(None)) => {}
            Message::Notified(Err(e)) => tracing::warn!("notification failed: {e}"),
            Message::NotificationAction(id, key) => {
                if id == self.notify_id && key == ACTION_RETRY && self.state == VpnState::Failed {
                    return self.start();
                }
            }
            Message::Blink => self.blink = !self.blink,
            Message::PopupShown => {
                let id = std::mem::take(&mut self.notify_id);
                if let (Some(proxy), true) = (self.notifier.clone(), id != 0) {
                    return cosmic::task::future(async move {
                        let _ = proxy.close_notification(id).await;
                        Message::Noop
                    });
                }
            }
            Message::Noop => {}
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        let applet = &self.core.applet;
        let (w, h) = applet.suggested_size(true);
        // jak `icon_button`, ale z kropka stanu w prawym dolnym rogu
        let glyph = icon::icon(icon::from_name(self.icon()).symbolic(true).size(w).into())
            .class(theme::Svg::Custom(Rc::new(|t| svg::Style {
                color: Some(t.cosmic().background(t.transparent).on.into()),
            })))
            .width(Length::Fixed(w.into()))
            .height(Length::Fixed(h.into()));
        let mut layers: Vec<Element<'_, Message>> = vec![glyph.into()];
        if let Some(color) = self.state_color(theme::active().cosmic()) {
            let transitional = matches!(self.state, VpnState::Connecting | VpnState::Disconnecting);
            let a = if transitional && self.blink { 0.3 } else { 1.0 };
            let size = (f32::from(w) * 0.4).round().max(5.0);
            layers.push(
                container(state_dot(Color { a, ..color }, size))
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .align_right(Length::Fill)
                    .align_bottom(Length::Fill)
                    .into(),
            );
        }
        let content = stack(layers)
            .width(Length::Fixed(w.into()))
            .height(Length::Fixed(h.into()));
        applet
            .button_from_element(content, true)
            .on_press_down(Message::TogglePopup)
            .into()
    }

    fn view_window(&self, _id: window::Id) -> Element<'_, Message> {
        let cosmic = theme::active().cosmic().clone();
        let Spacing {
            space_xxs,
            space_xs,
            space_s,
            ..
        } = cosmic.spacing;

        // Drugorzedny tekst: kolor tekstu motywu z mniejszym kontrastem.
        let on_bg: Color = cosmic.on_bg_color().into();
        let dim = theme::Text::Color(Color { a: 0.65, ..on_bg });
        let state_color = self
            .state_color(&cosmic)
            .unwrap_or(Color { a: 0.45, ..on_bg });

        let on = matches!(self.state, VpnState::Connecting | VpnState::Connected);
        let toggle = padded_control(
            toggler(on)
                .on_toggle(Message::Toggle)
                .text_size(14)
                .width(Length::Fill)
                .label(fl!("title")),
        );

        // Wiersz stanu: kropka w kolorze stanu, nazwa stanu, czas polaczenia po prawej.
        let dot = state_dot(state_color, 8.0);
        let mut status_row = Row::new()
            .spacing(space_xs)
            .align_y(Alignment::Center)
            .push(dot)
            .push(text::body(self.status_text()).width(Length::Fill));
        let connected = self.state == VpnState::Connected;
        if connected
            && let Some(since) = self.connected_since
            && let Ok(elapsed) = since.elapsed()
        {
            status_row = status_row.push(text::body(format_duration(elapsed)).class(dim));
        }

        let mut content = Column::new().push(toggle).push(padded_control(status_row));

        if self.state == VpnState::Failed || self.retrying {
            let mut failed = Column::new().spacing(space_xs);
            if let Some(reason) = self.reason() {
                failed = failed.push(text::body(reason.text()));
            }
            // przy ponawianiu systemd sam sprobuje ponownie - bez przycisku
            if self.state == VpnState::Failed {
                failed = failed.push(
                    Row::new()
                        .push(Space::new().width(Length::Fill))
                        .push(button::standard(fl!("retry")).on_press(Message::Retry)),
                );
            }
            content = content.push(padded_control(failed));
        }

        if connected {
            if let Some(p) = &self.info.ppp {
                // adres tunelu jako glowna informacja; przycisk obok kopiuje go do schowka
                let copy_icon = if self.copied {
                    icons::CHECK
                } else {
                    icons::COPY
                };
                content = content.push(padded_control(
                    Row::new()
                        .align_y(Alignment::Center)
                        .push(text::title4(p.ip.clone()).width(Length::Fill))
                        .push(
                            button::icon(icons::handle(copy_icon))
                                .icon_size(20)
                                .on_press(Message::CopyIp),
                        ),
                ));
            }
            if let Some((rx, tx)) = self.info.traffic {
                let rate = |r: Option<u64>| match r {
                    Some(r) => fl!("rate", value = format_bytes(r)),
                    None => fl!("rate-unknown"),
                };
                let tile = |glyph: &'static str, label: String, total: u64, r: Option<u64>| {
                    let head = Row::new()
                        .spacing(space_xxs)
                        .align_y(Alignment::Center)
                        .push(icon::icon(icons::handle(glyph)).size(16))
                        .push(text::body(label).class(dim));
                    container(
                        Column::new()
                            .spacing(2)
                            .push(head)
                            .push(text::title4(format_bytes(total)))
                            .push(text::body(rate(r)).class(dim)),
                    )
                    .padding(space_xs)
                    .width(Length::FillPortion(1))
                    .class(theme::Container::Card)
                };
                content = content.push(padded_control(
                    Row::new()
                        .spacing(space_xs)
                        .push(tile(
                            icons::DOWNLOAD,
                            fl!("received"),
                            rx,
                            self.speed.map(|s| s.0),
                        ))
                        .push(tile(
                            icons::UPLOAD,
                            fl!("sent"),
                            tx,
                            self.speed.map(|s| s.1),
                        )),
                ));
            }
        }

        if let Some(err) = self.error.as_ref().or(self.watch_error.as_ref()) {
            content = content.push(padded_control(
                text::body(fl!("error", error = err.as_str()))
                    .class(theme::Text::Color(cosmic.destructive_text_color().into())),
            ));
        }

        if self.log_visible() {
            content = content
                .push(padded_control(divider::horizontal::default()).padding([space_xxs, space_s]));
            let logs = &self.info.logs;
            for line in &logs[logs.len().saturating_sub(LOG_LINES_SHOWN)..] {
                content = content.push(padded_control(text::caption(line.clone()).class(dim)));
            }
        }

        content = content
            .push(padded_control(divider::horizontal::default()).padding([space_xxs, space_s]))
            .push(
                menu_button(text::body(if self.show_log {
                    fl!("hide-log")
                } else {
                    fl!("show-log")
                }))
                .on_press(Message::ToggleLog),
            )
            .width(Length::Fixed(320.0))
            .padding([8, 0]);

        self.core.applet.popup_container(content).into()
    }

    fn subscription(&self) -> Subscription<Message> {
        // Odswiezanie (liczniki, log) tylko przy otwartym popupie i tylko gdy jest co odswiezac.
        let tick =
            if self.popup.is_some() && (self.state == VpnState::Connected || self.log_visible()) {
                cosmic::iced::time::every(Duration::from_secs(2)).map(|_| Message::Tick)
            } else {
                Subscription::none()
            };
        // Miganie kropki na ikonie panelu tylko w stanach przejsciowych.
        let blink = if matches!(self.state, VpnState::Connecting | VpnState::Disconnecting) {
            cosmic::iced::time::every(Duration::from_millis(600)).map(|_| Message::Blink)
        } else {
            Subscription::none()
        };
        Subscription::batch([state_subscription(), notifier_subscription(), tick, blink])
    }

    fn on_close_requested(&self, id: window::Id) -> Option<Message> {
        Some(Message::CloseRequested(id))
    }

    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        Some(cosmic::applet::style())
    }
}

/// Okragla kropka w kolorze stanu.
fn state_dot<'a>(color: Color, size: f32) -> Element<'a, Message> {
    container(
        Space::new()
            .width(Length::Fixed(size))
            .height(Length::Fixed(size)),
    )
    .class(theme::Container::custom(move |_| container::Style {
        background: Some(Background::Color(color)),
        border: Border {
            radius: (size / 2.0).into(),
            ..Default::default()
        },
        ..Default::default()
    }))
    .into()
}

/// Polaczenie z serwerem powiadomien i nasluch klikniec w ich akcje; po bledzie ponawia po 30 s.
fn notifier_subscription() -> Subscription<Message> {
    struct Notifier;
    Subscription::run_with(TypeId::of::<Notifier>(), |_| {
        stream::channel(10, |mut output: mpsc::Sender<Message>| async move {
            loop {
                if let Err(e) = listen_notifications(&mut output).await {
                    tracing::warn!("notifications unavailable: {e}");
                }
                // porzuc stare polaczenie (zwalnia nazwy na szynie), zanim powstanie nowe
                if output.send(Message::NotifierLost).await.is_err() {
                    break;
                }
                tokio::time::sleep(Duration::from_secs(30)).await;
            }
        })
    })
}

async fn listen_notifications(output: &mut mpsc::Sender<Message>) -> zbus::Result<()> {
    let app_id = <FortiVpnApplet as cosmic::Application>::APP_ID;
    let proxy = notify::connect(app_id).await?;
    // niepoprawny sygnal pomijamy - nie moze zerwac nasluchu
    let actions = proxy
        .receive_action_invoked()
        .await?
        .filter_map(|signal| async move {
            let a = signal.args().ok()?;
            Some(Message::NotificationAction(a.id, a.action_key.to_owned()))
        });
    // popup otwarty na ktorymkolwiek monitorze (nowy wlasciciel nazwy)
    let popup = notify::popup_name(app_id)?;
    let shown = zbus::fdo::DBusProxy::new(proxy.inner().connection())
        .await?
        .receive_name_owner_changed_with_args(&[(0, popup.as_str())])
        .await?
        .filter_map(|signal| async move {
            let args = signal.args().ok()?;
            args.new_owner().is_some().then_some(Message::PopupShown)
        });
    if output.send(Message::NotifierReady(proxy)).await.is_err() {
        return Ok(());
    }
    let mut events = select(Box::pin(actions), Box::pin(shown));
    while let Some(msg) = events.next().await {
        if output.send(msg).await.is_err() {
            break;
        }
    }
    Ok(())
}

/// Obserwuje `ActiveState`/`SubState` unitu przez D-Bus; po bledzie ponawia po 5 s.
fn state_subscription() -> Subscription<Message> {
    Subscription::run_with(TypeId::of::<VpnState>(), |_| {
        stream::channel(10, |mut output: mpsc::Sender<Message>| async move {
            loop {
                match watch(&mut output).await {
                    Err(e) => {
                        let _ = output.send(Message::WatchError(e.to_string())).await;
                    }
                    Ok(()) => tracing::warn!("systemd signal stream ended, reconnecting"),
                }
                if output.is_closed() {
                    break;
                }
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        })
    })
}

async fn watch(output: &mut mpsc::Sender<Message>) -> zbus::Result<()> {
    // `_conn` musi zyc: Subscribe() systemd jest zwiazane z polaczeniem.
    let (_conn, unit) = systemd::watch().await?;
    let mut changes = select(
        unit.receive_active_state_changed().await.map(|_| ()),
        unit.receive_sub_state_changed().await.map(|_| ()),
    );

    loop {
        let status = systemd::status(&unit).await?;
        if output.send(Message::StateChanged(status)).await.is_err() {
            return Ok(()); // odbiorca zamkniety - koniec subskrypcji
        }
        if changes.next().await.is_none() {
            return Ok(());
        }
    }
}
