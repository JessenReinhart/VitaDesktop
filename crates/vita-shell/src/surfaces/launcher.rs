use crate::icons;
use crate::message::Message;
use crate::style::{
    FONT_MEDIUM, FONT_REGULAR, FONT_SEMIBOLD, calm_scrollable_style, color,
    fallback_app_tile_style, hairline_divider, launcher_tile_button, popover_card_surface,
    segmented_item_button, segmented_shell_style, subtle_pill_button, transparent_container,
};
use iced::border::Radius;
use iced::widget::{
    Id, Space, button, column, container, image, row, scrollable, space, text, text_input,
};
use iced::{Alignment, Background, Border, Color, Element, Length, Padding};
use std::collections::HashMap;
use vita_core::{
    AppCategory, InstalledApp, LauncherViewMode, ShellConfig, ThemeTokens, filter_and_sort_apps,
};

pub const LAUNCHER_WINDOW_WIDTH: f32 = 688.0;
pub const LAUNCHER_WINDOW_HEIGHT: f32 = 524.0;
const SHADOW_GUTTER: f32 = 18.0;

pub fn search_input_id() -> Id {
    Id::new("vita-launcher-search")
}

pub struct LauncherState<'a> {
    pub config: &'a ShellConfig,
    pub tokens: &'a ThemeTokens,
    pub apps: &'a [InstalledApp],
    pub app_icons: &'a HashMap<String, image::Handle>,
    pub query: &'a str,
    pub view_mode: LauncherViewMode,
    pub selected_category: AppCategory,
    pub page: usize,
    pub selected_idx: usize,
    pub apps_loading: bool,
}

fn truncate_app_name(name: &str, max_chars: usize) -> String {
    let mut chars = name.chars();
    let collected: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{}…", collected.trim_end())
    } else {
        collected
    }
}

pub fn view<'a>(state: LauncherState<'a>) -> Element<'a, Message> {
    let tokens = state.tokens;
    let icon_fg = color(tokens.text_primary);
    let muted_fg = color(tokens.text_secondary);
    let subtle_fg = color(tokens.text_muted);

    let effective_mode = if state.query.trim().is_empty() {
        state.view_mode
    } else {
        LauncherViewMode::Grid
    };

    let filtered = filter_and_sort_apps(state.apps, state.query, state.selected_category);
    let per_page = (state.config.launcher.cols * state.config.launcher.rows).max(1);
    let total_pages = filtered.len().div_ceil(per_page).max(1);
    let page = state.page.min(total_pages.saturating_sub(1));

    // Segmented Grid | Category control
    let view_toggle = container(
        row![
            button(icons::icon(icons::GRID, 15.0, icon_fg))
                .padding([6, 9])
                .style(segmented_item_button(
                    tokens,
                    state.view_mode == LauncherViewMode::Grid
                ))
                .on_press(Message::LauncherViewModeChanged(LauncherViewMode::Grid)),
            button(icons::icon(icons::CATEGORIES, 15.0, icon_fg))
                .padding([6, 9])
                .style(segmented_item_button(
                    tokens,
                    state.view_mode == LauncherViewMode::Category
                ))
                .on_press(Message::LauncherViewModeChanged(LauncherViewMode::Category)),
        ]
        .spacing(2)
        .align_y(Alignment::Center),
    )
    .padding(3)
    .style({
        let tokens = *tokens;
        move |_| segmented_shell_style(&tokens)
    });

    // Clean search bar wrapper with 1.7px SVG search icon and clear button
    let search_input = text_input("Search applications...", state.query)
        .id(search_input_id())
        .font(FONT_REGULAR)
        .size(13)
        .padding([7, 4])
        .width(Length::Fill)
        .on_input(Message::LauncherQueryChanged)
        .on_submit(Message::LauncherLaunchSelected)
        .style({
            let tokens = *tokens;
            move |_theme, _status| text_input::Style {
                background: Background::Color(Color::TRANSPARENT),
                border: Border::default(),
                icon: color(tokens.text_muted),
                placeholder: color(tokens.text_muted),
                value: color(tokens.text_primary),
                selection: color(tokens.accent_soft),
            }
        });

    let mut search_row = row![icons::icon(icons::SEARCH, 15.0, subtle_fg), search_input]
        .spacing(8)
        .align_y(Alignment::Center);

    if !state.query.is_empty() {
        search_row = search_row.push(
            button(icons::icon(icons::CLOSE, 13.0, muted_fg))
                .padding(4)
                .style(subtle_pill_button(tokens, false, tokens.r8))
                .on_press(Message::LauncherQueryChanged(String::new())),
        );
    }

    let search_box = container(search_row)
        .width(Length::Fill)
        .height(36)
        .padding([0, 12])
        .align_y(Alignment::Center)
        .style({
            let tokens = *tokens;
            move |_| container::Style {
                background: Some(Background::Color(color(tokens.surface_elevated))),
                border: Border {
                    color: color(tokens.border_control),
                    width: 1.0,
                    radius: Radius::from(tokens.r12),
                },
                snap: true,
                ..container::Style::default()
            }
        });

    let header = row![view_toggle, search_box]
        .spacing(12)
        .align_y(Alignment::Center)
        .width(Length::Fill);

    // Body: Grid (5x3 paginated) OR Category split view
    let body: Element<'a, Message> = if state.apps_loading && state.apps.is_empty() {
        container(
            column![
                text("Discovering installed applications...")
                    .font(FONT_MEDIUM)
                    .size(14)
                    .color(muted_fg),
                text("Indexing Win32 and Windows Store apps")
                    .font(FONT_REGULAR)
                    .size(12)
                    .color(subtle_fg),
            ]
            .spacing(4)
            .align_x(Alignment::Center),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .into()
    } else if filtered.is_empty() {
        container(
            column![
                text("No matching applications")
                    .font(FONT_SEMIBOLD)
                    .size(14)
                    .color(icon_fg),
                text("Try another keyword or clear your search filter.")
                    .font(FONT_REGULAR)
                    .size(12)
                    .color(muted_fg),
            ]
            .spacing(4)
            .align_x(Alignment::Center),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .into()
    } else {
        match effective_mode {
            LauncherViewMode::Grid => build_grid_body(&state, &filtered, page, total_pages),
            LauncherViewMode::Category => build_category_body(&state, &filtered),
        }
    };

    // Footer with keyboard hint + Pin button for selected item
    let selected_app = filtered.get(state.selected_idx).copied().or_else(|| {
        if effective_mode == LauncherViewMode::Grid {
            filtered.get(page * per_page).copied()
        } else {
            filtered.first().copied()
        }
    });

    let pin_control: Element<'a, Message> = if let Some(app) = selected_app {
        let is_pinned = state.config.is_pinned(&app.aumid);
        let pin_icon = if is_pinned {
            icons::PIN_OFF
        } else {
            icons::PIN
        };
        let pin_label = if is_pinned {
            "Unpin from dock"
        } else {
            "Pin to dock"
        };
        button(
            row![
                icons::icon(pin_icon, 13.0, icon_fg),
                text(pin_label).font(FONT_MEDIUM).size(11).color(icon_fg),
            ]
            .spacing(5)
            .align_y(Alignment::Center),
        )
        .padding([4, 9])
        .style(subtle_pill_button(tokens, is_pinned, tokens.r8))
        .on_press(Message::LauncherTogglePin(app.aumid.clone()))
        .into()
    } else {
        Space::new().into()
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
            text("Enter Launch   ·   ← → Page   ·   Esc Close")
                .font(FONT_REGULAR)
                .size(11)
                .color(subtle_fg),
            space::horizontal(),
            pin_control,
        ]
        .padding(Padding {
            top: 8.0,
            right: 2.0,
            bottom: 0.0,
            left: 2.0,
        })
        .align_y(Alignment::Center),
    ];

    let card = container(
        column![header, body, footer]
            .spacing(12)
            .width(Length::Fill)
            .height(Length::Fill),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .padding(Padding {
        top: 18.0,
        right: 18.0,
        bottom: 12.0,
        left: 18.0,
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

fn build_grid_body<'a>(
    state: &LauncherState<'a>,
    filtered: &[&'a InstalledApp],
    page: usize,
    total_pages: usize,
) -> Element<'a, Message> {
    let cols = state.config.launcher.cols.max(1);
    let rows = state.config.launcher.rows.max(1);
    let per_page = cols * rows;
    let start = page * per_page;
    let end = (start + per_page).min(filtered.len());
    let slice = &filtered[start..end];

    let mut grid_col = column![].spacing(6).width(Length::Fill);
    for r in 0..rows {
        let mut grid_row = row![].spacing(6).width(Length::Fill);
        for c in 0..cols {
            let local_idx = r * cols + c;
            let global_idx = start + local_idx;
            if let Some(app) = slice.get(local_idx) {
                let selected = global_idx == state.selected_idx;
                grid_row = grid_row.push(app_tile(
                    state.tokens,
                    app,
                    state.app_icons,
                    118.0,
                    108.0,
                    selected,
                ));
            } else {
                grid_row = grid_row.push(Space::new().width(118.0).height(108.0));
            }
        }
        grid_col = grid_col.push(grid_row);
    }

    let pagination: Element<'a, Message> = if total_pages > 1 {
        let tokens = state.tokens;
        let muted = color(tokens.text_secondary);
        let mut dots = row![].spacing(6).align_y(Alignment::Center);

        for p in 0..total_pages {
            let active = p == page;
            let dot_w = if active { 18.0 } else { 6.0 };
            let dot_color = color(if active {
                tokens.accent_primary
            } else {
                tokens.border_control
            });
            dots = dots.push(
                button(
                    container(Space::new())
                        .width(dot_w)
                        .height(6)
                        .style(move |_| container::Style {
                            background: Some(Background::Color(dot_color)),
                            border: Border {
                                radius: Radius::from(3.0),
                                ..Border::default()
                            },
                            ..container::Style::default()
                        }),
                )
                .padding([4, 2])
                .style(|_, _| button::Style {
                    background: None,
                    ..button::Style::default()
                })
                .on_press(Message::LauncherPageChanged(p)),
            );
        }

        let prev_btn = button(icons::icon(icons::CHEVRON_LEFT, 14.0, muted))
            .padding([3, 6])
            .style(subtle_pill_button(tokens, false, tokens.r8))
            .on_press(Message::LauncherPageChanged(page.saturating_sub(1)));

        let next_btn = button(icons::icon(icons::CHEVRON_RIGHT, 14.0, muted))
            .padding([3, 6])
            .style(subtle_pill_button(tokens, false, tokens.r8))
            .on_press(Message::LauncherPageChanged(
                (page + 1).min(total_pages.saturating_sub(1)),
            ));

        container(
            row![prev_btn, dots, next_btn]
                .spacing(8)
                .align_y(Alignment::Center),
        )
        .width(Length::Fill)
        .center_x(Length::Fill)
        .into()
    } else {
        Space::new().height(12).into()
    };

    column![
        container(grid_col).width(Length::Fill).height(Length::Fill),
        pagination,
    ]
    .spacing(6)
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

fn build_category_body<'a>(
    state: &LauncherState<'a>,
    filtered: &[&'a InstalledApp],
) -> Element<'a, Message> {
    let tokens = state.tokens;

    let mut rail = column![].spacing(4).width(146);
    for cat in AppCategory::ALL_FILTERABLE {
        let count = if cat == AppCategory::All {
            state.apps.len()
        } else {
            state.apps.iter().filter(|a| a.category == cat).count()
        };
        if count == 0 {
            continue;
        }
        let active = state.selected_category == cat;
        let label_color = color(if active {
            tokens.text_primary
        } else {
            tokens.text_secondary
        });
        let count_color = color(tokens.text_muted);

        rail = rail.push(
            button(
                row![
                    text(cat.label())
                        .font(if active { FONT_SEMIBOLD } else { FONT_MEDIUM })
                        .size(12)
                        .color(label_color),
                    space::horizontal(),
                    text(count.to_string())
                        .font(FONT_REGULAR)
                        .size(11)
                        .color(count_color),
                ]
                .align_y(Alignment::Center),
            )
            .width(Length::Fill)
            .padding([7, 10])
            .style(subtle_pill_button(tokens, active, tokens.r8))
            .on_press(Message::LauncherCategorySelected(cat)),
        );
    }

    let cols = 3usize;
    let mut grid = column![].spacing(8).width(Length::Fill);
    for (row_idx, chunk) in filtered.chunks(cols).enumerate() {
        let mut r = row![].spacing(8).width(Length::Fill);
        for (col_idx, app) in chunk.iter().enumerate() {
            let idx = row_idx * cols + col_idx;
            r = r.push(app_tile(
                tokens,
                app,
                state.app_icons,
                142.0,
                104.0,
                idx == state.selected_idx,
            ));
        }
        grid = grid.push(r);
    }

    let right_scroll = scrollable(grid)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(calm_scrollable_style(tokens));

    row![
        scrollable(rail)
            .width(146)
            .height(Length::Fill)
            .style(calm_scrollable_style(tokens)),
        container(Space::new())
            .width(1)
            .height(Length::Fill)
            .style({
                let tokens = *tokens;
                move |_| hairline_divider(&tokens)
            }),
        right_scroll,
    ]
    .spacing(12)
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

fn app_tile<'a>(
    tokens: &'a ThemeTokens,
    app: &'a InstalledApp,
    app_icons: &'a HashMap<String, image::Handle>,
    tile_w: f32,
    tile_h: f32,
    selected: bool,
) -> Element<'a, Message> {
    let icon_fg = color(tokens.text_primary);
    let label_fg = color(tokens.text_primary);

    let graphic: Element<'a, Message> = if let Some(handle) = app_icons.get(&app.aumid) {
        image(handle.clone()).width(48).height(48).into()
    } else {
        container(icons::icon(icons::APP_FALLBACK, 26.0, icon_fg))
            .width(48)
            .height(48)
            .center_x(48)
            .center_y(48)
            .style({
                let tokens = *tokens;
                move |_| fallback_app_tile_style(&tokens)
            })
            .into()
    };

    let display = truncate_app_name(&app.name, 22);
    button(
        column![
            graphic,
            text(display)
                .font(FONT_MEDIUM)
                .size(12)
                .color(label_fg)
                .align_x(iced::alignment::Horizontal::Center),
        ]
        .spacing(8)
        .align_x(Alignment::Center)
        .width(Length::Fill),
    )
    .width(tile_w)
    .height(tile_h)
    .padding([12, 8])
    .style(launcher_tile_button(tokens, selected))
    .on_press(Message::LauncherLaunchApp(app.aumid.clone()))
    .into()
}
