use crate::icons;
use crate::message::Message;
use crate::style::{
    FONT_MEDIUM, FONT_REGULAR, FONT_SEMIBOLD, calm_scrollable_style, color, popover_card_surface,
    subtle_pill_button, transparent_container,
};
use iced::border::Radius;
use iced::widget::{Space, button, column, container, row, scrollable, space, text};
use iced::{Alignment, Background, Border, Element, Length};
use vita_core::{NotificationLevel, ThemeTokens, ToastNotification};

pub const NOTIF_CENTER_WIDTH: f32 = 376.0;
pub const NOTIF_CENTER_HEIGHT: f32 = 408.0;
pub const TOAST_WINDOW_WIDTH: f32 = 356.0;
pub const TOAST_WINDOW_HEIGHT: f32 = 114.0;
const SHADOW_GUTTER: f32 = 16.0;

fn level_color(tokens: &ThemeTokens, level: NotificationLevel) -> iced::Color {
    color(match level {
        NotificationLevel::Info => tokens.accent_primary,
        NotificationLevel::Success => tokens.status_success,
        NotificationLevel::Caution => tokens.status_caution,
    })
}

pub fn view_center<'a>(
    tokens: &'a ThemeTokens,
    notifications: &'a [ToastNotification],
    focus_mode: bool,
) -> Element<'a, Message> {
    let primary_fg = color(tokens.text_primary);
    let secondary_fg = color(tokens.text_secondary);

    let mut header = row![
        text("Notifications")
            .font(FONT_SEMIBOLD)
            .size(15)
            .color(primary_fg),
        space::horizontal(),
    ]
    .align_y(Alignment::Center)
    .width(Length::Fill);

    if !notifications.is_empty() {
        header = header.push(
            button(
                text("Clear all")
                    .font(FONT_MEDIUM)
                    .size(11)
                    .color(secondary_fg),
            )
            .padding([4, 9])
            .style(subtle_pill_button(tokens, false, tokens.r8))
            .on_press(Message::ClearAllNotifications),
        );
    }

    let mut body = column![header].spacing(10).width(Length::Fill);

    if focus_mode {
        body = body.push(
            container(
                row![
                    icons::icon(icons::MOON, 14.0, color(tokens.accent_primary)),
                    text("Focus mode is on. Popups are paused.")
                        .font(FONT_MEDIUM)
                        .size(11)
                        .color(primary_fg),
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
                    ..container::Style::default()
                }
            }),
        );
    }

    if notifications.is_empty() {
        body = body.push(
            container(
                column![
                    container(icons::icon(icons::BELL, 20.0, secondary_fg))
                        .width(42)
                        .height(42)
                        .center_x(42)
                        .center_y(42)
                        .style({
                            let tokens = *tokens;
                            move |_| container::Style {
                                background: Some(Background::Color(color(tokens.surface_subtle))),
                                border: Border {
                                    radius: Radius::from(14.0),
                                    ..Border::default()
                                },
                                ..container::Style::default()
                            }
                        }),
                    text("All quiet here.")
                        .font(FONT_SEMIBOLD)
                        .size(14)
                        .color(primary_fg),
                    text("Shell updates and system alerts appear here.")
                        .font(FONT_REGULAR)
                        .size(12)
                        .color(secondary_fg),
                ]
                .spacing(6)
                .align_x(Alignment::Center),
            )
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill),
        );
    } else {
        let mut list = column![].spacing(8).width(Length::Fill);
        for notif in notifications.iter().rev() {
            list = list.push(notification_card(tokens, notif));
        }
        body = body.push(
            scrollable(list)
                .width(Length::Fill)
                .height(Length::Fill)
                .style(calm_scrollable_style(tokens)),
        );
    }

    let card = container(body)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(16)
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

pub fn view_toast<'a>(
    tokens: &'a ThemeTokens,
    notif: &'a ToastNotification,
) -> Element<'a, Message> {
    let card = container(toast_inner(tokens, notif))
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(14)
        .style({
            let tokens = *tokens;
            move |_| popover_card_surface(&tokens, tokens.r16)
        });

    container(card)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(SHADOW_GUTTER)
        .style(|_| transparent_container())
        .into()
}

fn notification_card<'a>(
    tokens: &'a ThemeTokens,
    notif: &'a ToastNotification,
) -> Element<'a, Message> {
    container(toast_inner(tokens, notif))
        .width(Length::Fill)
        .padding(12)
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
        })
        .into()
}

fn toast_inner<'a>(
    tokens: &'a ThemeTokens,
    notif: &'a ToastNotification,
) -> Element<'a, Message> {
    let primary_fg = color(tokens.text_primary);
    let secondary_fg = color(tokens.text_secondary);
    let muted_fg = color(tokens.text_muted);
    let dot_color = level_color(tokens, notif.level);

    let top_row = row![
        container(Space::new())
            .width(7)
            .height(7)
            .style(move |_| container::Style {
                background: Some(Background::Color(dot_color)),
                border: Border {
                    radius: Radius::from(3.5),
                    ..Border::default()
                },
                ..container::Style::default()
            }),
        text(&notif.app_name)
            .font(FONT_SEMIBOLD)
            .size(11)
            .color(secondary_fg),
        space::horizontal(),
        text(&notif.timestamp)
            .font(FONT_REGULAR)
            .size(10)
            .color(muted_fg),
        button(icons::icon(icons::CLOSE, 12.0, muted_fg))
            .padding(3)
            .style(subtle_pill_button(tokens, false, tokens.r8))
            .on_press(Message::DismissNotification(notif.id)),
    ]
    .spacing(6)
    .align_y(Alignment::Center);

    column![
        top_row,
        text(&notif.title)
            .font(FONT_SEMIBOLD)
            .size(13)
            .color(primary_fg),
        text(&notif.body)
            .font(FONT_REGULAR)
            .size(12)
            .color(secondary_fg),
    ]
    .spacing(2)
    .width(Length::Fill)
    .into()
}
