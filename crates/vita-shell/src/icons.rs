use iced::Color;
use iced::widget::svg::{self, Handle, Svg};

pub fn vita_mark<'a>() -> Svg<'a> {
    let handle = Handle::from_memory(include_bytes!("../../../assets/icons/vita-mark.svg").as_slice());
    Svg::new(handle).width(20).height(20)
}

pub fn icon<'a>(bytes: &'static [u8], size: f32, color: Color) -> Svg<'a> {
    let handle = Handle::from_memory(bytes);
    Svg::new(handle)
        .width(size)
        .height(size)
        .style(move |_theme, _status| svg::Style { color: Some(color) })
}

pub const SEARCH: &[u8] = include_bytes!("../../../assets/icons/search.svg");
pub const GRID: &[u8] = include_bytes!("../../../assets/icons/grid.svg");
pub const CATEGORIES: &[u8] = include_bytes!("../../../assets/icons/categories.svg");
pub const WIFI: &[u8] = include_bytes!("../../../assets/icons/wifi.svg");
pub const WIFI_OFF: &[u8] = include_bytes!("../../../assets/icons/wifi-off.svg");
pub const BLUETOOTH: &[u8] = include_bytes!("../../../assets/icons/bluetooth.svg");
pub const VOLUME_HIGH: &[u8] = include_bytes!("../../../assets/icons/volume-high.svg");
pub const VOLUME_MUTE: &[u8] = include_bytes!("../../../assets/icons/volume-mute.svg");
pub const BATTERY: &[u8] = include_bytes!("../../../assets/icons/battery.svg");
pub const BATTERY_CHARGING: &[u8] = include_bytes!("../../../assets/icons/battery-charging.svg");
pub const BELL: &[u8] = include_bytes!("../../../assets/icons/bell.svg");
pub const SUN: &[u8] = include_bytes!("../../../assets/icons/sun.svg");
pub const MOON: &[u8] = include_bytes!("../../../assets/icons/moon.svg");
pub const LAYERS: &[u8] = include_bytes!("../../../assets/icons/layers.svg");
pub const CLOCK: &[u8] = include_bytes!("../../../assets/icons/clock.svg");
pub const POWER: &[u8] = include_bytes!("../../../assets/icons/power.svg");
pub const LOCK: &[u8] = include_bytes!("../../../assets/icons/lock.svg");
pub const SETTINGS: &[u8] = include_bytes!("../../../assets/icons/settings.svg");
pub const SLIDERS: &[u8] = include_bytes!("../../../assets/icons/sliders.svg");
pub const PIN: &[u8] = include_bytes!("../../../assets/icons/pin.svg");
pub const PIN_OFF: &[u8] = include_bytes!("../../../assets/icons/pin-off.svg");
pub const CLOSE: &[u8] = include_bytes!("../../../assets/icons/close.svg");
pub const MINUS: &[u8] = include_bytes!("../../../assets/icons/minus.svg");
pub const MAXIMIZE: &[u8] = include_bytes!("../../../assets/icons/maximize.svg");
pub const RESTORE: &[u8] = include_bytes!("../../../assets/icons/restore.svg");
pub const CHECK: &[u8] = include_bytes!("../../../assets/icons/check.svg");
pub const PLUS: &[u8] = include_bytes!("../../../assets/icons/plus.svg");
pub const WINDOW: &[u8] = include_bytes!("../../../assets/icons/window.svg");
pub const CHEVRON_LEFT: &[u8] = include_bytes!("../../../assets/icons/chevron-left.svg");
pub const CHEVRON_RIGHT: &[u8] = include_bytes!("../../../assets/icons/chevron-right.svg");
pub const APP_FALLBACK: &[u8] = include_bytes!("../../../assets/icons/app-fallback.svg");
