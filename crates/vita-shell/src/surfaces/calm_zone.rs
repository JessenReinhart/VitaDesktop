use crate::message::Message;
use crate::style::{
    FONT_MEDIUM, FONT_REGULAR, FONT_SEMIBOLD, color, surface_card_button, transparent_container,
};
use chrono::{DateTime, Local, Timelike};
use iced::border::Radius;
use iced::widget::{button, column, container, row, space, text};
use iced::{Alignment, Background, Border, Element, Length, Padding, Shadow, Vector};
use vita_core::{ShellConfig, SurfaceMaterial, ThemeTokens};

pub const CALM_ZONE_WIDTH: f32 = 368.0;
pub const CALM_ZONE_HEIGHT: f32 = 286.0;
const SHADOW_GUTTER: f32 = 14.0;

pub fn view<'a>(
    config: &'a ShellConfig,
    tokens: &'a ThemeTokens,
    now: DateTime<Local>,
) -> Element<'a, Message> {
    let primary_fg = color(tokens.text_primary);
    let secondary_fg = color(tokens.text_secondary);
    let accent_fg = color(tokens.accent_primary);
    let warm_fg = color(tokens.accent_warm);

    let date_upper = now.format("%A, %-d %B").to_string().to_ascii_uppercase();
    let time_str = now.format("%H:%M").to_string();

    let hour = now.hour();
    let greeting_prefix = if hour < 12 {
        "Good morning"
    } else if hour < 18 {
        "Good afternoon"
    } else {
        "Good evening"
    };
    let greeting = if config.calm_zone.user_name.trim().is_empty() {
        format!("{greeting_prefix}.")
    } else {
        format!("{greeting_prefix}, {}.", config.calm_zone.user_name.trim())
    };

    let mut content = column![
        text(date_upper)
            .font(FONT_SEMIBOLD)
            .size(11)
            .color(secondary_fg),
        text(time_str)
            .font(FONT_SEMIBOLD)
            .size(42)
            .color(primary_fg),
        column![
            text(greeting)
                .font(FONT_SEMIBOLD)
                .size(15)
                .color(primary_fg),
            text("A fresh space for whatever comes next.")
                .font(FONT_REGULAR)
                .size(12)
                .color(secondary_fg),
        ]
        .spacing(2),
    ]
    .spacing(4)
    .width(Length::Fill);

    if config.calm_zone.show_thread_widget {
        let thread_card = container(
            row![
                column![
                    text("YOUR PERSONAL THREAD")
                        .font(FONT_SEMIBOLD)
                        .size(9)
                        .color(accent_fg),
                    text(&config.calm_zone.thread_title)
                        .font(FONT_SEMIBOLD)
                        .size(13)
                        .color(primary_fg),
                    text(&config.calm_zone.thread_subtitle)
                        .font(FONT_REGULAR)
                        .size(11)
                        .color(secondary_fg),
                ]
                .spacing(2),
                space::horizontal(),
                button(text("Open Notes").font(FONT_MEDIUM).size(11).color(warm_fg),)
                    .padding([6, 10])
                    .style(surface_card_button(tokens, false, tokens.pill))
                    .on_press(Message::OpenTargetAndDismiss(
                        "shell:AppsFolder\\Microsoft.WindowsNotepad_8wekyb3d8bbwe!App".to_string(),
                    )),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        )
        .width(Length::Fill)
        .padding([11, 13])
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

        content = content.push(container(thread_card).padding(Padding {
            top: 8.0,
            ..Padding::ZERO
        }));
    }

    let bg = match tokens.material {
        SurfaceMaterial::Matte => tokens.surface_base,
        SurfaceMaterial::Frosted => tokens.surface_dock.with_alpha(0.86),
    };

    let card = container(content)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(Padding {
            top: 16.0,
            right: 18.0,
            bottom: 16.0,
            left: 18.0,
        })
        .style({
            let tokens = *tokens;
            move |_| container::Style {
                background: Some(Background::Color(color(bg))),
                text_color: Some(primary_fg),
                border: Border {
                    color: color(tokens.border_subtle),
                    width: 1.0,
                    radius: Radius::from(tokens.r24),
                },
                shadow: Shadow {
                    color: color(tokens.shadow_color),
                    offset: Vector::new(0.0, 6.0),
                    blur_radius: 18.0,
                },
                snap: true,
            }
        });

    container(card)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(SHADOW_GUTTER)
        .style(|_| transparent_container())
        .into()
}
