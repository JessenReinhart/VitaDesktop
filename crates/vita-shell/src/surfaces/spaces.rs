use crate::icons;
use crate::message::Message;
use crate::style::{
    FONT_MEDIUM, FONT_REGULAR, FONT_SEMIBOLD, calm_scrollable_style, color, hairline_divider,
    popover_card_surface, surface_card_button, transparent_container,
};
use iced::border::Radius;
use iced::widget::{Space, Svg, button, column, container, row, scrollable, space, svg, text};
use iced::{Alignment, Background, Border, Element, Length, Padding};
use vita_core::{SpaceInfo, ThemeTokens};

pub const SPACES_WINDOW_WIDTH: f32 = 724.0;
pub const SPACES_WINDOW_HEIGHT: f32 = 376.0;
const SHADOW_GUTTER: f32 = 18.0;

const ART_PERSONAL: &[u8] = include_bytes!("../../../../assets/spaces/personal.svg");
const ART_STUDIO: &[u8] = include_bytes!("../../../../assets/spaces/studio.svg");
const ART_UNWIND: &[u8] = include_bytes!("../../../../assets/spaces/unwind.svg");
const ART_NIGHT: &[u8] = include_bytes!("../../../../assets/spaces/night.svg");

fn space_art<'a>(index: usize, is_dark: bool) -> Svg<'a> {
    let bytes = if is_dark {
        ART_NIGHT
    } else {
        match index % 3 {
            0 => ART_PERSONAL,
            1 => ART_STUDIO,
            _ => ART_UNWIND,
        }
    };
    Svg::new(svg::Handle::from_memory(bytes))
        .width(Length::Fill)
        .height(86)
}

pub fn view<'a>(tokens: &'a ThemeTokens, spaces: &'a [SpaceInfo]) -> Element<'a, Message> {
    let primary_fg = color(tokens.text_primary);
    let secondary_fg = color(tokens.text_secondary);
    let subtle_fg = color(tokens.text_muted);

    let header = row![
        column![
            text("Spaces")
                .font(FONT_SEMIBOLD)
                .size(16)
                .color(primary_fg),
            text("A space for every side of you.")
                .font(FONT_REGULAR)
                .size(12)
                .color(secondary_fg),
        ]
        .spacing(2),
        space::horizontal(),
        button(
            row![
                text("New space")
                    .font(FONT_MEDIUM)
                    .size(12)
                    .color(primary_fg),
                icons::icon(icons::PLUS, 14.0, primary_fg),
            ]
            .spacing(6)
            .align_y(Alignment::Center),
        )
        .padding([8, 12])
        .style(surface_card_button(tokens, false, tokens.r12))
        .on_press(Message::CreateNewSpace),
    ]
    .align_y(Alignment::Center)
    .width(Length::Fill);

    let mut cards_row = row![].spacing(12).align_y(Alignment::Center);
    for space in spaces {
        cards_row = cards_row.push(space_card(tokens, space));
    }

    let cards_strip: Element<'a, Message> = if spaces.len() > 3 {
        scrollable(cards_row)
            .direction(scrollable::Direction::Horizontal(
                scrollable::Scrollbar::new().width(4).scroller_width(4),
            ))
            .width(Length::Fill)
            .style(calm_scrollable_style(tokens))
            .into()
    } else {
        container(cards_row)
            .width(Length::Fill)
            .center_x(Length::Fill)
            .into()
    };

    let footer = column![
        container(Space::new())
            .width(Length::Fill)
            .height(1)
            .style({
                let tokens = *tokens;
                move |_| hairline_divider(&tokens)
            }),
        row![
            text("Alt + `  Overview   ·   Win + Ctrl + ← / →  Switch spaces   ·   Esc  Close")
                .font(FONT_REGULAR)
                .size(11)
                .color(subtle_fg),
        ]
        .padding(Padding {
            top: 8.0,
            ..Padding::ZERO
        })
        .width(Length::Fill)
        .align_y(Alignment::Center),
    ];

    let card = container(
        column![header, cards_strip, footer]
            .spacing(16)
            .width(Length::Fill)
            .height(Length::Fill),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .padding(Padding {
        top: 18.0,
        right: 20.0,
        bottom: 14.0,
        left: 20.0,
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

fn space_card<'a>(tokens: &'a ThemeTokens, space_item: &'a SpaceInfo) -> Element<'a, Message> {
    let primary_fg = color(tokens.text_primary);
    let secondary_fg = color(tokens.text_secondary);
    let subtle_fg = color(tokens.text_muted);
    let warm_border = color(tokens.accent_warm);
    let soft_badge_bg = color(tokens.accent_warm.with_alpha(0.16));

    let count_label = match space_item.window_count {
        0 => "Empty".to_string(),
        1 => "1 window".to_string(),
        n => format!("{n} windows"),
    };

    let mut title_row = row![
        text(&space_item.name)
            .font(FONT_SEMIBOLD)
            .size(14)
            .color(primary_fg),
        space::horizontal(),
    ]
    .align_y(Alignment::Center);

    if space_item.is_current {
        title_row = title_row.push(
            container(
                text("CURRENT")
                    .font(FONT_SEMIBOLD)
                    .size(9)
                    .color(warm_border),
            )
            .padding([2, 6])
            .style(move |_| container::Style {
                background: Some(Background::Color(soft_badge_bg)),
                border: Border {
                    radius: Radius::from(6.0),
                    ..Border::default()
                },
                ..container::Style::default()
            }),
        );
    }

    let body = column![
        space_art(space_item.index, tokens.is_dark),
        column![
            title_row,
            text(&space_item.tagline)
                .font(FONT_REGULAR)
                .size(11)
                .color(secondary_fg),
            Space::new().height(4),
            text(count_label)
                .font(FONT_MEDIUM)
                .size(11)
                .color(subtle_fg),
        ]
        .spacing(2)
        .padding([6, 4]),
    ]
    .spacing(6);

    let tokens_copy = *tokens;
    let is_current = space_item.is_current;
    button(body)
        .width(206)
        .height(194)
        .padding(8)
        .style(move |_theme, status| {
            let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
            let bg = if hovered {
                color(tokens_copy.surface_subtle)
            } else {
                color(tokens_copy.surface_elevated)
            };
            button::Style {
                background: Some(Background::Color(bg)),
                text_color: primary_fg,
                border: Border {
                    color: if is_current {
                        warm_border
                    } else if hovered {
                        color(tokens_copy.text_muted)
                    } else {
                        color(tokens_copy.border_control)
                    },
                    width: if is_current { 2.0 } else { 1.0 },
                    radius: Radius::from(tokens_copy.r16),
                },
                ..button::Style::default()
            }
        })
        .on_press(Message::SwitchSpace(space_item.index))
        .into()
}
