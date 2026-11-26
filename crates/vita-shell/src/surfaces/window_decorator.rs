use crate::icons;
use crate::message::Message;
use crate::style::{FONT_MEDIUM, color, transparent_container};
use iced::border::Radius;
use iced::widget::{Space, button, container, image, mouse_area, row, space, text};
use iced::{Alignment, Background, Border, Color, Element, Length};
use vita_core::{
    DecoratedWindowState, DecoratorButtonStyle, DecoratorMode, RgbaColor, ThemeTokens,
    WindowDecoratorConfig,
};

pub const FULL_HEADER_HEIGHT: f32 = 34.0;
pub const CONTROLS_ONLY_WIDTH: f32 = 140.0;
pub const CONTROLS_ONLY_HEIGHT: f32 = 29.0;

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
    config: &WindowDecoratorConfig,
    state: &'a DecoratedWindowState,
    app_icon: Option<&'a image::Handle>,
) -> Element<'a, Message> {
    let primary_fg = color(tokens.text_primary);

    match state.mode {
        DecoratorMode::FullHeader => {
            let icon_el: Element<'a, Message> = if let Some(handle) = app_icon {
                image(handle.clone()).width(16).height(16).into()
            } else {
                icons::icon(icons::WINDOW, 15.0, color(tokens.text_secondary)).into()
            };

            let drag_area = mouse_area(
                container(
                    row![
                        icon_el,
                        text(truncate(&state.title, 46))
                            .font(FONT_MEDIUM)
                            .size(12.5)
                            .color(primary_fg),
                        space::horizontal(),
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center)
                    .width(Length::Fill),
                )
                .width(Length::Fill)
                .height(Length::Fill)
                .padding([0, 8])
                .align_y(Alignment::Center)
                .style(|_| transparent_container()),
            )
            .on_press(Message::DecoratorHeaderPressed(state.hwnd));

            let controls = render_window_controls(
                tokens,
                state.hwnd,
                state.is_maximized,
                config.button_style,
                false,
            );

            container(
                row![drag_area, controls]
                    .spacing(4)
                    .align_y(Alignment::Center)
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .width(Length::Fill)
            .height(Length::Fill)
            .padding([0, 8])
            .style({
                let tokens = *tokens;
                let max = state.is_maximized;
                move |_| container::Style {
                    background: Some(Background::Color(color(tokens.surface_elevated))),
                    border: Border {
                        color: color(tokens.border_control),
                        width: 1.0,
                        radius: if max {
                            Radius::from(0.0)
                        } else {
                            Radius {
                                top_left: 8.0,
                                top_right: 8.0,
                                bottom_left: 0.0,
                                bottom_right: 0.0,
                            }
                        },
                    },
                    snap: true,
                    ..container::Style::default()
                }
            })
            .into()
        }
        DecoratorMode::ControlsOnly => {
            let bg_color = state.titlebar_bg.unwrap_or(tokens.surface_elevated);
            let mut adapted = *tokens;
            if state.titlebar_bg.is_some() {
                if bg_color.relative_luminance() > 0.35 {
                    adapted.text_secondary = RgbaColor::from_hex("#4A506A");
                    adapted.text_primary = RgbaColor::from_hex("#191C2F");
                    adapted.surface_subtle = RgbaColor::rgba(0.0, 0.0, 0.0, 0.06);
                } else {
                    adapted.text_secondary = RgbaColor::from_hex("#BEC1D2");
                    adapted.text_primary = RgbaColor::from_hex("#F7F7FA");
                    adapted.surface_subtle = RgbaColor::rgba(1.0, 1.0, 1.0, 0.08);
                }
            }

            let controls = render_window_controls(
                &adapted,
                state.hwnd,
                state.is_maximized,
                config.button_style,
                true,
            );

            container(controls)
                .width(Length::Fill)
                .height(Length::Fill)
                .align_x(iced::alignment::Horizontal::Right)
                .center_y(Length::Fill)
                .padding([0, 4])
                .style(move |_| container::Style {
                    background: Some(Background::Color(color(bg_color))),
                    border: Border {
                        color: Color::TRANSPARENT,
                        width: 0.0,
                        radius: Radius {
                            top_left: 0.0,
                            top_right: if state.is_maximized { 0.0 } else { 7.0 },
                            bottom_left: 0.0,
                            bottom_right: 0.0,
                        },
                    },
                    snap: true,
                    ..container::Style::default()
                })
                .into()
        }
    }
}

/// Reusable Minimize / Maximize-Restore / Close button cluster shared between the custom
/// window decorator and GNOME-Unite maximized Top Bar controls.
pub fn render_window_controls<'a>(
    tokens: &ThemeTokens,
    hwnd: isize,
    is_maximized: bool,
    style: DecoratorButtonStyle,
    compact: bool,
) -> Element<'a, Message> {
    let icon_fg = color(tokens.text_secondary);
    let close_fg = color(tokens.text_primary);
    let max_glyph = if is_maximized {
        icons::RESTORE
    } else {
        icons::MAXIMIZE
    };

    match style {
        DecoratorButtonStyle::Horizon => {
            let sz = if compact { 12.0 } else { 14.0 };
            let pad = if compact { [6, 12] } else { [4, 6] };
            let min_w = if compact { 9.0 } else { 10.0 };

            let min_glyph = container(container(Space::new().width(min_w).height(1.8)).style(
                move |_| container::Style {
                    background: Some(Background::Color(icon_fg)),
                    border: Border {
                        radius: Radius::from(1.0),
                        ..Border::default()
                    },
                    ..container::Style::default()
                },
            ))
            .width(sz)
            .height(sz)
            .center_x(sz)
            .center_y(sz);

            let min_btn = button(min_glyph)
                .padding(pad)
                .style(hover_pill_button(*tokens, tokens.r8))
                .on_press(Message::DecoratorMinimize(hwnd));

            let max_btn = button(icons::icon(max_glyph, sz, icon_fg))
                .padding(pad)
                .style(hover_pill_button(*tokens, tokens.r8))
                .on_press(Message::DecoratorToggleMaximize(hwnd));

            let close_btn = button(icons::icon(icons::CLOSE, sz, close_fg))
                .padding(pad)
                .style(close_pill_button(*tokens))
                .on_press(Message::DecoratorClose(hwnd));

            row![min_btn, max_btn, close_btn]
                .spacing(4)
                .align_y(Alignment::Center)
                .into()
        }
        DecoratorButtonStyle::TrafficLights => {
            let min_dot = traffic_dot(
                tokens,
                RgbaColor::rgba(0.910, 0.722, 0.294, 1.0),
                compact,
                Message::DecoratorMinimize(hwnd),
            );
            let max_dot = traffic_dot(
                tokens,
                RgbaColor::rgba(0.310, 0.659, 0.482, 1.0),
                compact,
                Message::DecoratorToggleMaximize(hwnd),
            );
            let close_dot = traffic_dot(
                tokens,
                RgbaColor::rgba(0.878, 0.365, 0.267, 1.0),
                compact,
                Message::DecoratorClose(hwnd),
            );

            row![min_dot, max_dot, close_dot]
                .spacing(if compact { 6 } else { 8 })
                .padding([0, 6])
                .align_y(Alignment::Center)
                .into()
        }
    }
}

fn traffic_dot<'a>(
    tokens: &ThemeTokens,
    dot_color: RgbaColor,
    compact: bool,
    msg: Message,
) -> Element<'a, Message> {
    let border_c = color(RgbaColor::rgba(
        dot_color.r * 0.82,
        dot_color.g * 0.82,
        dot_color.b * 0.82,
        1.0,
    ));
    let diameter = if compact { 12.0 } else { 13.0 };
    let inner =
        container(Space::new().width(diameter).height(diameter)).style(move |_| container::Style {
            background: Some(Background::Color(color(dot_color))),
            border: Border {
                color: border_c,
                width: 1.0,
                radius: Radius::from(99.0),
            },
            ..container::Style::default()
        });

    if compact {
        let wrapped = container(inner)
            .width(28)
            .height(24)
            .center_x(28)
            .center_y(24);
        button(wrapped)
            .padding(0)
            .style(hover_pill_button(*tokens, tokens.r8))
            .on_press(msg)
            .into()
    } else {
        button(inner)
            .padding(2)
            .style(|_, _| button::Style {
                background: None,
                ..button::Style::default()
            })
            .on_press(msg)
            .into()
    }
}

fn hover_pill_button(
    tokens: ThemeTokens,
    radius: f32,
) -> impl Fn(&iced::Theme, button::Status) -> button::Style {
    move |_theme, status| {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        let bg = hovered.then_some(Background::Color(color(tokens.surface_subtle)));
        button::Style {
            background: bg,
            text_color: color(tokens.text_primary),
            border: Border {
                color: Color::TRANSPARENT,
                width: 0.0,
                radius: Radius::from(radius),
            },
            ..button::Style::default()
        }
    }
}

fn close_pill_button(
    tokens: ThemeTokens,
) -> impl Fn(&iced::Theme, button::Status) -> button::Style {
    move |_theme, status| {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        let bg = if hovered {
            Some(Background::Color(color(
                tokens.status_error.with_alpha(0.18),
            )))
        } else {
            None
        };
        let border_c = if hovered {
            color(tokens.status_error.with_alpha(0.45))
        } else {
            Color::TRANSPARENT
        };
        button::Style {
            background: bg,
            text_color: color(tokens.text_primary),
            border: Border {
                color: border_c,
                width: 1.0,
                radius: Radius::from(tokens.r8),
            },
            ..button::Style::default()
        }
    }
}
