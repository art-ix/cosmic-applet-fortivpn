use crate::{
    fl, icons,
    info::{self, Info, format_bytes, format_duration},
    systemd::{self, VpnState},
};
use cosmic::{
    Element, Task, app,
    applet::{menu_button, padded_control},
    cosmic_theme::Spacing,
    iced::{
        Alignment, Background, Border, Color, Length, Subscription,
        futures::{SinkExt, StreamExt, channel::mpsc, stream::select},
        stream, window,
    },
    theme,
    widget::{Column, Row, Space, button, container, divider, icon, text, toggler},
};
use std::{
    any::TypeId,
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
}

#[derive(Debug, Clone)]
enum Message {
    TogglePopup,
    CloseRequested(window::Id),
    Toggle(bool),
    ToggleLog,
    StateChanged(VpnState),
    ActionDone(Result<(), String>),
    WatchError(String),
    InfoLoaded(Info),
    LogsLoaded(Vec<String>),
    DelayedRefresh,
    ConnectedSince(Option<SystemTime>),
    Tick,
    Traffic(Option<(u64, u64)>),
    CopyIp,
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
                    return cosmic::surface::surface_task(cosmic::surface::action::destroy_popup(
                        p,
                    ));
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
                return Task::batch([open, Self::refresh_info()]);
            }
            Message::CloseRequested(id) => {
                if Some(id) == self.popup {
                    self.popup = None;
                    self.reset_sampling();
                }
            }
            Message::Toggle(enable) => {
                if self.state == VpnState::Disconnecting {
                    return Task::none();
                }
                self.error = None;
                return cosmic::task::future(async move {
                    Message::ActionDone(systemd::set_enabled(enable).await)
                });
            }
            Message::ToggleLog => self.show_log = !self.show_log,
            Message::StateChanged(state) => {
                self.watch_error = None;
                // sub_state potrafi zmieniac sie bez zmiany VpnState - nie odpalaj wtedy journalctl/ip
                if state == self.state {
                    return Task::none();
                }
                self.state = state;
                self.reset_sampling();
                if state != VpnState::Failed {
                    self.error = None;
                }
                if state == VpnState::Connected {
                    // ppp0 / log moga pojawic sie chwile po READY=1 - odswiez jeszcze raz
                    return Task::batch([
                        Self::refresh_info(),
                        cosmic::task::future(async {
                            tokio::time::sleep(Duration::from_secs(2)).await;
                            Message::DelayedRefresh
                        }),
                    ]);
                }
                return Self::refresh_info();
            }
            Message::ActionDone(res) => {
                if let Err(e) = res {
                    tracing::warn!("systemd action failed: {e}");
                    self.error = Some(e);
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
            Message::ConnectedSince(t) => self.connected_since = t,
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
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        self.core
            .applet
            .icon_button(self.icon())
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
        let state_color: Color = match self.state {
            VpnState::Connected => cosmic.success_color().into(),
            VpnState::Connecting | VpnState::Disconnecting => cosmic.warning_color().into(),
            VpnState::Failed => cosmic.destructive_color().into(),
            VpnState::Inactive => Color { a: 0.45, ..on_bg },
        };

        let on = matches!(self.state, VpnState::Connecting | VpnState::Connected);
        let toggle = padded_control(
            toggler(on)
                .on_toggle(Message::Toggle)
                .text_size(14)
                .width(Length::Fill)
                .label(fl!("title")),
        );

        // Wiersz stanu: kropka w kolorze stanu, nazwa stanu, czas polaczenia po prawej.
        let dot = container(
            Space::new()
                .width(Length::Fixed(8.0))
                .height(Length::Fixed(8.0)),
        )
        .class(theme::Container::custom(move |_| container::Style {
            background: Some(Background::Color(state_color)),
            border: Border {
                radius: 4.0.into(),
                ..Default::default()
            },
            ..Default::default()
        }));
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
            for line in &self.info.logs {
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
        Subscription::batch([state_subscription(), tick])
    }

    fn on_close_requested(&self, id: window::Id) -> Option<Message> {
        Some(Message::CloseRequested(id))
    }

    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        Some(cosmic::applet::style())
    }
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
        let state = systemd::current_state(&unit).await?;
        let since = systemd::connected_since(&unit).await.ok().flatten();
        if output.send(Message::ConnectedSince(since)).await.is_err()
            || output.send(Message::StateChanged(state)).await.is_err()
        {
            return Ok(()); // odbiorca zamkniety - koniec subskrypcji
        }
        if changes.next().await.is_none() {
            return Ok(());
        }
    }
}
