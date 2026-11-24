use crate::icons;
use crate::style::{
    FONT_MEDIUM, FONT_REGULAR, FONT_SEMIBOLD, calm_scrollable_style, color,
    fallback_app_tile_style, hairline_divider, menu_row_button, popover_card_surface,
    subtle_pill_button, transparent_container,
};
use iced::border::Radius;
use iced::mouse;
use iced::widget::{
    Space, button, column, container, image, mouse_area, row, scrollable, space, text,
};
use iced::{Alignment, Background, Border, Color, Element, Length, Padding};
use std::collections::HashMap;
use vita_core::ThemeTokens;
use vita_win32::TrayIconInfo;

pub const TRAY_POPOVER_WIDTH: f32 = 316.0;
const SHADOW_GUTTER: f32 = 16.0;
const ROW_HEIGHT: f32 = 46.0;
const ROW_SPACING: f32 = 4.0;
const MAX_VISIBLE_ROWS: usize = 7;

pub fn estimate_height(count: usize) -> f32 {
    let body_h = if count == 0 {
        194.0
    } else {
        let visible = count.min(MAX_VISIBLE_ROWS);
        let rows_h = (visible as f32) * ROW_HEIGHT
            + (visible.saturating_sub(1) as f32) * ROW_SPACING;
        68.0 + rows_h
    };
    body_h + SHADOW_GUTTER * 2.0
}

fn truncate_label(label: &str, max_chars: usize) -> String {
    let mut chars = label.chars();
    let head: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{}…", head.trim_end())
    } else {
        head
    }
}

fn display_title(item: &TrayIconInfo) -> &str {
    if !item.tooltip.trim().is_empty() {
        item.tooltip.trim()
    } else if !item.exe_name.trim().is_empty() {
        item.exe_name.trim()
    } else {
        "Tray icon"
    }
}

fn display_subtitle(item: &TrayIconInfo) -> String {
    let exe = item.exe_name.trim();
    match (exe.is_empty(), item.is_overflow) {
        (false, true) => format!("{exe} · Hidden tray"),
        (false, false) => exe.to_string(),
        (true, true) => "Hidden tray".to_string(),
        (true, false) => "System tray".to_string(),
    }
}

fn menu_dot<'a, Msg: 'a>(dot_color: Color) -> Element<'a, Msg> {
    container(Space::new())
        .width(3)
        .height(3)
        .style(move |_| container::Style {
            background: Some(Background::Color(dot_color)),
            border: Border {
                radius: Radius::from(1.5),
                ..Border::default()
            },
            ..container::Style::default()
        })
        .into()
}

pub fn view<'a, Msg: Clone + 'a>(
    tokens: &'a ThemeTokens,
    icons_list: &'a [TrayIconInfo],
    icon_handles: &'a HashMap<String, image::Handle>,
    on_left_click: impl Fn(usize) -> Msg + 'a,
    on_right_click: impl Fn(usize) -> Msg + 'a,
    on_refresh: Msg,
) -> Element<'a, Msg> {
    let primary_fg = color(tokens.text_primary);
    let secondary_fg = color(tokens.text_secondary);
    let muted_fg = color(tokens.text_muted);

    let mut header_left = row![
        text("System tray")
            .font(FONT_SEMIBOLD)
            .size(14)
            .color(primary_fg),
    ]
    .spacing(8)
    .align_y(Alignment::Center);

    if !icons_list.is_empty() {
        header_left = header_left.push(
            text(icons_list.len().to_string())
                .font(FONT_MEDIUM)
                .size(11)
                .color(muted_fg),
        );
    }

    let refresh_btn = button(
        text("Refresh")
            .font(FONT_MEDIUM)
            .size(11)
            .color(secondary_fg),
    )
    .padding([4, 9])
    .style(subtle_pill_button(tokens, false, tokens.r8))
    .on_press(on_refresh);

    let header = row![header_left, space::horizontal(), refresh_btn]
        .align_y(Alignment::Center)
        .width(Length::Fill)
        .padding([2, 4]);

    let divider = container(Space::new())
        .width(Length::Fill)
        .height(1)
        .style({
            let tokens = *tokens;
            move |_| hairline_divider(&tokens)
        });

    let content: Element<'a, Msg> = if icons_list.is_empty() {
        container(
            column![
                container(icons::icon(icons::LAYERS, 18.0, secondary_fg))
                    .width(40)
                    .height(40)
                    .center_x(40)
                    .center_y(40)
                    .style({
                        let tokens = *tokens;
                        move |_| container::Style {
                            background: Some(Background::Color(color(tokens.surface_subtle))),
                            border: Border {
                                radius: Radius::from(tokens.r12),
                                ..Border::default()
                            },
                            ..container::Style::default()
                        }
                    }),
                text("No tray icons right now.")
                    .font(FONT_SEMIBOLD)
                    .size(13)
                    .color(primary_fg),
                text("Background app icons will appear here.")
                    .font(FONT_REGULAR)
                    .size(11.5)
                    .color(secondary_fg),
            ]
            .spacing(6)
            .align_x(Alignment::Center),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .into()
    } else {
        let mut list = column![].spacing(ROW_SPACING).width(Length::Fill);

        for (idx, item) in icons_list.iter().enumerate() {
            let icon_box: Element<'a, Msg> = if let Some(handle) = icon_handles.get(&item.id) {
                image(handle.clone()).width(20).height(20).into()
            } else {
                container(icons::icon(icons::APP_FALLBACK, 13.0, primary_fg))
                    .width(20)
                    .height(20)
                    .center_x(20)
                    .center_y(20)
                    .style({
                        let tokens = *tokens;
                        move |_| fallback_app_tile_style(&tokens)
                    })
                    .into()
            };

            let label_col = container(
                column![
                    text(truncate_label(display_title(item), 22))
                        .font(FONT_MEDIUM)
                        .size(12.5)
                        .color(primary_fg)
                        .wrapping(text::Wrapping::None),
                    text(truncate_label(&display_subtitle(item), 26))
                        .font(FONT_REGULAR)
                        .size(10.5)
                        .color(secondary_fg)
                        .wrapping(text::Wrapping::None),
                ]
                .spacing(1)
                .width(Length::Fill),
            )
            .width(Length::Fill)
            .clip(true);

            let primary_btn = button(
                row![icon_box, label_col]
                    .spacing(9)
                    .align_y(Alignment::Center),
            )
            .width(Length::Fill)
            .padding([6, 8])
            .style(menu_row_button(tokens, false))
            .on_press(on_left_click(idx));

            let interactive_primary = mouse_area(primary_btn)
                .on_right_press(on_right_click(idx))
                .interaction(mouse::Interaction::Pointer);

            let context_btn = button(
                row![
                    menu_dot(secondary_fg),
                    menu_dot(secondary_fg),
                    menu_dot(secondary_fg),
                ]
                .spacing(2)
                .align_y(Alignment::Center),
            )
            .padding([10, 8])
            .style(subtle_pill_button(tokens, false, tokens.r8))
            .on_press(on_right_click(idx));

            list = list.push(
                row![interactive_primary, context_btn]
                    .spacing(4)
                    .align_y(Alignment::Center)
                    .width(Length::Fill),
            );
        }

        let list_pad = Padding {
            top: 0.0,
            right: if icons_list.len() > MAX_VISIBLE_ROWS {
                10.0
            } else {
                0.0
            },
            bottom: 0.0,
            left: 0.0,
        };

        scrollable(container(list).width(Length::Fill).padding(list_pad))
            .width(Length::Fill)
            .height(Length::Fill)
            .style(calm_scrollable_style(tokens))
            .into()
    };

    let body = column![header, divider, content]
        .spacing(6)
        .width(Length::Fill)
        .height(Length::Fill);

    let card = container(body)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(Padding {
            top: 12.0,
            right: 12.0,
            bottom: 10.0,
            left: 12.0,
        })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_height_grows_and_caps() {
        let empty = estimate_height(0);
        let one = estimate_height(1);
        let seven = estimate_height(7);
        let twenty = estimate_height(20);

        assert!(empty > 0.0);
        assert!(seven > one);
        assert_eq!(seven, twenty);
    }

    #[test]
    fn truncate_label_handles_unicode_safely() {
        assert_eq!(truncate_label("Discord", 10), "Discord");
        assert_eq!(truncate_label("OneDrive — Syncing files", 8), "OneDrive…");
        assert_eq!(truncate_label("日本語テストアプリ", 4), "日本語テ…");
    }
}
