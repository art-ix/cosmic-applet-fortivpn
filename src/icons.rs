//! Wlasne ikony (SVG w `assets/icons/`, siatka 24x24, kreska 2 px) osadzone w binarce.
//! Sa "symbolic": libcosmic koloruje je kolorem tekstu motywu (jasny/ciemny).

use cosmic::widget::icon::{self, Handle};

pub const DOWNLOAD: &str = include_str!("../assets/icons/download.svg");
pub const UPLOAD: &str = include_str!("../assets/icons/upload.svg");
pub const COPY: &str = include_str!("../assets/icons/copy.svg");
pub const CHECK: &str = include_str!("../assets/icons/check.svg");

pub fn handle(svg: &'static str) -> Handle {
    let mut h = icon::from_svg_bytes(svg.as_bytes());
    h.symbolic = true;
    h
}
