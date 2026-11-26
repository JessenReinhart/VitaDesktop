use crate::icons;
use crate::message::Message;
use crate::style::{
    FONT_MONO, FONT_REGULAR, calm_scrollable_style, color, hairline_divider, menu_row_button,
    popover_card_surface, transparent_container,
};
use iced::widget::{Space, button, column, container, row, scrollable, space, text};
use iced::{Alignment, Element, Length, Padding};
use vita_core::{GlobalMenuCategory, ThemeTokens};

pub const GLOBAL_MENU_WIDTH: f32 = 264.0;
const SHADOW_GUTTER: f32 = 16.0;

pub fn estimate_height(category: Option<&GlobalMenuCategory>) -> f32 {
    let inner = match category {
        Some(cat) if cat.loaded && !cat.items.is_empty() => {
            let rows_h: f32 = cat
                .items
                .iter()
                .map(|i| if i.is_separator { 9.0 } else { 30.0 })
                .sum();
            (rows_h + 16.0).min(440.0)
        }
        _ => 56.0,
    };
    inner + SHADOW_GUTTER * 2.0
}

pub fn view<'a>(
    tokens: &'a ThemeTokens,
    category: Option<&'a GlobalMenuCategory>,
) -> Element<'a, Message> {
    let primary_fg = color(tokens.text_primary);
    let secondary_fg = color(tokens.text_secondary);
    let muted_fg = color(tokens.text_muted);
    let accent_fg = color(tokens.accent_primary);

    let inner_body: Element<'a, Message> = match category {
        Some(cat) if !cat.loaded => container(
            text("Loading menu actions...")
                .font(FONT_REGULAR)
                .size(12.5)
                .color(secondary_fg),
        )
        .padding([12, 14])
        .width(Length::Fill)
        .into(),
        Some(cat) if !cat.items.is_empty() => {
            let mut list = column![].spacing(1).width(Length::Fill);
            for item in &cat.items {
                if item.is_separator {
                    list = list.push(
                        container(
                            container(Space::new().width(Length::Fill).height(1))
                                .width(Length::Fill)
                                .height(1)
                                .style({
                                    let tokens = *tokens;
                                    move |_| hairline_divider(&tokens)
                                }),
                        )
                        .padding([4, 8]),
                    );
                    continue;
                }

                let check_slot: Element<'a, Message> = if item.checked {
                    icons::icon(icons::CHECK, 12.0, accent_fg).into()
                } else {
                    Space::new().width(12).height(12).into()
                };

                let label_color = if item.enabled { primary_fg } else { muted_fg };
                let mut content = row![
                    check_slot,
                    text(&item.label)
                        .font(FONT_REGULAR)
                        .size(12.5)
                        .color(label_color),
                    space::horizontal(),
                ]
                .spacing(8)
                .align_y(Alignment::Center)
                .width(Length::Fill);

                if let Some(sc) = &item.shortcut {
                    content = content.push(text(sc).font(FONT_MONO).size(11).color(muted_fg));
                }

                if item.enabled
                    && let Some(action) = item.action.clone()
                {
                    list = list.push(
                        button(content)
                            .width(Length::Fill)
                            .padding([6, 10])
                            .style(menu_row_button(tokens, false))
                            .on_press(Message::GlobalMenuInvoke(action)),
                    );
                } else {
                    list = list.push(container(content).width(Length::Fill).padding([6, 10]));
                }
            }

            scrollable(list)
                .height(Length::Shrink)
                .style(calm_scrollable_style(tokens))
                .into()
        }
        _ => container(
            text("No menu actions exposed for this menu")
                .font(FONT_REGULAR)
                .size(12)
                .color(muted_fg),
        )
        .padding([12, 14])
        .width(Length::Fill)
        .into(),
    };

    let card = container(inner_body)
        .width(Length::Fill)
        .padding(Padding::from([8, 6]))
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
