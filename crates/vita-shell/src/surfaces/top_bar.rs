use crate::icons;
use crate::message::Message;
use crate::style::{
    FONT_MEDIUM, FONT_REGULAR, FONT_SEMIBOLD, color, floating_pill_surface,
    full_width_topbar_style, hairline_divider, subtle_pill_button, transparent_container,
};
use crate::surfaces::window_decorator;
use chrono::{DateTime, Local};
use iced::border::Radius;
use iced::widget::{Space, button, container, row, space, stack, text};
use iced::{Alignment, Background, Border, Element, Length, Padding};
use vita_core::{
    ActiveAppMenu, DecoratedWindowState, ShellConfig, SystemSummary, ThemeTokens, TopBarStyle,
};

pub struct TopBarState<'a> {
    pub config: &'a ShellConfig,
    pub tokens: &'a ThemeTokens,
    pub summary: &'a SystemSummary,
    pub now: DateTime<Local>,
    pub monitor_idx: usize,
    pub focus_mode: bool,
    pub quick_controls_open: bool,
    pub notifications_open: bool,
    pub tray_open: bool,
    pub calendar_open: bool,
    pub has_unread_notifs: bool,
    pub active_menu: Option<&'a ActiveAppMenu>,
    pub open_global_menu: Option<(usize, usize)>,
    pub maximized_window: Option<&'a DecoratedWindowState>,
    pub has_maximized_window: bool,
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

pub fn view<'a>(state: TopBarState<'a>) -> Element<'a, Message> {
    match state.config.top_bar.style {
        TopBarStyle::FullWidth => view_pantheon_full_width(&state),
        TopBarStyle::FloatingPills => view_horizon_floating_pills(&state),
    }
}

fn global_menu_strip<'a>(
    state: &TopBarState<'a>,
    tokens: &'a ThemeTokens,
    pill_radius: f32,
) -> Option<Element<'a, Message>> {
    if !state.config.top_bar.show_global_menu {
        return None;
    }
    let menu = state.active_menu?;
    if menu.categories.is_empty() {
        return None;
    }

    let primary_fg = color(tokens.text_primary);
    let secondary_fg = color(tokens.text_secondary);
    let mut strip = row![].spacing(2).align_y(Alignment::Center);

    for (idx, cat) in menu.categories.iter().take(7).enumerate() {
        let is_open = state.open_global_menu == Some((state.monitor_idx, idx));
        let label = truncate(&cat.label, if idx == 0 { 16 } else { 12 });
        let font = if idx == 0 { FONT_SEMIBOLD } else { FONT_REGULAR };
        let fg = if idx == 0 || is_open {
            primary_fg
        } else {
            secondary_fg
        };

        strip = strip.push(
            button(text(label).font(font).size(12.5).color(fg))
                .padding([4, 8])
                .style(subtle_pill_button(tokens, is_open, pill_radius))
                .on_press(Message::OpenGlobalMenu {
                    monitor_idx: state.monitor_idx,
                    menu_index: idx,
                }),
        );
    }

    Some(strip.into())
}

fn view_pantheon_full_width<'a>(state: &TopBarState<'a>) -> Element<'a, Message> {
    let config = state.config;
    let tokens = state.tokens;
    let icon_fg = color(tokens.text_primary);
    let muted_fg = color(tokens.text_secondary);

    let apps_btn = button(
        row![
            icons::icon(icons::SEARCH, 14.0, icon_fg),
            text("Applications").font(FONT_MEDIUM).size(13),
        ]
        .spacing(7)
        .align_y(Alignment::Center),
    )
    .padding([4, 10])
    .style(subtle_pill_button(tokens, false, tokens.r8))
    .on_press(Message::ToggleLauncherOn(state.monitor_idx));

    let secondary_left: Element<'a, Message> = if let Some(gm) =
        global_menu_strip(state, tokens, tokens.r8)
    {
        gm
    } else {
        button(
            row![
                icons::icon(icons::LAYERS, 13.0, muted_fg),
                text(config.preset.label())
                    .font(FONT_MEDIUM)
                    .size(11)
                    .color(muted_fg),
            ]
            .spacing(5)
            .align_y(Alignment::Center),
        )
        .padding([3, 8])
        .style(subtle_pill_button(tokens, false, tokens.pill))
        .on_press(Message::CyclePreset)
        .into()
    };

    let left_cluster = container(
        row![apps_btn, secondary_left]
            .spacing(6)
            .align_y(Alignment::Center),
    )
    .width(Length::FillPortion(1))
    .align_x(iced::alignment::Horizontal::Left);

    let time_fmt = if config.top_bar.show_seconds {
        "%a, %b %-d  %H:%M:%S"
    } else {
        "%a, %b %-d  %H:%M"
    };
    let clock = button(
        text(state.now.format(time_fmt).to_string())
            .font(FONT_SEMIBOLD)
            .size(13)
            .color(icon_fg),
    )
    .padding([4, 10])
    .style(subtle_pill_button(tokens, state.calendar_open, tokens.r8))
    .on_press(Message::ToggleCalendarOn(state.monitor_idx));

    let status_btn = button(status_indicators(
        tokens,
        state.summary,
        state.focus_mode,
        true,
    ))
    .padding([4, 8])
    .style(subtle_pill_button(
        tokens,
        state.quick_controls_open,
        tokens.r8,
    ))
    .on_press(Message::ToggleQuickControlsOn(state.monitor_idx));

    let mut right_items = row![
        tray_button(tokens, state.tray_open, state.monitor_idx, [4, 7], tokens.r8),
        theme_toggle_button(tokens),
        bell_button(
            tokens,
            state.notifications_open,
            state.has_unread_notifs,
            state.monitor_idx,
            6,
        ),
        status_btn,
    ]
    .spacing(4)
    .align_y(Alignment::Center);

    if config.decorator.enabled
        && config.decorator.maximized_topbar_controls
        && let Some(max_win) = state.maximized_window
        && max_win.monitor_idx == state.monitor_idx
    {
        right_items = right_items.push(window_decorator::render_window_controls(
            tokens,
            max_win.hwnd,
            true,
            config.decorator.button_style,
            true,
        ));
    }

    right_items = right_items.push(exit_button(tokens));

    let right_cluster = container(right_items)
        .width(Length::FillPortion(1))
        .align_x(iced::alignment::Horizontal::Right);

    let bar_row = row![left_cluster, clock, right_cluster]
        .spacing(12)
        .align_y(Alignment::Center)
        .width(Length::Fill)
        .height(Length::Fill);

    let solid_max = config.decorator.enabled
        && config.decorator.hide_maximized_titlebar
        && state.has_maximized_window;

    container(bar_row)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding([0, 12])
        .style({
            let tokens = *tokens;
            move |_| {
                let mut s = full_width_topbar_style(&tokens);
                if solid_max {
                    s.background = Some(Background::Color(color(
                        tokens.surface_topbar.with_alpha(1.0),
                    )));
                }
                s
            }
        })
        .into()
}

fn view_horizon_floating_pills<'a>(state: &TopBarState<'a>) -> Element<'a, Message> {
    let config = state.config;
    let tokens = state.tokens;
    let icon_fg = color(tokens.text_primary);
    let muted_fg = color(tokens.text_secondary);

    let brand_pill = container(
        button(
            row![
                icons::vita_mark(),
                text("Vita").font(FONT_SEMIBOLD).size(13).color(icon_fg),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        )
        .padding([5, 12])
        .style(subtle_pill_button(tokens, false, tokens.pill))
        .on_press(Message::ToggleLauncherOn(state.monitor_idx)),
    )
    .height(34)
    .align_y(Alignment::Center)
    .style({
        let tokens = *tokens;
        move |_| floating_pill_surface(&tokens, tokens.pill)
    });

    let secondary_left_pill: Element<'a, Message> = if let Some(gm) =
        global_menu_strip(state, tokens, tokens.pill)
    {
        container(gm)
            .height(34)
            .padding([0, 6])
            .align_y(Alignment::Center)
            .style({
                let tokens = *tokens;
                move |_| floating_pill_surface(&tokens, tokens.pill)
            })
            .into()
    } else {
        container(
            button(
                row![
                    icons::icon(icons::LAYERS, 13.0, muted_fg),
                    text(config.preset.label())
                        .font(FONT_MEDIUM)
                        .size(12)
                        .color(muted_fg),
                ]
                .spacing(6)
                .align_y(Alignment::Center),
            )
            .padding([5, 12])
            .style(subtle_pill_button(tokens, false, tokens.pill))
            .on_press(Message::CyclePreset),
        )
        .height(34)
        .align_y(Alignment::Center)
        .style({
            let tokens = *tokens;
            move |_| floating_pill_surface(&tokens, tokens.pill)
        })
        .into()
    };

    let tray_pill = container(tray_button(
        tokens,
        state.tray_open,
        state.monitor_idx,
        [4, 7],
        tokens.pill,
    ))
    .height(34)
    .padding([0, 3])
    .align_y(Alignment::Center)
    .style({
        let tokens = *tokens;
        move |_| floating_pill_surface(&tokens, tokens.pill)
    });

    let theme_pill = container(theme_toggle_button(tokens))
        .width(34)
        .height(34)
        .center_x(34)
        .center_y(34)
        .style({
            let tokens = *tokens;
            move |_| floating_pill_surface(&tokens, tokens.pill)
        });

    let bell_pill = container(bell_button(
        tokens,
        state.notifications_open,
        state.has_unread_notifs,
        state.monitor_idx,
        7,
    ))
    .width(34)
    .height(34)
    .center_x(34)
    .center_y(34)
    .style({
        let tokens = *tokens;
        move |_| floating_pill_surface(&tokens, tokens.pill)
    });

    let time_fmt = if config.top_bar.show_seconds {
        "%a %-d %b  %H:%M:%S"
    } else {
        "%a %-d %b  %H:%M"
    };

    let indicators_btn = button(status_indicators(
        tokens,
        state.summary,
        state.focus_mode,
        false,
    ))
    .padding([4, 10])
    .style(subtle_pill_button(
        tokens,
        state.quick_controls_open,
        tokens.pill,
    ))
    .on_press(Message::ToggleQuickControlsOn(state.monitor_idx));

    let clock_btn = button(
        text(state.now.format(time_fmt).to_string())
            .font(FONT_MEDIUM)
            .size(13)
            .color(icon_fg),
    )
    .padding([4, 11])
    .style(subtle_pill_button(tokens, state.calendar_open, tokens.pill))
    .on_press(Message::ToggleCalendarOn(state.monitor_idx));

    let status_pill = container(
        row![
            indicators_btn,
            container(Space::new())
                .width(1)
                .height(14)
                .style({
                    let tokens = *tokens;
                    move |_| hairline_divider(&tokens)
                }),
            clock_btn,
        ]
        .spacing(2)
        .align_y(Alignment::Center),
    )
    .height(34)
    .padding([0, 3])
    .align_y(Alignment::Center)
    .style({
        let tokens = *tokens;
        move |_| floating_pill_surface(&tokens, tokens.pill)
    });

    let power_pill = container(exit_button(tokens))
        .width(34)
        .height(34)
        .center_x(34)
        .center_y(34)
        .style({
            let tokens = *tokens;
            move |_| floating_pill_surface(&tokens, tokens.pill)
        });

    let mut right_row = row![tray_pill, theme_pill, bell_pill, status_pill]
        .spacing(8)
        .align_y(Alignment::Center);

    if config.decorator.enabled
        && config.decorator.maximized_topbar_controls
        && let Some(max_win) = state.maximized_window
        && max_win.monitor_idx == state.monitor_idx
    {
        let win_pill = container(window_decorator::render_window_controls(
            tokens,
            max_win.hwnd,
            true,
            config.decorator.button_style,
            true,
        ))
        .height(34)
        .padding([0, 6])
        .align_y(Alignment::Center)
        .style({
            let tokens = *tokens;
            move |_| floating_pill_surface(&tokens, tokens.pill)
        });
        right_row = right_row.push(win_pill);
    }

    right_row = right_row.push(power_pill);

    let has_max_win = config.decorator.enabled
        && config.decorator.hide_maximized_titlebar
        && state.has_maximized_window;

    let content = row![
        row![brand_pill, secondary_left_pill]
            .spacing(8)
            .align_y(Alignment::Center),
        space::horizontal(),
        right_row,
    ]
    .align_y(Alignment::Center)
    .width(Length::Fill);

    container(content)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(if has_max_win {
            Padding {
                top: 0.0,
                right: 14.0,
                bottom: 0.0,
                left: 14.0,
            }
        } else {
            Padding {
                top: 8.0,
                right: 14.0,
                bottom: 0.0,
                left: 14.0,
            }
        })
        .align_y(if has_max_win {
            iced::alignment::Vertical::Center
        } else {
            iced::alignment::Vertical::Top
        })
        .style({
            let tokens = *tokens;
            move |_| {
                if has_max_win {
                    let mut s = full_width_topbar_style(&tokens);
                    s.background =
                        Some(Background::Color(color(tokens.surface_base.with_alpha(1.0))));
                    s
                } else {
                    transparent_container()
                }
            }
        })
        .into()
}

fn status_indicators<'a>(
    tokens: &'a ThemeTokens,
    summary: &'a SystemSummary,
    focus_mode: bool,
    show_volume_pct: bool,
) -> Element<'a, Message> {
    let icon_fg = color(tokens.text_primary);
    let secondary_fg = color(tokens.text_secondary);
    let accent_fg = color(tokens.accent_primary);

    let wifi_icon = if summary.wifi_connected {
        icons::WIFI
    } else {
        icons::WIFI_OFF
    };
    let vol_icon = if summary.is_muted || summary.volume_pct == 0 {
        icons::VOLUME_MUTE
    } else {
        icons::VOLUME_HIGH
    };

    let mut items = row![].spacing(10).align_y(Alignment::Center);
    if focus_mode {
        items = items.push(icons::icon(icons::MOON, 14.0, accent_fg));
    }

    items = items.push(icons::icon(wifi_icon, 15.0, icon_fg));
    items = items.push(if show_volume_pct {
        Element::from(
            row![
                icons::icon(vol_icon, 15.0, icon_fg),
                text(format!("{}%", summary.volume_pct))
                    .font(FONT_MEDIUM)
                    .size(12)
                    .color(secondary_fg),
            ]
            .spacing(4)
            .align_y(Alignment::Center),
        )
    } else {
        Element::from(icons::icon(vol_icon, 15.0, icon_fg))
    });

    if let Some(batt) = summary.battery_pct {
        let batt_icon = if summary.is_charging {
            icons::BATTERY_CHARGING
        } else {
            icons::BATTERY
        };
        items = items.push(
            row![
                icons::icon(batt_icon, 15.0, icon_fg),
                text(format!("{batt}%"))
                    .font(FONT_MEDIUM)
                    .size(12)
                    .color(secondary_fg),
            ]
            .spacing(4)
            .align_y(Alignment::Center),
        );
    }

    items.into()
}

fn tray_button<'a>(
    tokens: &'a ThemeTokens,
    active: bool,
    monitor_idx: usize,
    padding: [u16; 2],
    radius: f32,
) -> Element<'a, Message> {
    let icon_fg = color(tokens.text_primary);
    let muted_fg = color(tokens.text_secondary);
    button(
        row![
            icons::icon(icons::GRID, 13.0, icon_fg),
            icons::icon(icons::CHEVRON_DOWN, 11.0, muted_fg),
        ]
        .spacing(3)
        .align_y(Alignment::Center),
    )
    .padding(padding)
    .style(subtle_pill_button(tokens, active, radius))
    .on_press(Message::ToggleTrayOn(monitor_idx))
    .into()
}

fn bell_button<'a>(
    tokens: &'a ThemeTokens,
    active: bool,
    has_unread: bool,
    monitor_idx: usize,
    padding: u16,
) -> Element<'a, Message> {
    let icon_fg = color(tokens.text_primary);
    let bell = icons::icon(icons::BELL, 15.0, icon_fg);
    let content: Element<'a, Message> = if has_unread {
        let dot_color = color(tokens.accent_warm);
        let dot = container(Space::new()).width(6).height(6).style(move |_| {
            container::Style {
                background: Some(Background::Color(dot_color)),
                border: Border {
                    radius: Radius::from(3.0),
                    ..Border::default()
                },
                ..container::Style::default()
            }
        });
        stack![
            bell,
            container(dot)
                .width(17)
                .height(17)
                .align_x(iced::alignment::Horizontal::Right)
                .align_y(iced::alignment::Vertical::Top),
        ]
        .into()
    } else {
        bell.into()
    };

    button(content)
        .padding(padding)
        .style(subtle_pill_button(tokens, active, tokens.pill))
        .on_press(Message::ToggleNotificationsOn(monitor_idx))
        .into()
}

fn theme_toggle_button<'a>(tokens: &'a ThemeTokens) -> Element<'a, Message> {
    let icon_fg = color(tokens.text_primary);
    let glyph = if tokens.is_dark {
        icons::SUN
    } else {
        icons::MOON
    };
    button(icons::icon(glyph, 15.0, icon_fg))
        .padding(6)
        .style(subtle_pill_button(tokens, false, tokens.pill))
        .on_press(Message::ToggleTheme)
        .into()
}

fn exit_button<'a>(tokens: &'a ThemeTokens) -> Element<'a, Message> {
    let icon_fg = color(tokens.text_primary);
    button(icons::icon(icons::POWER, 15.0, icon_fg))
        .padding(6)
        .style(subtle_pill_button(tokens, false, tokens.pill))
        .on_press(Message::ExitRequested)
        .into()
}
