use crate::icons;
use crate::message::Message;
use crate::style::{
    FONT_MEDIUM, FONT_REGULAR, FONT_SEMIBOLD, color, menu_row_button, popover_card_surface,
    quick_tile_button, subtle_pill_button, surface_card_button, transparent_container,
    warm_accent_slider_style,
};
use iced::border::Radius;
use iced::widget::{Space, button, column, container, row, slider, space, text};
use iced::{Alignment, Background, Border, Color, Element, Length, Padding};
use vita_core::{SystemSummary, ThemeTokens};

pub const QUICK_CONTROLS_WIDTH: f32 = 382.0;
const SHADOW_GUTTER: f32 = 18.0;

pub fn estimate_height(summary: &SystemSummary, power_confirm_open: bool) -> f32 {
    let mut h = 342.0;
    if summary.brightness_pct.is_some() {
        h += 54.0;
    }
    if summary.battery_pct.is_some() {
        h += 48.0;
    }
    if power_confirm_open {
        h += 48.0;
    }
    h + SHADOW_GUTTER * 2.0
}

fn truncate(s: &str, max_chars: usize) -> String {
    let mut chars = s.chars();
    let head: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{}…", head.trim_end())
    } else {
        head
    }
}

pub fn view<'a>(
    tokens: &'a ThemeTokens,
    summary: &'a SystemSummary,
    focus_mode: bool,
    power_confirm_open: bool,
) -> Element<'a, Message> {
    let primary_fg = color(tokens.text_primary);
    let secondary_fg = color(tokens.text_secondary);
    let accent_fg = color(tokens.accent_primary);

    // Header
    let header = row![
        text("Quick controls")
            .font(FONT_SEMIBOLD)
            .size(15)
            .color(primary_fg),
        space::horizontal(),
        button(icons::icon(icons::SETTINGS, 15.0, secondary_fg))
            .padding(6)
            .style(subtle_pill_button(tokens, false, tokens.pill))
            .on_press(Message::OpenSettings),
    ]
    .align_y(Alignment::Center)
    .width(Length::Fill);

    // Connectivity 2-up tiles
    let wifi_sub = if summary.wifi_connected {
        summary
            .wifi_ssid
            .as_deref()
            .map(|s| truncate(s, 15))
            .unwrap_or_else(|| "Connected".to_string())
    } else {
        "Disconnected".to_string()
    };
    let wifi_icon = if summary.wifi_connected {
        icons::WIFI
    } else {
        icons::WIFI_OFF
    };
    let wifi_tile = connectivity_tile(
        tokens,
        wifi_icon,
        "Wi-Fi",
        &wifi_sub,
        summary.wifi_connected,
        Message::OpenTargetAndDismiss("ms-settings:network-wifi".to_string()),
    );

    let (bt_active, bt_sub) = match summary.bluetooth_on {
        Some(true) => (true, "On"),
        Some(false) => (false, "Off"),
        None => (false, "Unavailable"),
    };
    let bt_tile = connectivity_tile(
        tokens,
        icons::BLUETOOTH,
        "Bluetooth",
        bt_sub,
        bt_active,
        Message::OpenTargetAndDismiss("ms-settings:bluetooth".to_string()),
    );

    let tiles_row = row![wifi_tile, bt_tile].spacing(10).width(Length::Fill);

    // Sliders card (Brightness + Volume)
    let mut sliders_col = column![].spacing(14).width(Length::Fill);

    if let Some(brightness) = summary.brightness_pct {
        sliders_col = sliders_col.push(
            column![
                row![
                    icons::icon(icons::SUN, 15.0, primary_fg),
                    text("Brightness")
                        .font(FONT_MEDIUM)
                        .size(12)
                        .color(primary_fg),
                    space::horizontal(),
                    text(format!("{brightness}%"))
                        .font(FONT_MEDIUM)
                        .size(12)
                        .color(secondary_fg),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
                slider(0..=100, brightness, Message::BrightnessChanged)
                    .step(1u8)
                    .width(Length::Fill)
                    .style(warm_accent_slider_style(tokens)),
            ]
            .spacing(6),
        );
    }

    let vol_icon = if summary.is_muted || summary.volume_pct == 0 {
        icons::VOLUME_MUTE
    } else {
        icons::VOLUME_HIGH
    };
    let vol_label = if summary.is_muted {
        "Muted".to_string()
    } else {
        format!("{}%", summary.volume_pct)
    };
    let vol_value = if summary.is_muted {
        0
    } else {
        summary.volume_pct
    };

    sliders_col = sliders_col.push(
        column![
            row![
                button(icons::icon(vol_icon, 15.0, primary_fg))
                    .padding([2, 3])
                    .style(subtle_pill_button(tokens, summary.is_muted, tokens.r8))
                    .on_press(Message::ToggleMute),
                text("Volume").font(FONT_MEDIUM).size(12).color(primary_fg),
                space::horizontal(),
                text(vol_label)
                    .font(FONT_MEDIUM)
                    .size(12)
                    .color(secondary_fg),
            ]
            .spacing(6)
            .align_y(Alignment::Center),
            slider(0..=100, vol_value, Message::VolumeChanged)
                .step(1u8)
                .width(Length::Fill)
                .style(warm_accent_slider_style(tokens)),
        ]
        .spacing(6),
    );

    let sliders_card = container(sliders_col)
        .width(Length::Fill)
        .padding(14)
        .style({
            let tokens = *tokens;
            move |_| container::Style {
                background: Some(Background::Color(color(tokens.surface_elevated))),
                border: Border {
                    color: color(tokens.border_control),
                    width: 1.0,
                    radius: Radius::from(tokens.r16),
                },
                snap: true,
                ..container::Style::default()
            }
        });

    // Focus Mode card with clean pill switch
    let focus_card = container(
        row![
            icons::icon(
                icons::MOON,
                16.0,
                if focus_mode { accent_fg } else { primary_fg }
            ),
            column![
                text("Focus mode")
                    .font(FONT_SEMIBOLD)
                    .size(13)
                    .color(primary_fg),
                text("Keep the little distractions away.")
                    .font(FONT_REGULAR)
                    .size(11)
                    .color(secondary_fg),
            ]
            .spacing(1),
            space::horizontal(),
            pill_switch(tokens, focus_mode, Message::ToggleFocusMode),
        ]
        .spacing(10)
        .align_y(Alignment::Center),
    )
    .width(Length::Fill)
    .padding([11, 14])
    .style({
        let tokens = *tokens;
        move |_| container::Style {
            background: Some(Background::Color(color(tokens.surface_elevated))),
            border: Border {
                color: color(tokens.border_control),
                width: 1.0,
                radius: Radius::from(tokens.r16),
            },
            snap: true,
            ..container::Style::default()
        }
    });

    let mut content = column![header, tiles_row, sliders_card, focus_card]
        .spacing(10)
        .width(Length::Fill);

    // Optional Battery pill
    if let Some(batt_text) = summary.battery_status_text() {
        let batt_icon = if summary.is_charging {
            icons::BATTERY_CHARGING
        } else {
            icons::BATTERY
        };
        content = content.push(
            container(
                row![
                    icons::icon(batt_icon, 15.0, primary_fg),
                    text(batt_text).font(FONT_MEDIUM).size(12).color(primary_fg),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            )
            .width(Length::Fill)
            .padding([9, 12])
            .style({
                let tokens = *tokens;
                move |_| container::Style {
                    background: Some(Background::Color(color(tokens.surface_subtle))),
                    border: Border {
                        color: color(tokens.border_subtle),
                        width: 1.0,
                        radius: Radius::from(tokens.r12),
                    },
                    snap: true,
                    ..container::Style::default()
                }
            }),
        );
    }

    // Lock & Power action buttons
    let actions_row = row![
        button(
            container(
                row![
                    icons::icon(icons::LOCK, 14.0, primary_fg),
                    text("Lock").font(FONT_MEDIUM).size(12).color(primary_fg),
                ]
                .spacing(7)
                .align_y(Alignment::Center),
            )
            .center_x(Length::Fill)
            .center_y(Length::Fill),
        )
        .width(Length::FillPortion(1))
        .height(38)
        .padding([0, 12])
        .style(surface_card_button(tokens, false, tokens.r12))
        .on_press(Message::PowerLock),
        button(
            container(
                row![
                    icons::icon(icons::POWER, 14.0, primary_fg),
                    text("Power").font(FONT_MEDIUM).size(12).color(primary_fg),
                ]
                .spacing(7)
                .align_y(Alignment::Center),
            )
            .center_x(Length::Fill)
            .center_y(Length::Fill),
        )
        .width(Length::FillPortion(1))
        .height(38)
        .padding([0, 12])
        .style(surface_card_button(tokens, power_confirm_open, tokens.r12))
        .on_press(Message::TogglePowerConfirm),
    ]
    .spacing(10)
    .width(Length::Fill);

    content = content.push(actions_row);

    if power_confirm_open {
        let danger_fg = color(tokens.status_error);
        let confirm_strip = container(
            row![
                button(
                    container(text("Sleep").font(FONT_MEDIUM).size(11).color(primary_fg))
                        .center_x(Length::Fill)
                )
                .width(Length::FillPortion(1))
                .padding([6, 6])
                .style(menu_row_button(tokens, false))
                .on_press(Message::PowerSleep),
                button(
                    container(text("Restart").font(FONT_MEDIUM).size(11).color(primary_fg))
                        .center_x(Length::Fill)
                )
                .width(Length::FillPortion(1))
                .padding([6, 6])
                .style(menu_row_button(tokens, false))
                .on_press(Message::PowerRestart),
                button(
                    container(
                        text("Shut down")
                            .font(FONT_MEDIUM)
                            .size(11)
                            .color(danger_fg)
                    )
                    .center_x(Length::Fill)
                )
                .width(Length::FillPortion(1))
                .padding([6, 6])
                .style(menu_row_button(tokens, true))
                .on_press(Message::PowerShutdown),
                button(
                    container(
                        text("Exit Vita")
                            .font(FONT_MEDIUM)
                            .size(11)
                            .color(secondary_fg)
                    )
                    .center_x(Length::Fill)
                )
                .width(Length::FillPortion(1))
                .padding([6, 6])
                .style(menu_row_button(tokens, false))
                .on_press(Message::ExitRequested),
            ]
            .spacing(4)
            .align_y(Alignment::Center),
        )
        .width(Length::Fill)
        .padding(4)
        .style({
            let tokens = *tokens;
            move |_| container::Style {
                background: Some(Background::Color(color(tokens.surface_elevated))),
                border: Border {
                    color: color(tokens.border_control),
                    width: 1.0,
                    radius: Radius::from(tokens.r12),
                },
                snap: true,
                ..container::Style::default()
            }
        });
        content = content.push(confirm_strip);
    }

    let card = container(content)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(Padding {
            top: 16.0,
            right: 16.0,
            bottom: 16.0,
            left: 16.0,
        })
        .style({
            let tokens = *tokens;
            move |_| popover_card_surface(&tokens, tokens.r24)
        });

    container(card)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(SHADOW_GUTTER)
        .style(|_| transparent_container())
        .into()
}

fn connectivity_tile<'a>(
    tokens: &'a ThemeTokens,
    icon_bytes: &'static [u8],
    title: &'a str,
    subtitle: &str,
    active: bool,
    on_press: Message,
) -> Element<'a, Message> {
    let primary_fg = color(tokens.text_primary);
    let secondary_fg = color(tokens.text_secondary);
    let badge_fg = if active {
        color(tokens.text_on_accent)
    } else {
        primary_fg
    };
    let badge_bg = if active {
        color(tokens.accent_primary)
    } else {
        color(tokens.surface_subtle)
    };

    button(
        row![
            container(icons::icon(icon_bytes, 16.0, badge_fg))
                .width(34)
                .height(34)
                .center_x(34)
                .center_y(34)
                .style(move |_| container::Style {
                    background: Some(Background::Color(badge_bg)),
                    border: Border {
                        radius: Radius::from(10.0),
                        ..Border::default()
                    },
                    ..container::Style::default()
                }),
            column![
                text(title).font(FONT_SEMIBOLD).size(13).color(primary_fg),
                text(subtitle.to_string())
                    .font(FONT_REGULAR)
                    .size(11)
                    .color(secondary_fg),
            ]
            .spacing(1),
        ]
        .spacing(10)
        .align_y(Alignment::Center),
    )
    .width(Length::FillPortion(1))
    .height(66)
    .padding([10, 12])
    .style(quick_tile_button(tokens, active))
    .on_press(on_press)
    .into()
}

fn pill_switch<'a>(
    tokens: &'a ThemeTokens,
    checked: bool,
    on_press: Message,
) -> Element<'a, Message> {
    let track_bg = color(if checked {
        tokens.accent_warm
    } else {
        tokens.surface_subtle
    });
    let track_border = color(if checked {
        tokens.accent_warm
    } else {
        tokens.border_control
    });
    let knob_bg = Color::WHITE;

    let knob = container(Space::new())
        .width(18)
        .height(18)
        .style(move |_| container::Style {
            background: Some(Background::Color(knob_bg)),
            border: Border {
                color: Color::from_rgba(0.0, 0.0, 0.0, 0.08),
                width: 0.5,
                radius: Radius::from(9.0),
            },
            ..container::Style::default()
        });

    let track_row = if checked {
        row![space::horizontal(), knob]
    } else {
        row![knob, space::horizontal()]
    };

    button(
        container(track_row.align_y(Alignment::Center))
            .width(42)
            .height(24)
            .padding(3)
            .align_y(Alignment::Center)
            .style(move |_| container::Style {
                background: Some(Background::Color(track_bg)),
                border: Border {
                    color: track_border,
                    width: 1.0,
                    radius: Radius::from(12.0),
                },
                ..container::Style::default()
            }),
    )
    .padding(0)
    .style(|_, _| button::Style {
        background: None,
        ..button::Style::default()
    })
    .on_press(on_press)
    .into()
}
