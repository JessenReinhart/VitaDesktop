use crate::icons;
use crate::message::Message;
use crate::style::{
    FONT_MEDIUM, FONT_SEMIBOLD, color, dock_icon_button, dock_pill_surface,
    fallback_app_tile_style, hairline_divider, subtle_pill_button, tooltip_style,
    transparent_container,
};
use iced::border::Radius;
use iced::widget::{Space, button, column, container, image, mouse_area, row, stack, text};
use iced::{Alignment, Background, Border, Element, Length, Padding};
use std::collections::HashMap;
use vita_core::{DockItem, DockMagnifier, ShellConfig, ThemeTokens};

pub const DOCK_TOP_HEADROOM: f32 = 54.0;
pub const DOCK_SIDE_MARGIN: f32 = 48.0;

const ICON_SLOT_GAP: f32 = 6.0;

pub fn pill_height(config: &ShellConfig) -> f32 {
    config.dock.effective_pill_height()
}

pub fn bottom_margin(config: &ShellConfig) -> f32 {
    config.dock.bottom_margin.clamp(4, 24) as f32
}

pub fn window_height(config: &ShellConfig) -> f32 {
    DOCK_TOP_HEADROOM + pill_height(config) + bottom_margin(config)
}

pub fn pill_horizontal_padding(config: &ShellConfig) -> f32 {
    (pill_height(config) * 0.20).round().clamp(8.0, 14.0)
}

pub fn utility_cluster_width(config: &ShellConfig) -> f32 {
    match (config.dock.show_space_chip, config.dock.show_resume_chip) {
        (true, true) => 186.0,
        (true, false) => 102.0,
        (false, true) => 84.0,
        (false, false) => 0.0,
    }
}

pub fn compute_dock_layout(config: &ShellConfig, item_count: usize) -> (f32, Vec<f32>) {
    let base = config.dock.icon_size as f32;
    let slot_w = base + 8.0;
    let icons_width = if item_count == 0 {
        slot_w
    } else {
        item_count as f32 * slot_w + (item_count.saturating_sub(1) as f32) * ICON_SLOT_GAP
    };

    let util_w = utility_cluster_width(config);
    let utility_w = if util_w > 0.0 { util_w + 21.0 } else { 0.0 };
    let h_pad = pill_horizontal_padding(config);

    let pill_width = utility_w + icons_width + h_pad * 2.0;
    // Wide side margin accommodates horizontal spring expansion on magnification without clipping.
    let window_width = pill_width + DOCK_SIDE_MARGIN * 2.0;

    let mut centers = Vec::with_capacity(item_count);
    for i in 0..item_count {
        let cx = (i as f32) * (slot_w + ICON_SLOT_GAP) + slot_w * 0.5;
        centers.push(cx);
    }

    (window_width, centers)
}

pub fn slot_screen_center_x(
    config: &ShellConfig,
    monitor_logical_width: f32,
    item_count: usize,
    idx: usize,
) -> f32 {
    let (win_w, centers) = compute_dock_layout(config, item_count);
    let win_x = (monitor_logical_width - win_w) * 0.5;
    let util_w = utility_cluster_width(config);
    let utility_offset = if util_w > 0.0 { util_w + 21.0 } else { 0.0 };
    let slot_local_x = centers.get(idx).copied().unwrap_or(0.0);
    win_x + DOCK_SIDE_MARGIN + pill_horizontal_padding(config) + utility_offset + slot_local_x
}

#[allow(clippy::too_many_arguments)]
pub fn view<'a>(
    config: &'a ShellConfig,
    tokens: &'a ThemeTokens,
    items: &'a [DockItem],
    magnifier: &'a DockMagnifier,
    hovered_idx: Option<usize>,
    icon_handles: &'a HashMap<String, image::Handle>,
    active_space_name: &'a str,
    monitor_idx: usize,
) -> Element<'a, Message> {
    let base_size = config.dock.icon_size as f32;
    let icon_fg = color(tokens.text_primary);
    let muted_fg = color(tokens.text_secondary);

    let mut icons_row = row![].spacing(ICON_SLOT_GAP).align_y(Alignment::End);
    let mut dynamic_icons_width = 0.0_f32;

    for (idx, item) in items.iter().enumerate() {
        let scale = magnifier.scale_at(idx);
        let lift = magnifier.lift_at(idx);
        let scaled_size = (base_size * scale).round();
        let slot_w = scaled_size + 8.0;
        dynamic_icons_width += slot_w;
        if idx + 1 < items.len() {
            dynamic_icons_width += ICON_SLOT_GAP;
        }

        let graphic: Element<'a, Message> = if let Some(handle) = icon_handles.get(&item.id) {
            image(handle.clone())
                .width(scaled_size)
                .height(scaled_size)
                .into()
        } else {
            container(icons::icon(
                icons::APP_FALLBACK,
                (scaled_size * 0.54).round(),
                icon_fg,
            ))
            .width(scaled_size)
            .height(scaled_size)
            .center_x(scaled_size)
            .center_y(scaled_size)
            .style({
                let tokens = *tokens;
                move |_| fallback_app_tile_style(&tokens)
            })
            .into()
        };

        let dot: Element<'a, Message> = if item.is_focused() {
            container(Space::new())
                .width(12)
                .height(4)
                .style({
                    let dot_color = color(tokens.dock_dot_focused);
                    move |_| container::Style {
                        background: Some(Background::Color(dot_color)),
                        border: Border {
                            radius: Radius::from(2.0),
                            ..Border::default()
                        },
                        ..container::Style::default()
                    }
                })
                .into()
        } else if item.is_running() {
            container(Space::new())
                .width(5)
                .height(4)
                .style({
                    let dot_color = color(tokens.dock_dot_running);
                    move |_| container::Style {
                        background: Some(Background::Color(dot_color)),
                        border: Border {
                            radius: Radius::from(2.0),
                            ..Border::default()
                        },
                        ..container::Style::default()
                    }
                })
                .into()
        } else {
            Space::new().width(4).height(4).into()
        };

        let slot_btn = button(
            column![graphic, Space::new().height(2.0 + lift), dot].align_x(Alignment::Center),
        )
        .padding([0, 4])
        .style(dock_icon_button())
        .on_press(Message::DockItemClicked(monitor_idx, idx));

        icons_row = icons_row.push(slot_btn);
    }

    if items.is_empty() {
        dynamic_icons_width = base_size + 8.0;
    }

    let interactive_icons = mouse_area(icons_row)
        .on_enter(Message::DockPointerEntered(monitor_idx))
        .on_move(move |pt| Message::DockPointerMoved(monitor_idx, pt))
        .on_exit(Message::DockPointerExited(monitor_idx))
        .on_right_press(Message::DockRightClicked(monitor_idx));

    let pill_h = pill_height(config);
    let bottom_gap = bottom_margin(config);
    let h_pad = pill_horizontal_padding(config);
    let util_w = utility_cluster_width(config);
    let icon_bottom_pad = ((pill_h - (base_size + 6.0)) * 0.5).floor().clamp(1.0, 16.0);

    let has_utility = util_w > 0.0;
    let mut pill_inner = row![].spacing(10).align_y(Alignment::End);

    if has_utility {
        let mut utility_row = row![].spacing(6).align_y(Alignment::Center);
        let chip_pad = if pill_h < 54.0 { [4, 8] } else { [6, 10] };
        let resume_pad = if pill_h < 54.0 { [4, 8] } else { [8, 10] };

        if config.dock.show_space_chip {
            let space_btn = button(
                row![
                    icons::icon(icons::LAYERS, 15.0, icon_fg),
                    column![
                        text("SPACE")
                            .font(FONT_SEMIBOLD)
                            .size(9)
                            .color(muted_fg),
                        text(active_space_name)
                            .font(FONT_MEDIUM)
                            .size(12)
                            .color(icon_fg),
                    ]
                    .spacing(0),
                ]
                .spacing(7)
                .align_y(Alignment::Center),
            )
            .padding(chip_pad)
            .style(subtle_pill_button(tokens, false, tokens.r12))
            .on_press(Message::ToggleSpacesOn(monitor_idx));
            utility_row = utility_row.push(space_btn);
        }

        if config.dock.show_resume_chip {
            let resume_btn = button(
                row![
                    icons::icon(icons::CLOCK, 14.0, icon_fg),
                    text("Resume").font(FONT_MEDIUM).size(12).color(icon_fg),
                ]
                .spacing(6)
                .align_y(Alignment::Center),
            )
            .padding(resume_pad)
            .style(subtle_pill_button(tokens, false, tokens.r12))
            .on_press(Message::ToggleLauncherOn(monitor_idx));
            utility_row = utility_row.push(resume_btn);
        }

        let box_h = (pill_h - icon_bottom_pad * 2.0).max(28.0);
        let utility_box = container(utility_row)
            .width(util_w)
            .height(box_h)
            .align_y(Alignment::Center);

        pill_inner = pill_inner.push(utility_box);
        pill_inner = pill_inner.push(
            container(
                container(Space::new())
                    .width(1)
                    .height((pill_h * 0.45).clamp(18.0, 28.0))
                    .style({
                        let tokens = *tokens;
                        move |_| hairline_divider(&tokens)
                    }),
            )
            .height(box_h)
            .align_y(Alignment::Center),
        );
    }

    let utility_total = if has_utility { util_w + 21.0 } else { 0.0 };
    let dynamic_pill_width = utility_total + dynamic_icons_width + h_pad * 2.0;

    pill_inner = pill_inner.push(interactive_icons);

    // Layer 0: rounded pill surface aligned cleanly at the bottom.
    // Layer 1: unclipped bottom-aligned icon row that smoothly rises above the pill top edge on hover.
    let pill_backdrop = container(
        container(Space::new().width(dynamic_pill_width).height(pill_h)).style({
            let tokens = *tokens;
            let dock_cfg = config.dock.clone();
            move |_| dock_pill_surface(&tokens, &dock_cfg)
        }),
    )
    .width(Length::Fill)
    .height(pill_h + DOCK_TOP_HEADROOM)
    .center_x(Length::Fill)
    .align_y(iced::alignment::Vertical::Bottom);

    let foreground_icons = container(pill_inner)
        .width(Length::Fill)
        .height(pill_h + DOCK_TOP_HEADROOM)
        .center_x(Length::Fill)
        .align_y(iced::alignment::Vertical::Bottom)
        .padding(Padding {
            top: 0.0,
            right: h_pad,
            bottom: icon_bottom_pad,
            left: h_pad,
        });

    let tooltip_overlay: Element<'a, Message> = if let Some(idx) = hovered_idx
        && let Some(item) = items.get(idx)
    {
        let label = if item.windows.len() > 1 {
            format!("{} ({})", item.label, item.windows.len())
        } else {
            item.label.clone()
        };
        container(
            container(text(label).font(FONT_MEDIUM).size(12))
                .padding([4, 10])
                .style({
                    let tokens = *tokens;
                    move |_| tooltip_style(&tokens)
                }),
        )
        .width(Length::Fill)
        .height(DOCK_TOP_HEADROOM)
        .center_x(Length::Fill)
        .align_y(iced::alignment::Vertical::Top)
        .padding(Padding {
            top: 2.0,
            ..Padding::ZERO
        })
        .into()
    } else {
        Space::new().height(DOCK_TOP_HEADROOM).into()
    };

    let dock_stack = stack![pill_backdrop, foreground_icons, tooltip_overlay];

    container(column![dock_stack, Space::new().height(bottom_gap)].align_x(Alignment::Center))
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .align_y(iced::alignment::Vertical::Bottom)
        .style(|_| transparent_container())
        .into()
}
