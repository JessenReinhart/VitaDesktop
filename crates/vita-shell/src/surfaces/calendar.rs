use crate::icons;
use crate::style::{
    FONT_MEDIUM, FONT_MONO, FONT_REGULAR, FONT_SEMIBOLD, calm_scrollable_style, color,
    hairline_divider, popover_card_surface, subtle_pill_button, transparent_container,
};
use chrono::{DateTime, Datelike, Local, NaiveDate};
use iced::border::Radius;
use iced::widget::{Space, button, column, container, row, scrollable, space, text, text_input};
use iced::{Alignment, Background, Border, Color, Element, Length, Padding, Shadow};
use vita_core::{CalendarDay, CalendarEvent, ThemeTokens, build_month_grid};

pub const CALENDAR_WIDTH: f32 = 348.0;
pub const CALENDAR_HEIGHT: f32 = 446.0;
const SHADOW_GUTTER: f32 = 16.0;

const WEEKDAYS_MON: [&str; 7] = ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"];
const WEEKDAYS_SUN: [&str; 7] = ["Su", "Mo", "Tu", "We", "Th", "Fr", "Sa"];

fn month_name(month: u32) -> &'static str {
    match month {
        1 => "January",
        2 => "February",
        3 => "March",
        4 => "April",
        5 => "May",
        6 => "June",
        7 => "July",
        8 => "August",
        9 => "September",
        10 => "October",
        11 => "November",
        12 => "December",
        _ => "Month",
    }
}

#[allow(clippy::too_many_arguments)]
pub fn view<'a, Msg: Clone + 'a>(
    tokens: &'a ThemeTokens,
    now: DateTime<Local>,
    display_year: i32,
    display_month: u32,
    selected_date: NaiveDate,
    monday_first: bool,
    events: &'a [CalendarEvent],
    draft: &'a str,
    on_prev_month: Msg,
    on_next_month: Msg,
    on_today: Msg,
    on_select_date: impl Fn(NaiveDate) -> Msg + 'a,
    on_draft_changed: impl Fn(String) -> Msg + 'a,
    on_add_event: Msg,
    on_toggle_event: impl Fn(u64) -> Msg + 'a,
    on_delete_event: impl Fn(u64) -> Msg + 'a,
) -> Element<'a, Msg> {
    let primary_fg = color(tokens.text_primary);
    let secondary_fg = color(tokens.text_secondary);
    let muted_fg = color(tokens.text_muted);

    let today = now.date_naive();
    let away_from_today =
        selected_date != today || display_year != today.year() || display_month != today.month();

    let header_date = now.format("%A, %b %-d").to_string();
    let header_clock = now.format("%H:%M:%S").to_string();

    let header = row![
        column![
            text(header_date)
                .font(FONT_SEMIBOLD)
                .size(14)
                .color(primary_fg),
            text(header_clock)
                .font(FONT_MONO)
                .size(11.5)
                .color(secondary_fg),
        ]
        .spacing(1),
        space::horizontal(),
        button(text("Today").font(FONT_MEDIUM).size(11.5).color(primary_fg),)
            .padding([4, 10])
            .style(subtle_pill_button(tokens, away_from_today, tokens.pill))
            .on_press(on_today),
    ]
    .align_y(Alignment::Center)
    .width(Length::Fill);

    let month_label = format!("{} {display_year}", month_name(display_month.clamp(1, 12)));
    let month_nav = row![
        button(icons::icon(icons::CHEVRON_LEFT, 14.0, primary_fg))
            .padding([4, 7])
            .style(subtle_pill_button(tokens, false, tokens.r8))
            .on_press(on_prev_month),
        space::horizontal(),
        text(month_label)
            .font(FONT_SEMIBOLD)
            .size(13)
            .color(primary_fg),
        space::horizontal(),
        button(icons::icon(icons::CHEVRON_RIGHT, 14.0, primary_fg))
            .padding([4, 7])
            .style(subtle_pill_button(tokens, false, tokens.r8))
            .on_press(on_next_month),
    ]
    .align_y(Alignment::Center)
    .width(Length::Fill);

    let weekday_labels = if monday_first {
        &WEEKDAYS_MON
    } else {
        &WEEKDAYS_SUN
    };
    let mut weekday_row = row![].spacing(3).width(Length::Fill);
    for label in weekday_labels {
        weekday_row = weekday_row.push(
            container(text(*label).font(FONT_MEDIUM).size(10.5).color(muted_fg))
                .width(Length::FillPortion(1))
                .center_x(Length::Fill),
        );
    }

    let grid = build_month_grid(
        display_year,
        display_month,
        today,
        selected_date,
        monday_first,
        events,
    );
    let mut grid_rows = column![].spacing(2).width(Length::Fill);
    for week in grid.chunks(7) {
        let mut week_row = row![].spacing(3).width(Length::Fill);
        for day in week {
            let day_fg = if day.is_today {
                color(tokens.text_on_accent)
            } else if day.is_selected || day.in_current_month {
                primary_fg
            } else {
                muted_fg
            };
            let day_font = if day.is_today || day.is_selected {
                FONT_SEMIBOLD
            } else if day.in_current_month {
                FONT_MEDIUM
            } else {
                FONT_REGULAR
            };

            let dot: Element<'a, Msg> = if day.event_count > 0 {
                let dot_color = color(if day.is_today {
                    tokens.text_on_accent
                } else if day.in_current_month {
                    tokens.accent_primary
                } else {
                    tokens.accent_primary.with_alpha(0.55)
                });
                container(Space::new())
                    .width(4)
                    .height(4)
                    .style(move |_| container::Style {
                        background: Some(Background::Color(dot_color)),
                        border: Border {
                            radius: Radius::from(2.0),
                            ..Border::default()
                        },
                        ..container::Style::default()
                    })
                    .into()
            } else {
                Space::new().width(4).height(4).into()
            };

            let cell_inner = container(
                column![
                    text(day.date.day().to_string())
                        .font(day_font)
                        .size(11.5)
                        .color(day_fg),
                    dot
                ]
                .spacing(1)
                .align_x(Alignment::Center),
            )
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill);

            week_row = week_row.push(
                button(cell_inner)
                    .width(Length::FillPortion(1))
                    .height(26)
                    .padding(0)
                    .style(day_cell_button(*tokens, *day))
                    .on_press(on_select_date(day.date)),
            );
        }
        grid_rows = grid_rows.push(week_row);
    }

    let calendar_block = column![weekday_row, grid_rows]
        .spacing(4)
        .width(Length::Fill);

    // Selected-date agenda section
    let selected_key = selected_date.format("%Y-%m-%d").to_string();
    let day_events: Vec<&CalendarEvent> =
        events.iter().filter(|e| e.date == selected_key).collect();

    let agenda_title = if selected_date == today {
        format!("Today · {}", selected_date.format("%b %-d"))
    } else {
        selected_date.format("%a, %b %-d").to_string()
    };
    let mut agenda_header = row![
        text(agenda_title)
            .font(FONT_SEMIBOLD)
            .size(12)
            .color(primary_fg),
        space::horizontal(),
    ]
    .align_y(Alignment::Center)
    .width(Length::Fill);

    if !day_events.is_empty() {
        let count_text = if day_events.len() == 1 {
            "1 reminder".to_string()
        } else {
            format!("{} reminders", day_events.len())
        };
        agenda_header = agenda_header.push(
            text(count_text)
                .font(FONT_REGULAR)
                .size(11)
                .color(secondary_fg),
        );
    }

    let reminders_area: Element<'a, Msg> = if day_events.is_empty() {
        container(
            text("No reminders for this day")
                .font(FONT_REGULAR)
                .size(11.5)
                .color(muted_fg),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .into()
    } else {
        let mut list = column![].spacing(4).width(Length::Fill);
        for ev in day_events {
            let check_box: Element<'a, Msg> = if ev.done {
                container(icons::icon(
                    icons::CHECK,
                    10.0,
                    color(tokens.text_on_accent),
                ))
                .width(16)
                .height(16)
                .center_x(16)
                .center_y(16)
                .style({
                    let tokens = *tokens;
                    move |_| container::Style {
                        background: Some(Background::Color(color(tokens.accent_primary))),
                        border: Border {
                            radius: Radius::from(5.0),
                            ..Border::default()
                        },
                        snap: true,
                        ..container::Style::default()
                    }
                })
                .into()
            } else {
                container(Space::new())
                    .width(16)
                    .height(16)
                    .style({
                        let tokens = *tokens;
                        move |_| container::Style {
                            background: Some(Background::Color(color(tokens.surface_elevated))),
                            border: Border {
                                color: color(tokens.border_control),
                                width: 1.5,
                                radius: Radius::from(5.0),
                            },
                            snap: true,
                            ..container::Style::default()
                        }
                    })
                    .into()
            };

            let title_fg = if ev.done { secondary_fg } else { primary_fg };
            let toggle_btn = button(
                row![
                    check_box,
                    text(&ev.title)
                        .font(FONT_REGULAR)
                        .size(11.5)
                        .color(title_fg)
                        .width(Length::Fill),
                ]
                .spacing(8)
                .align_y(Alignment::Center)
                .width(Length::Fill),
            )
            .width(Length::Fill)
            .padding([5, 6])
            .style(reminder_toggle_button(*tokens))
            .on_press(on_toggle_event(ev.id));

            let delete_btn = button(icons::icon(icons::CLOSE, 11.0, secondary_fg))
                .padding(5)
                .style(reminder_delete_button(*tokens))
                .on_press(on_delete_event(ev.id));

            list = list.push(
                row![toggle_btn, delete_btn]
                    .spacing(4)
                    .align_y(Alignment::Center)
                    .width(Length::Fill),
            );
        }

        scrollable(list)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(calm_scrollable_style(tokens))
            .into()
    };

    let can_add = !draft.trim().is_empty();
    let mut draft_input = text_input("Add reminder...", draft)
        .font(FONT_REGULAR)
        .size(12)
        .padding([6, 4])
        .width(Length::Fill)
        .on_input(on_draft_changed)
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
    if can_add {
        draft_input = draft_input.on_submit(on_add_event.clone());
    }

    let input_box = container(draft_input)
        .width(Length::Fill)
        .height(32)
        .padding([0, 8])
        .align_y(Alignment::Center)
        .style({
            let tokens = *tokens;
            move |_| container::Style {
                background: Some(Background::Color(color(tokens.surface_elevated))),
                border: Border {
                    color: color(tokens.border_control),
                    width: 1.0,
                    radius: Radius::from(tokens.r8),
                },
                snap: true,
                ..container::Style::default()
            }
        });

    let add_fg = color(if can_add {
        tokens.text_on_accent
    } else {
        tokens.text_secondary
    });
    let mut add_btn = button(
        container(text("Add").font(FONT_MEDIUM).size(11.5).color(add_fg)).center_y(Length::Fill),
    )
    .height(32)
    .padding([0, 12])
    .style(add_event_button(*tokens, can_add));
    if can_add {
        add_btn = add_btn.on_press(on_add_event);
    }

    let add_bar = row![input_box, add_btn]
        .spacing(6)
        .align_y(Alignment::Center)
        .width(Length::Fill);

    let agenda_section = column![agenda_header, reminders_area, add_bar]
        .spacing(5)
        .width(Length::Fill)
        .height(Length::Fill);

    let body = column![
        header,
        divider(*tokens),
        month_nav,
        calendar_block,
        divider(*tokens),
        agenda_section,
    ]
    .spacing(5)
    .width(Length::Fill)
    .height(Length::Fill);

    let card = container(body)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(Padding {
            top: 12.0,
            right: 14.0,
            bottom: 12.0,
            left: 14.0,
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

fn divider<'a, Msg: 'a>(tokens: ThemeTokens) -> Element<'a, Msg> {
    container(Space::new().width(Length::Fill).height(1))
        .width(Length::Fill)
        .height(1)
        .style(move |_| hairline_divider(&tokens))
        .into()
}

fn day_cell_button(
    tokens: ThemeTokens,
    day: CalendarDay,
) -> impl Fn(&iced::Theme, button::Status) -> button::Style {
    move |_theme, status| {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        if day.is_today {
            let fill = if hovered {
                color(tokens.accent_primary.with_alpha(0.88))
            } else {
                color(tokens.accent_primary)
            };
            return button::Style {
                background: Some(Background::Color(fill)),
                text_color: color(tokens.text_on_accent),
                border: Border {
                    color: Color::TRANSPARENT,
                    width: 0.0,
                    radius: Radius::from(tokens.pill),
                },
                shadow: Shadow::default(),
                snap: true,
            };
        }

        if day.is_selected {
            let fill = if hovered {
                color(
                    tokens
                        .accent_soft
                        .with_alpha((tokens.accent_soft.a + 0.08).min(0.35)),
                )
            } else {
                color(tokens.accent_soft)
            };
            return button::Style {
                background: Some(Background::Color(fill)),
                text_color: color(tokens.text_primary),
                border: Border {
                    color: color(tokens.accent_primary),
                    width: 1.5,
                    radius: Radius::from(tokens.pill),
                },
                shadow: Shadow::default(),
                snap: true,
            };
        }

        let bg = if hovered {
            Some(Background::Color(color(tokens.surface_subtle)))
        } else {
            None
        };
        let fg = if day.in_current_month {
            color(tokens.text_primary)
        } else {
            color(tokens.text_muted)
        };
        button::Style {
            background: bg,
            text_color: fg,
            border: Border {
                color: Color::TRANSPARENT,
                width: 0.0,
                radius: Radius::from(tokens.pill),
            },
            shadow: Shadow::default(),
            snap: true,
        }
    }
}

fn reminder_toggle_button(
    tokens: ThemeTokens,
) -> impl Fn(&iced::Theme, button::Status) -> button::Style {
    move |_theme, status| {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: hovered.then_some(Background::Color(color(tokens.surface_subtle))),
            text_color: color(tokens.text_primary),
            border: Border {
                radius: Radius::from(tokens.r8),
                ..Border::default()
            },
            shadow: Shadow::default(),
            snap: true,
        }
    }
}

fn reminder_delete_button(
    tokens: ThemeTokens,
) -> impl Fn(&iced::Theme, button::Status) -> button::Style {
    move |_theme, status| {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: hovered.then_some(Background::Color(color(
                tokens.status_error.with_alpha(0.14),
            ))),
            text_color: color(if hovered {
                tokens.status_error
            } else {
                tokens.text_secondary
            }),
            border: Border {
                radius: Radius::from(tokens.r8),
                ..Border::default()
            },
            shadow: Shadow::default(),
            snap: true,
        }
    }
}

fn add_event_button(
    tokens: ThemeTokens,
    enabled: bool,
) -> impl Fn(&iced::Theme, button::Status) -> button::Style {
    move |_theme, status| {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        if enabled {
            let bg = if hovered {
                color(tokens.accent_primary.with_alpha(0.88))
            } else {
                color(tokens.accent_primary)
            };
            button::Style {
                background: Some(Background::Color(bg)),
                text_color: color(tokens.text_on_accent),
                border: Border {
                    color: Color::TRANSPARENT,
                    width: 0.0,
                    radius: Radius::from(tokens.r8),
                },
                shadow: Shadow::default(),
                snap: true,
            }
        } else {
            button::Style {
                background: Some(Background::Color(color(tokens.surface_subtle))),
                text_color: color(tokens.text_secondary),
                border: Border {
                    color: color(tokens.border_control),
                    width: 1.0,
                    radius: Radius::from(tokens.r8),
                },
                shadow: Shadow::default(),
                snap: true,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vita_core::{AccentPreset, SurfaceMaterial, ThemeMode};

    #[derive(Clone)]
    #[allow(dead_code)]
    enum TestMsg {
        Prev,
        Next,
        Today,
        Select(NaiveDate),
        Draft(String),
        Add,
        Toggle(u64),
        Delete(u64),
    }

    #[test]
    fn month_names_and_dimensions_are_valid() {
        assert!((CALENDAR_WIDTH - 348.0).abs() < f32::EPSILON);
        assert!((CALENDAR_HEIGHT - 446.0).abs() < f32::EPSILON);
        assert_eq!(month_name(1), "January");
        assert_eq!(month_name(11), "November");
        assert_eq!(month_name(12), "December");
    }

    #[test]
    fn view_builds_across_themes_without_panicking() {
        let now = Local::now();
        let selected = NaiveDate::from_ymd_opt(2026, 11, 23).unwrap();
        let events = vec![
            CalendarEvent {
                id: 1,
                date: "2026-11-23".to_string(),
                title: "Team standup".to_string(),
                done: false,
            },
            CalendarEvent {
                id: 2,
                date: "2026-11-23".to_string(),
                title: "Review PR".to_string(),
                done: true,
            },
        ];

        for mode in [ThemeMode::Light, ThemeMode::Dark] {
            let tokens = ThemeTokens::resolve(
                mode,
                false,
                AccentPreset::Terracotta,
                SurfaceMaterial::Matte,
                false,
            );
            let _populated = view(
                &tokens,
                now,
                2026,
                11,
                selected,
                true,
                &events,
                "Draft",
                TestMsg::Prev,
                TestMsg::Next,
                TestMsg::Today,
                TestMsg::Select,
                TestMsg::Draft,
                TestMsg::Add,
                TestMsg::Toggle,
                TestMsg::Delete,
            );
            let _empty = view(
                &tokens,
                now,
                2026,
                12,
                selected,
                false,
                &[],
                "",
                TestMsg::Prev,
                TestMsg::Next,
                TestMsg::Today,
                TestMsg::Select,
                TestMsg::Draft,
                TestMsg::Add,
                TestMsg::Toggle,
                TestMsg::Delete,
            );
        }
    }
}
