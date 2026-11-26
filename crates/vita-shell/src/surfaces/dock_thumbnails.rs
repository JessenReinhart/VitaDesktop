use crate::icons;
use crate::style::{
    FONT_MEDIUM, FONT_REGULAR, FONT_SEMIBOLD, color, popover_card_surface, subtle_pill_button,
    transparent_container,
};
use iced::border::Radius;
use iced::widget::{Space, button, column, container, image, row, stack, text};
use iced::{
    Alignment, Background, Border, Color, ContentFit, Element, Length, Padding, Shadow, Vector,
};
use std::collections::HashMap;
use vita_core::{DockItem, LogicalRect, PhysicalRect, RunningWindow, SurfaceMaterial, ThemeTokens};

pub const MAX_PREVIEW_WINDOWS: usize = 8;
pub const MAX_PREVIEW_COLS: usize = 4;
pub const SHADOW_GUTTER: f32 = 16.0;
pub const POPOVER_PADDING: f32 = 12.0;
pub const CARD_WIDTH: f32 = 204.0;
pub const PREVIEW_WIDTH: f32 = 188.0;
pub const PREVIEW_HEIGHT: f32 = 112.0;
pub const CARD_HEADER_HEIGHT: f32 = 28.0;
pub const CARD_HEADER_GAP: f32 = 4.0;
pub const CARD_PAD_TOP: f32 = 4.0;
pub const CARD_PAD_SIDE: f32 = 8.0;
pub const CARD_PAD_BOTTOM: f32 = 8.0;
pub const CARD_HEIGHT: f32 =
    CARD_PAD_TOP + CARD_HEADER_HEIGHT + CARD_HEADER_GAP + PREVIEW_HEIGHT + CARD_PAD_BOTTOM;
pub const COL_GAP: f32 = 12.0;
pub const ROW_GAP: f32 = 12.0;

#[derive(Debug, Clone, PartialEq)]
pub struct PreviewSlotGeometry {
    pub hwnd: isize,
    pub logical_rect: LogicalRect,
    pub physical_rect: PhysicalRect,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PreviewStripLayout {
    pub window_width: f32,
    pub window_height: f32,
    pub slots: Vec<PreviewSlotGeometry>,
}

/// Computes deterministic logical and physical preview slot bounds for up to 8 windows
/// (up to 4 cards per row, 2 rows max) so Iced widget positions match `DwmThumbnailHost::sync_slots`
/// pixel-for-pixel across DPI scales.
pub fn compute_preview_layout(windows: &[RunningWindow], dpi_scale: f32) -> PreviewStripLayout {
    let scale = dpi_scale.max(0.5);
    let visible_count = windows.len().min(MAX_PREVIEW_WINDOWS);
    if visible_count == 0 {
        let popover_w = POPOVER_PADDING * 2.0 + CARD_WIDTH;
        let popover_h = POPOVER_PADDING * 2.0 + CARD_HEIGHT;
        return PreviewStripLayout {
            window_width: SHADOW_GUTTER * 2.0 + popover_w,
            window_height: SHADOW_GUTTER * 2.0 + popover_h,
            slots: Vec::new(),
        };
    }

    let cols = visible_count.min(MAX_PREVIEW_COLS);
    let rows = visible_count.div_ceil(MAX_PREVIEW_COLS);
    let grid_w = (cols as f32) * CARD_WIDTH + (cols.saturating_sub(1) as f32) * COL_GAP;
    let grid_h = (rows as f32) * CARD_HEIGHT + (rows.saturating_sub(1) as f32) * ROW_GAP;
    let popover_w = POPOVER_PADDING * 2.0 + grid_w;
    let popover_h = POPOVER_PADDING * 2.0 + grid_h;

    let mut slots = Vec::with_capacity(visible_count);
    for (idx, win) in windows.iter().take(visible_count).enumerate() {
        let row_idx = idx / MAX_PREVIEW_COLS;
        let col_idx = idx % MAX_PREVIEW_COLS;
        let card_x = SHADOW_GUTTER + POPOVER_PADDING + (col_idx as f32) * (CARD_WIDTH + COL_GAP);
        let card_y = SHADOW_GUTTER + POPOVER_PADDING + (row_idx as f32) * (CARD_HEIGHT + ROW_GAP);
        let logical_rect = LogicalRect {
            x: card_x + CARD_PAD_SIDE,
            y: card_y + CARD_PAD_TOP + CARD_HEADER_HEIGHT + CARD_HEADER_GAP,
            width: PREVIEW_WIDTH,
            height: PREVIEW_HEIGHT,
        };
        slots.push(PreviewSlotGeometry {
            hwnd: win.hwnd,
            logical_rect,
            physical_rect: logical_rect.to_physical(scale),
        });
    }

    PreviewStripLayout {
        window_width: SHADOW_GUTTER * 2.0 + popover_w,
        window_height: SHADOW_GUTTER * 2.0 + popover_h,
        slots,
    }
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

fn surface_fill_color(tokens: &ThemeTokens) -> Color {
    match tokens.material {
        SurfaceMaterial::Matte => color(tokens.surface_base),
        SurfaceMaterial::Frosted => color(tokens.surface_dock.with_alpha(0.92)),
    }
}

fn solid_band_style(bg: Color, radius: Radius) -> container::Style {
    container::Style {
        background: Some(Background::Color(bg)),
        border: Border {
            radius,
            ..Border::default()
        },
        snap: true,
        ..container::Style::default()
    }
}

fn preview_viewport_button(
    tokens: &ThemeTokens,
) -> impl Fn(&iced::Theme, button::Status) -> button::Style {
    let tokens = *tokens;
    move |_theme, status| {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: None,
            text_color: color(tokens.text_primary),
            border: Border {
                color: if hovered {
                    color(tokens.accent_primary)
                } else {
                    Color::TRANSPARENT
                },
                width: if hovered { 1.5 } else { 0.0 },
                radius: Radius::from(tokens.r8),
            },
            shadow: Shadow::default(),
            snap: true,
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn view<'a, Msg: Clone + 'a>(
    tokens: &'a ThemeTokens,
    item: &'a DockItem,
    layout: &'a PreviewStripLayout,
    icon_handles: &'a HashMap<String, image::Handle>,
    fallback_thumbs: &'a HashMap<isize, image::Handle>,
    live_dwm_active: bool,
    on_focus: impl Fn(isize) -> Msg + 'a,
    on_close: impl Fn(isize) -> Msg + 'a,
) -> Element<'a, Msg> {
    let popover_w =
        (layout.window_width - SHADOW_GUTTER * 2.0).max(POPOVER_PADDING * 2.0 + CARD_WIDTH);
    let popover_h =
        (layout.window_height - SHADOW_GUTTER * 2.0).max(POPOVER_PADDING * 2.0 + CARD_HEIGHT);

    let card_layer: Element<'a, Msg> = if layout.slots.is_empty() {
        container(
            text("No open windows")
                .font(FONT_MEDIUM)
                .size(12)
                .color(color(tokens.text_secondary)),
        )
        .width(popover_w)
        .height(popover_h)
        .center_x(popover_w)
        .center_y(popover_h)
        .style({
            let tokens = *tokens;
            move |_| popover_card_surface(&tokens, tokens.r16)
        })
        .into()
    } else {
        let slot_windows: Vec<(isize, Option<&RunningWindow>)> = layout
            .slots
            .iter()
            .enumerate()
            .map(|(idx, slot)| {
                let win = item
                    .windows
                    .get(idx)
                    .filter(|w| w.hwnd == slot.hwnd)
                    .or_else(|| item.windows.iter().find(|w| w.hwnd == slot.hwnd));
                (slot.hwnd, win)
            })
            .collect();

        let cols = slot_windows.len().min(MAX_PREVIEW_COLS);
        let any_live_cutout = live_dwm_active
            && slot_windows
                .iter()
                .any(|(_, w)| w.is_some_and(|win| !win.is_minimized));

        let mut grid_col = column![].spacing(ROW_GAP);
        for row_slice in slot_windows.chunks(MAX_PREVIEW_COLS) {
            let mut row_widgets = row![].spacing(COL_GAP);
            for &(slot_hwnd, win_opt) in row_slice {
                let card = build_preview_card(
                    tokens,
                    item,
                    slot_hwnd,
                    win_opt,
                    icon_handles,
                    fallback_thumbs,
                    live_dwm_active,
                    &on_focus,
                    &on_close,
                );
                row_widgets = row_widgets.push(card);
            }
            grid_col = grid_col.push(row_widgets);
        }

        if any_live_cutout {
            let backdrop = build_cutout_backdrop(tokens, &slot_windows, cols, popover_w, popover_h);
            let foreground = container(grid_col)
                .width(popover_w)
                .height(popover_h)
                .padding(POPOVER_PADDING)
                .style(|_| transparent_container());
            let border_overlay = container(Space::new().width(Length::Fill).height(Length::Fill))
                .width(popover_w)
                .height(popover_h)
                .style({
                    let tokens = *tokens;
                    move |_| container::Style {
                        background: None,
                        border: Border {
                            color: color(tokens.border_control),
                            width: 1.0,
                            radius: Radius::from(tokens.r16),
                        },
                        ..container::Style::default()
                    }
                });

            stack![backdrop, foreground, border_overlay]
                .width(popover_w)
                .height(popover_h)
                .into()
        } else {
            container(grid_col)
                .width(popover_w)
                .height(popover_h)
                .padding(POPOVER_PADDING)
                .style({
                    let tokens = *tokens;
                    move |_| popover_card_surface(&tokens, tokens.r16)
                })
                .into()
        }
    };

    container(card_layer)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(SHADOW_GUTTER)
        .style(|_| transparent_container())
        .into()
}

/// Paints the `popover_card_surface` backdrop around non-minimized DWM preview rectangles so the
/// DX12 DirectComposition swapchain keeps `alpha = 0.0` over each live thumbnail slot.
fn build_cutout_backdrop<'a, Msg: 'a>(
    tokens: &'a ThemeTokens,
    slot_windows: &[(isize, Option<&RunningWindow>)],
    cols: usize,
    popover_w: f32,
    popover_h: f32,
) -> Element<'a, Msg> {
    let card_bg = surface_fill_color(tokens);
    let r16 = tokens.r16;
    let top_band_h = POPOVER_PADDING + CARD_PAD_TOP + CARD_HEADER_HEIGHT + CARD_HEADER_GAP;
    let mid_band_h =
        CARD_PAD_BOTTOM + ROW_GAP + CARD_PAD_TOP + CARD_HEADER_HEIGHT + CARD_HEADER_GAP;
    let bottom_band_h = CARD_PAD_BOTTOM + POPOVER_PADDING;
    let left_margin_w = POPOVER_PADDING + CARD_PAD_SIDE;
    let col_gap_strip_w = CARD_PAD_SIDE + COL_GAP + CARD_PAD_SIDE;

    let mut mask_col = column![
        container(Space::new())
            .width(popover_w)
            .height(top_band_h)
            .style(move |_| solid_band_style(
                card_bg,
                Radius {
                    top_left: r16,
                    top_right: r16,
                    bottom_left: 0.0,
                    bottom_right: 0.0,
                },
            )),
    ];

    for (r_idx, row_slice) in slot_windows.chunks(MAX_PREVIEW_COLS).enumerate() {
        if r_idx > 0 {
            mask_col = mask_col.push(
                container(Space::new())
                    .width(popover_w)
                    .height(mid_band_h)
                    .style(move |_| solid_band_style(card_bg, Radius::from(0.0))),
            );
        }

        let mut band_row = row![
            container(Space::new())
                .width(left_margin_w)
                .height(PREVIEW_HEIGHT)
                .style(move |_| solid_band_style(card_bg, Radius::from(0.0))),
        ];

        for (c_idx, &(_, win_opt)) in row_slice.iter().enumerate() {
            if c_idx > 0 {
                band_row = band_row.push(
                    container(Space::new())
                        .width(col_gap_strip_w)
                        .height(PREVIEW_HEIGHT)
                        .style(move |_| solid_band_style(card_bg, Radius::from(0.0))),
                );
            }
            if win_opt.is_some_and(|w| !w.is_minimized) {
                band_row = band_row.push(Space::new().width(PREVIEW_WIDTH).height(PREVIEW_HEIGHT));
            } else {
                band_row = band_row.push(
                    container(Space::new())
                        .width(PREVIEW_WIDTH)
                        .height(PREVIEW_HEIGHT)
                        .style(move |_| solid_band_style(card_bg, Radius::from(0.0))),
                );
            }
        }

        let trailing_cols = cols.saturating_sub(row_slice.len());
        let right_margin_w =
            CARD_PAD_SIDE + POPOVER_PADDING + (trailing_cols as f32) * (CARD_WIDTH + COL_GAP);
        band_row = band_row.push(
            container(Space::new())
                .width(right_margin_w)
                .height(PREVIEW_HEIGHT)
                .style(move |_| solid_band_style(card_bg, Radius::from(0.0))),
        );

        mask_col = mask_col.push(band_row);
    }

    mask_col = mask_col.push(
        container(Space::new())
            .width(popover_w)
            .height(bottom_band_h)
            .style(move |_| {
                solid_band_style(
                    card_bg,
                    Radius {
                        top_left: 0.0,
                        top_right: 0.0,
                        bottom_left: r16,
                        bottom_right: r16,
                    },
                )
            }),
    );

    container(mask_col)
        .width(popover_w)
        .height(popover_h)
        .style({
            let tokens = *tokens;
            move |_| container::Style {
                background: None,
                text_color: Some(color(tokens.text_primary)),
                border: Border {
                    color: Color::TRANSPARENT,
                    width: 0.0,
                    radius: Radius::from(tokens.r16),
                },
                shadow: Shadow {
                    color: color(tokens.shadow_color),
                    offset: Vector::new(0.0, 10.0),
                    blur_radius: 26.0,
                },
                snap: true,
            }
        })
        .into()
}

#[allow(clippy::too_many_arguments)]
fn build_preview_card<'a, Msg: Clone + 'a>(
    tokens: &'a ThemeTokens,
    item: &'a DockItem,
    slot_hwnd: isize,
    win_opt: Option<&'a RunningWindow>,
    icon_handles: &'a HashMap<String, image::Handle>,
    fallback_thumbs: &'a HashMap<isize, image::Handle>,
    live_dwm_active: bool,
    on_focus: &impl Fn(isize) -> Msg,
    on_close: &impl Fn(isize) -> Msg,
) -> Element<'a, Msg> {
    let Some(win) = win_opt else {
        return container(Space::new())
            .width(CARD_WIDTH)
            .height(CARD_HEIGHT)
            .style(|_| transparent_container())
            .into();
    };

    let primary_fg = color(tokens.text_primary);
    let secondary_fg = color(tokens.text_secondary);
    let is_focused = win.is_focused;
    let is_minimized = win.is_minimized;
    let is_live_cutout = live_dwm_active && !is_minimized;

    let app_icon_small: Element<'a, Msg> = if let Some(handle) = icon_handles.get(&item.id) {
        image(handle.clone()).width(14).height(14).into()
    } else {
        icons::icon(icons::WINDOW, 13.0, secondary_fg).into()
    };

    let raw_title = if win.title.trim().is_empty() {
        item.label.as_str()
    } else {
        win.title.as_str()
    };
    let title_box = container(
        text(truncate_title(raw_title, 18))
            .font(if is_focused {
                FONT_SEMIBOLD
            } else {
                FONT_MEDIUM
            })
            .size(12)
            .color(primary_fg)
            .wrapping(text::Wrapping::None),
    )
    .width(Length::Fill)
    .clip(true);

    let title_btn = button(
        row![app_icon_small, title_box]
            .spacing(6)
            .align_y(Alignment::Center),
    )
    .width(Length::Fill)
    .height(CARD_HEADER_HEIGHT)
    .padding([0, 4])
    .style(subtle_pill_button(tokens, false, tokens.r8))
    .on_press(on_focus(slot_hwnd));

    let close_btn = button(
        container(icons::icon(icons::CLOSE, 11.0, secondary_fg))
            .width(22)
            .height(22)
            .center_x(22)
            .center_y(22),
    )
    .width(22)
    .height(22)
    .padding(0)
    .style(subtle_pill_button(tokens, false, tokens.r8))
    .on_press(on_close(slot_hwnd));

    let header_row = row![title_btn, close_btn]
        .spacing(4)
        .width(PREVIEW_WIDTH)
        .height(CARD_HEADER_HEIGHT)
        .align_y(Alignment::Center);

    let viewport_body: Element<'a, Msg> = if is_live_cutout {
        container(Space::new().width(PREVIEW_WIDTH).height(PREVIEW_HEIGHT))
            .width(PREVIEW_WIDTH)
            .height(PREVIEW_HEIGHT)
            .style(|_| container::Style {
                background: Some(Background::Color(Color::TRANSPARENT)),
                ..container::Style::default()
            })
            .into()
    } else {
        build_fallback_or_minimized_viewport(
            tokens,
            item,
            win,
            icon_handles,
            fallback_thumbs.get(&win.hwnd),
        )
    };

    let preview_btn = button(viewport_body)
        .width(PREVIEW_WIDTH)
        .height(PREVIEW_HEIGHT)
        .padding(0)
        .style(preview_viewport_button(tokens))
        .on_press(on_focus(slot_hwnd));

    let card_content = column![header_row, preview_btn].spacing(CARD_HEADER_GAP);

    container(card_content)
        .width(CARD_WIDTH)
        .height(CARD_HEIGHT)
        .padding(Padding {
            top: CARD_PAD_TOP,
            right: CARD_PAD_SIDE,
            bottom: CARD_PAD_BOTTOM,
            left: CARD_PAD_SIDE,
        })
        .style({
            let tokens = *tokens;
            move |_| container::Style {
                background: if is_live_cutout {
                    None
                } else {
                    Some(Background::Color(color(if is_focused {
                        tokens.accent_soft
                    } else {
                        tokens.surface_elevated
                    })))
                },
                text_color: Some(color(tokens.text_primary)),
                border: Border {
                    color: color(if is_focused {
                        tokens.accent_primary
                    } else {
                        tokens.border_control
                    }),
                    width: if is_focused { 1.5 } else { 1.0 },
                    radius: Radius::from(tokens.r12),
                },
                snap: true,
                ..container::Style::default()
            }
        })
        .into()
}

fn build_fallback_or_minimized_viewport<'a, Msg: 'a>(
    tokens: &'a ThemeTokens,
    item: &'a DockItem,
    win: &'a RunningWindow,
    icon_handles: &'a HashMap<String, image::Handle>,
    fallback_thumb: Option<&'a image::Handle>,
) -> Element<'a, Msg> {
    let secondary_fg = color(tokens.text_secondary);
    let badge_fg = color(tokens.text_primary);
    let make_minimized_badge = || {
        container(
            text("Minimized")
                .font(FONT_SEMIBOLD)
                .size(11)
                .color(badge_fg),
        )
        .padding([3, 10])
        .style({
            let tokens = *tokens;
            move |_| container::Style {
                background: Some(Background::Color(color(
                    tokens.surface_base.with_alpha(0.92),
                ))),
                border: Border {
                    color: color(tokens.border_control),
                    width: 1.0,
                    radius: Radius::from(tokens.pill),
                },
                snap: true,
                ..container::Style::default()
            }
        })
    };

    if let Some(handle) = fallback_thumb {
        let thumb_layer: Element<'a, Msg> = container(
            image(handle.clone())
                .width(PREVIEW_WIDTH)
                .height(PREVIEW_HEIGHT)
                .content_fit(ContentFit::Contain)
                .border_radius(tokens.r8),
        )
        .width(PREVIEW_WIDTH)
        .height(PREVIEW_HEIGHT)
        .center_x(PREVIEW_WIDTH)
        .center_y(PREVIEW_HEIGHT)
        .clip(true)
        .style({
            let tokens = *tokens;
            move |_| container::Style {
                background: Some(Background::Color(color(tokens.surface_subtle))),
                border: Border {
                    color: color(tokens.border_subtle),
                    width: 1.0,
                    radius: Radius::from(tokens.r8),
                },
                snap: true,
                ..container::Style::default()
            }
        })
        .into();

        if win.is_minimized {
            let badge = container(make_minimized_badge())
                .width(PREVIEW_WIDTH)
                .height(PREVIEW_HEIGHT)
                .center_x(PREVIEW_WIDTH)
                .center_y(PREVIEW_HEIGHT);

            stack![thumb_layer, badge]
                .width(PREVIEW_WIDTH)
                .height(PREVIEW_HEIGHT)
                .into()
        } else {
            thumb_layer
        }
    } else {
        let icon_el: Element<'a, Msg> = if let Some(handle) = icon_handles.get(&item.id) {
            image(handle.clone()).width(28).height(28).into()
        } else {
            icons::icon(icons::WINDOW, 24.0, secondary_fg).into()
        };

        let body: Element<'a, Msg> = if win.is_minimized {
            column![icon_el, make_minimized_badge()]
                .spacing(8)
                .align_x(Alignment::Center)
                .into()
        } else {
            column![
                icon_el,
                text("Preview unavailable")
                    .font(FONT_REGULAR)
                    .size(11)
                    .color(secondary_fg),
            ]
            .spacing(6)
            .align_x(Alignment::Center)
            .into()
        };

        container(body)
            .width(PREVIEW_WIDTH)
            .height(PREVIEW_HEIGHT)
            .center_x(PREVIEW_WIDTH)
            .center_y(PREVIEW_HEIGHT)
            .style({
                let tokens = *tokens;
                move |_| container::Style {
                    background: Some(Background::Color(color(tokens.surface_subtle))),
                    border: Border {
                        color: color(tokens.border_subtle),
                        width: 1.0,
                        radius: Radius::from(tokens.r8),
                    },
                    snap: true,
                    ..container::Style::default()
                }
            })
            .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_window(hwnd: isize) -> RunningWindow {
        RunningWindow {
            hwnd,
            title: format!("Window {hwnd}"),
            exe_name: "app.exe".to_string(),
            exe_path: String::new(),
            pid: 100,
            is_focused: hwnd == 1,
            is_minimized: false,
            is_maximized: false,
            frame_rect: PhysicalRect::default(),
            monitor_idx: 0,
            icon: None,
        }
    }

    #[test]
    fn compute_preview_layout_produces_non_overlapping_inbounds_slots() {
        for count in [1_usize, 2, 4, 6] {
            let windows: Vec<RunningWindow> =
                (1..=count).map(|idx| sample_window(idx as isize)).collect();

            for dpi in [1.0_f32, 1.5_f32] {
                let layout = compute_preview_layout(&windows, dpi);
                assert_eq!(layout.slots.len(), count);
                assert!(layout.window_width > SHADOW_GUTTER * 2.0);
                assert!(layout.window_height > SHADOW_GUTTER * 2.0);

                let phys_win_w = (layout.window_width * dpi).round() as i32;
                let phys_win_h = (layout.window_height * dpi).round() as i32;

                for (i, slot) in layout.slots.iter().enumerate() {
                    assert_eq!(slot.hwnd, (i + 1) as isize);
                    assert_eq!(slot.logical_rect.width, PREVIEW_WIDTH);
                    assert_eq!(slot.logical_rect.height, PREVIEW_HEIGHT);
                    assert!(slot.logical_rect.x >= SHADOW_GUTTER + POPOVER_PADDING);
                    assert!(slot.logical_rect.y >= SHADOW_GUTTER + POPOVER_PADDING);
                    assert!(
                        slot.logical_rect.x + slot.logical_rect.width
                            <= layout.window_width - SHADOW_GUTTER - POPOVER_PADDING
                    );
                    assert!(
                        slot.logical_rect.y + slot.logical_rect.height
                            <= layout.window_height - SHADOW_GUTTER - POPOVER_PADDING
                    );

                    assert_eq!(slot.physical_rect, slot.logical_rect.to_physical(dpi));
                    assert!(slot.physical_rect.width > 0);
                    assert!(slot.physical_rect.height > 0);
                    assert!(slot.physical_rect.x >= 0);
                    assert!(slot.physical_rect.y >= 0);
                    assert!(slot.physical_rect.right() <= phys_win_w);
                    assert!(slot.physical_rect.bottom() <= phys_win_h);

                    for (j, other) in layout.slots.iter().enumerate() {
                        if i != j {
                            assert!(
                                !slot.physical_rect.intersects(other.physical_rect),
                                "slots {i} and {j} overlapped at dpi {dpi}: {:?} vs {:?}",
                                slot.physical_rect,
                                other.physical_rect
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn compute_preview_layout_handles_empty_and_caps_at_eight() {
        let empty = compute_preview_layout(&[], 1.0);
        assert!(empty.slots.is_empty());
        assert!(empty.window_width > 0.0);
        assert!(empty.window_height > 0.0);

        let ten: Vec<RunningWindow> = (1..=10).map(|idx| sample_window(idx as isize)).collect();
        let capped = compute_preview_layout(&ten, 1.25);
        assert_eq!(capped.slots.len(), MAX_PREVIEW_WINDOWS);
    }
}
