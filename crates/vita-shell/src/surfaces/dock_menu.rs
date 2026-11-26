use crate::icons;
use crate::message::Message;
use crate::style::{
    FONT_MEDIUM, FONT_REGULAR, FONT_SEMIBOLD, color, fallback_app_tile_style, hairline_divider,
    menu_row_button, popover_card_surface, subtle_pill_button, transparent_container,
};
use iced::border::Radius;
use iced::widget::{Space, button, column, container, image, row, space, text};
use iced::{Alignment, Background, Border, Element, Length, Padding};
use std::collections::HashMap;
use vita_core::{DockItem, ThemeTokens};

pub const DOCK_POPOVER_WIDTH: f32 = 316.0;
const SHADOW_GUTTER: f32 = 16.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockPopoverMode {
    ContextMenu,
    WindowPicker,
}

#[derive(Debug, Clone)]
pub struct DockPopoverTarget {
    pub dock_item_id: String,
    pub anchor_center_x_logical: f32,
    pub mode: DockPopoverMode,
}

pub fn estimate_popover_height(item: &DockItem, mode: DockPopoverMode) -> f32 {
    let body_h = match mode {
        DockPopoverMode::ContextMenu => {
            let mut rows = 2; // Launch/New window + Pin/Unpin
            if item.windows.len() > 1 {
                rows += 1; // Choose a window
            }
            if !item.windows.is_empty() {
                rows += 1; // Close window(s)
            }
            58.0 + (rows as f32) * 38.0 + 18.0
        }
        DockPopoverMode::WindowPicker => {
            let count = item.windows.len().clamp(1, 6);
            62.0 + (count as f32) * 42.0 + 16.0
        }
    };
    body_h + SHADOW_GUTTER * 2.0
}

fn truncate_title(title: &str, max_chars: usize) -> String {
    let mut chars = title.chars();
    let head: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{}…", head.trim_end())
    } else {
        head
    }
}

pub fn view<'a>(
    tokens: &'a ThemeTokens,
    item: &'a DockItem,
    mode: DockPopoverMode,
    icon_handles: &'a HashMap<String, image::Handle>,
) -> Element<'a, Message> {
    let card_body = match mode {
        DockPopoverMode::ContextMenu => view_context_menu(tokens, item),
        DockPopoverMode::WindowPicker => view_window_picker(tokens, item, icon_handles),
    };

    let card = container(card_body)
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

fn view_context_menu<'a>(tokens: &'a ThemeTokens, item: &'a DockItem) -> Element<'a, Message> {
    let primary_fg = color(tokens.text_primary);
    let secondary_fg = color(tokens.text_secondary);
    let danger_fg = color(tokens.status_error);

    let subtitle = match (item.windows.len(), item.is_pinned) {
        (0, _) => "Pinned app".to_string(),
        (1, _) => "1 open window".to_string(),
        (n, _) => format!("{n} open windows"),
    };

    let header = column![
        text(&item.label)
            .font(FONT_SEMIBOLD)
            .size(13)
            .color(primary_fg),
        text(subtitle)
            .font(FONT_REGULAR)
            .size(11)
            .color(secondary_fg),
    ]
    .spacing(1)
    .padding([2, 6]);

    let mut list = column![
        header,
        container(Space::new())
            .width(Length::Fill)
            .height(1)
            .style({
                let tokens = *tokens;
                move |_| hairline_divider(&tokens)
            }),
    ]
    .spacing(6);

    let launch_label = if item.is_running() {
        "Open new window"
    } else {
        "Launch"
    };
    list = list.push(
        button(
            row![
                icons::icon(icons::PLUS, 14.0, primary_fg),
                text(launch_label)
                    .font(FONT_MEDIUM)
                    .size(12)
                    .color(primary_fg),
            ]
            .spacing(10)
            .align_y(Alignment::Center),
        )
        .width(Length::Fill)
        .padding([8, 8])
        .style(menu_row_button(tokens, false))
        .on_press(Message::DockMenuLaunchNew(item.id.clone())),
    );

    if item.windows.len() > 1 {
        list = list.push(
            button(
                row![
                    icons::icon(icons::WINDOW, 14.0, primary_fg),
                    text("Choose a window")
                        .font(FONT_MEDIUM)
                        .size(12)
                        .color(primary_fg),
                ]
                .spacing(10)
                .align_y(Alignment::Center),
            )
            .width(Length::Fill)
            .padding([8, 8])
            .style(menu_row_button(tokens, false))
            .on_press(Message::DockMenuSwitchToPicker),
        );
    }

    let (pin_icon, pin_text) = if item.is_pinned {
        (icons::PIN_OFF, "Unpin from dock")
    } else {
        (icons::PIN, "Pin to dock")
    };
    list = list.push(
        button(
            row![
                icons::icon(pin_icon, 14.0, primary_fg),
                text(pin_text).font(FONT_MEDIUM).size(12).color(primary_fg),
            ]
            .spacing(10)
            .align_y(Alignment::Center),
        )
        .width(Length::Fill)
        .padding([8, 8])
        .style(menu_row_button(tokens, false))
        .on_press(Message::DockMenuTogglePin(item.id.clone())),
    );

    if !item.windows.is_empty() {
        let close_label = if item.windows.len() > 1 {
            format!("Close all ({})", item.windows.len())
        } else {
            "Close window".to_string()
        };
        list = list.push(
            button(
                row![
                    icons::icon(icons::CLOSE, 14.0, danger_fg),
                    text(close_label)
                        .font(FONT_MEDIUM)
                        .size(12)
                        .color(danger_fg),
                ]
                .spacing(10)
                .align_y(Alignment::Center),
            )
            .width(Length::Fill)
            .padding([8, 8])
            .style(menu_row_button(tokens, true))
            .on_press(Message::DockMenuCloseAll(item.id.clone())),
        );
    }

    list.into()
}

fn view_window_picker<'a>(
    tokens: &'a ThemeTokens,
    item: &'a DockItem,
    icon_handles: &'a HashMap<String, image::Handle>,
) -> Element<'a, Message> {
    let primary_fg = color(tokens.text_primary);
    let secondary_fg = color(tokens.text_secondary);

    let header = column![
        text("Your open windows")
            .font(FONT_SEMIBOLD)
            .size(13)
            .color(primary_fg),
        text(&item.label)
            .font(FONT_REGULAR)
            .size(11)
            .color(secondary_fg),
    ]
    .spacing(1)
    .padding([2, 6]);

    let mut list = column![
        header,
        container(Space::new())
            .width(Length::Fill)
            .height(1)
            .style({
                let tokens = *tokens;
                move |_| hairline_divider(&tokens)
            }),
    ]
    .spacing(6);

    for win in item.windows.iter().take(6) {
        let thumb: Element<'a, Message> = if let Some(handle) = icon_handles.get(&item.id) {
            image(handle.clone()).width(20).height(20).into()
        } else {
            container(icons::icon(icons::WINDOW, 13.0, primary_fg))
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

        let active_badge: Element<'a, Message> = if win.is_focused {
            let dot_color = color(tokens.dock_dot_focused);
            container(Space::new())
                .width(6)
                .height(6)
                .style(move |_| container::Style {
                    background: Some(Background::Color(dot_color)),
                    border: Border {
                        radius: Radius::from(3.0),
                        ..Border::default()
                    },
                    ..container::Style::default()
                })
                .into()
        } else {
            Space::new().width(6).into()
        };

        let focus_row = button(
            row![
                thumb,
                text(truncate_title(&win.title, 26))
                    .font(FONT_MEDIUM)
                    .size(12)
                    .color(primary_fg),
                space::horizontal(),
                active_badge,
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        )
        .width(Length::Fill)
        .padding([7, 8])
        .style(menu_row_button(tokens, false))
        .on_press(Message::FocusSpecificWindow(win.hwnd));

        let close_btn = button(icons::icon(icons::CLOSE, 12.0, secondary_fg))
            .padding(6)
            .style(subtle_pill_button(tokens, false, tokens.r8))
            .on_press(Message::CloseSpecificWindow(win.hwnd));

        list = list.push(
            row![focus_row, close_btn]
                .spacing(4)
                .align_y(Alignment::Center),
        );
    }

    list.into()
}
