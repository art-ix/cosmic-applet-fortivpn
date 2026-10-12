mod app;
mod diagnose;
mod icons;
mod info;
mod localize;
mod notify;
mod systemd;

fn main() -> cosmic::iced::Result {
    tracing_subscriber::fmt::init();
    tracing::info!("Starting FortiVPN applet {}", env!("CARGO_PKG_VERSION"));
    localize::localize();
    app::run()
}
