use crate::icons;
use crate::message::Message;
use crate::style::{
    FONT_MEDIUM, FONT_MONO, FONT_REGULAR, FONT_SEMIBOLD, calm_scrollable_style, color,
    hairline_divider, subtle_pill_button, surface_card_button, transparent_container,
    warm_accent_slider_style,
};
use iced::border::Radius;
use iced::widget::{
    Space, button, column, container, mouse_area, row, scrollable, slider, space, text, text_input,
};
use iced::{Alignment, Background, Border, Color, Element, Length};
use vita_core::{
    AccentPreset, DecoratorButtonStyle, DockBackgroundStyle, DockVisibility, MonitorInfo,
    RgbaColor, ShellConfig, ShellPreset, SurfaceMaterial, ThemeMode, ThemeTokens, TopBarStyle,
};

pub const SETTINGS_WINDOW_WIDTH: f32 = 860.0;
pub const SETTINGS_WINDOW_HEIGHT: f32 = 600.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SettingsTab {
    Overview,
    #[default]
    Appearance,
    BarAndDock,
    CalmZone,
    System,
}

impl SettingsTab {
    pub const ALL: [Self; 5] = [
        Self::Overview,
        Self::Appearance,
        Self::BarAndDock,
        Self::CalmZone,
        Self::System,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Appearance => "Appearance",
            Self::BarAndDock => "Bar & Dock",
            Self::CalmZone => "Calm Zone",
            Self::System => "System",
        }
    }

    pub fn icon_bytes(self) -> &'static [u8] {
        match self {
            Self::Overview => icons::LAYERS,
            Self::Appearance => icons::SUN,
            Self::BarAndDock => icons::SLIDERS,
            Self::CalmZone => icons::CLOCK,
            Self::System => icons::SETTINGS,
        }
    }
}

pub struct SettingsState<'a> {
    pub config: &'a ShellConfig,
    pub tokens: &'a ThemeTokens,
    pub primary_monitor: &'a MonitorInfo,
    pub monitor_count: usize,
    pub running_window_count: usize,
    pub space_count: usize,
    pub installed_app_count: usize,
    pub active_tab: SettingsTab,
    pub config_path: &'a str,
}

pub fn view<'a>(state: SettingsState<'a>) -> Element<'a, Message> {
    let tokens = state.tokens;
    let primary_fg = color(tokens.text_primary);
    let secondary_fg = color(tokens.text_secondary);
    let muted_fg = color(tokens.text_muted);

    // 56px draggable titlebar with separate non-drag window controls on the right.
    let drag_region = mouse_area(
        container(
            row![
                icons::vita_mark(),
                text("Vita Settings")
                    .font(FONT_SEMIBOLD)
                    .size(14)
                    .color(primary_fg),
                container(
                    text(state.config.preset.label())
                        .font(FONT_MEDIUM)
                        .size(11)
                        .color(secondary_fg)
                )
                .padding([3, 10])
                .style({
                    let tokens = *tokens;
                    move |_| container::Style {
                        background: Some(Background::Color(color(tokens.surface_subtle))),
                        border: Border {
                            color: color(tokens.border_subtle),
                            width: 1.0,
                            radius: Radius::from(tokens.pill),
                        },
                        snap: true,
                        ..container::Style::default()
                    }
                }),
                space::horizontal(),
            ]
            .spacing(10)
            .align_y(Alignment::Center)
            .width(Length::Fill),
        )
        .height(Length::Fill)
        .align_y(Alignment::Center)
        .width(Length::Fill)
        .style(|_| transparent_container()),
    )
    .on_press(Message::DragSettingsWindow);

    let minimize_btn = button(icons::icon(icons::MINUS, 14.0, secondary_fg))
        .padding(7)
        .style(subtle_pill_button(tokens, false, tokens.r8))
        .on_press(Message::MinimizeSettings);

    let close_btn = button(icons::icon(icons::CLOSE, 14.0, primary_fg))
        .padding(7)
        .style(subtle_pill_button(tokens, false, tokens.r8))
        .on_press(Message::CloseSettings);

    let titlebar = container(
        row![
            drag_region,
            row![minimize_btn, close_btn]
                .spacing(6)
                .align_y(Alignment::Center),
        ]
        .spacing(10)
        .align_y(Alignment::Center)
        .width(Length::Fill),
    )
    .width(Length::Fill)
    .height(56)
    .padding([0, 18])
    .align_y(Alignment::Center)
    .style({
        let tokens = *tokens;
        move |_| container::Style {
            background: Some(Background::Color(color(tokens.surface_elevated))),
            border: Border {
                radius: Radius {
                    top_left: 7.0,
                    top_right: 7.0,
                    bottom_left: 0.0,
                    bottom_right: 0.0,
                },
                ..Border::default()
            },
            snap: true,
            ..container::Style::default()
        }
    });

    // Left Studio Sidebar (208px)
    let mut nav_col = column![
        container(
            text("STUDIO")
                .font(FONT_SEMIBOLD)
                .size(10)
                .color(muted_fg)
        )
        .padding([2, 10])
    ]
    .spacing(4)
    .width(Length::Fill);

    for tab in SettingsTab::ALL {
        let active = tab == state.active_tab;
        let icon_color = if active {
            color(tokens.accent_primary)
        } else {
            secondary_fg
        };
        let label_color = if active { primary_fg } else { secondary_fg };

        let tab_btn = button(
            row![
                icons::icon(tab.icon_bytes(), 15.0, icon_color),
                text(tab.label())
                    .font(if active { FONT_SEMIBOLD } else { FONT_MEDIUM })
                    .size(13)
                    .color(label_color),
            ]
            .spacing(10)
            .align_y(Alignment::Center),
        )
        .width(Length::Fill)
        .padding([9, 12])
        .style(sidebar_nav_button(tokens, active))
        .on_press(Message::SettingsTabSelected(tab));

        nav_col = nav_col.push(tab_btn);
    }

    let b = state.primary_monitor.physical_bounds;
    let dpi_pct = (state.primary_monitor.dpi_scale * 100.0).round() as u32;
    let sidebar_footer = container(
        column![
            text("Primary display")
                .font(FONT_MEDIUM)
                .size(10)
                .color(muted_fg),
            text(format!("{}x{} · {}%", b.width, b.height, dpi_pct))
                .font(FONT_MONO)
                .size(11)
                .color(secondary_fg),
        ]
        .spacing(2),
    )
    .width(Length::Fill)
    .padding([10, 12])
    .style({
        let tokens = *tokens;
        move |_| container::Style {
            background: Some(Background::Color(color(tokens.surface_elevated))),
            border: Border {
                color: color(tokens.border_subtle),
                width: 1.0,
                radius: Radius::from(tokens.r12),
            },
            snap: true,
            ..container::Style::default()
        }
    });

    let sidebar = container(
        column![nav_col, space::vertical(), sidebar_footer]
            .spacing(12)
            .height(Length::Fill)
            .width(Length::Fill),
    )
    .width(208)
    .height(Length::Fill)
    .padding([16, 12])
    .style({
        let tokens = *tokens;
        move |_| container::Style {
            background: Some(Background::Color(color(tokens.surface_subtle))),
            border: Border {
                radius: Radius {
                    top_left: 0.0,
                    top_right: 0.0,
                    bottom_left: 7.0,
                    bottom_right: 0.0,
                },
                ..Border::default()
            },
            snap: true,
            ..container::Style::default()
        }
    });

    let vertical_divider = container(Space::new().width(1).height(Length::Fill)).style({
        let tokens = *tokens;
        move |_| container::Style {
            background: Some(Background::Color(color(tokens.divider))),
            ..container::Style::default()
        }
    });

    let tab_body: Element<'a, Message> = match state.active_tab {
        SettingsTab::Overview => view_overview(&state),
        SettingsTab::Appearance => view_appearance(&state),
        SettingsTab::BarAndDock => view_bar_and_dock(&state),
        SettingsTab::CalmZone => view_calm_zone(&state),
        SettingsTab::System => view_system(&state),
    };

    let content_scroll = scrollable(
        container(tab_body)
            .width(Length::Fill)
            .padding([22, 26]),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .style(calm_scrollable_style(tokens));

    let body_row = row![sidebar, vertical_divider, content_scroll]
        .width(Length::Fill)
        .height(Length::Fill);

    let root_col = column![titlebar, divider(tokens), body_row]
        .width(Length::Fill)
        .height(Length::Fill);

    let outer_bg = match tokens.material {
        SurfaceMaterial::Matte => tokens.surface_base,
        SurfaceMaterial::Frosted => tokens.surface_base.with_alpha(0.95),
    };

    container(root_col)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(1)
        .style({
            let tokens = *tokens;
            move |_| container::Style {
                background: Some(Background::Color(color(outer_bg))),
                text_color: Some(color(tokens.text_primary)),
                border: Border {
                    color: color(tokens.border_control),
                    width: 1.0,
                    radius: Radius::from(8.0),
                },
                snap: true,
                ..container::Style::default()
            }
        })
        .into()
}

fn view_overview<'a>(state: &SettingsState<'a>) -> Element<'a, Message> {
    let tokens = state.tokens;
    let config = state.config;
    let primary_fg = color(tokens.text_primary);
    let secondary_fg = color(tokens.text_secondary);
    let muted_fg = color(tokens.text_muted);

    let b = state.primary_monitor.physical_bounds;
    let dpi_pct = (state.primary_monitor.dpi_scale * 100.0).round() as u32;
    let mon_name = state.primary_monitor.name.trim_start_matches(r"\\.\");
    let win_noun = if state.running_window_count == 1 {
        "window"
    } else {
        "windows"
    };
    let space_noun = if state.space_count == 1 {
        "space"
    } else {
        "spaces"
    };
    let material_label = match config.surface_material {
        SurfaceMaterial::Matte => "Matte",
        SurfaceMaterial::Frosted => "Frosted",
    };
    let bar_label = match config.top_bar.style {
        TopBarStyle::FloatingPills => "Floating pills",
        TopBarStyle::FullWidth => "Full-width bar",
    };

    let metrics_row_1 = row![
        metric_card(
            tokens,
            "Active preset",
            config.preset.label().to_string(),
            format!("{material_label} · {bar_label}"),
            false,
        ),
        metric_card(
            tokens,
            "Display & DPI",
            format!("{}x{} · {}%", b.width, b.height, dpi_pct),
            format!("{mon_name} ({} connected)", state.monitor_count),
            true,
        ),
    ]
    .spacing(12)
    .width(Length::Fill);

    let metrics_row_2 = row![
        metric_card(
            tokens,
            "Workspace",
            format!(
                "{} {} · {} {}",
                state.running_window_count, win_noun, state.space_count, space_noun
            ),
            "Live Win32 & Virtual Desktop sync".to_string(),
            false,
        ),
        metric_card(
            tokens,
            "App Launcher",
            format!("{} installed apps", state.installed_app_count),
            "Cached from Windows AppsFolder".to_string(),
            false,
        ),
    ]
    .spacing(12)
    .width(Length::Fill);

    let mut quick_presets = row![].spacing(8).width(Length::Fill);
    for preset in ShellPreset::BUILTINS {
        let active = config.preset == preset;
        quick_presets = quick_presets.push(
            button(
                text(preset.label())
                    .font(if active { FONT_SEMIBOLD } else { FONT_MEDIUM })
                    .size(12)
                    .color(if active {
                        color(tokens.accent_primary)
                    } else {
                        primary_fg
                    }),
            )
            .width(Length::FillPortion(1))
            .padding([9, 10])
            .style(surface_card_button(tokens, active, tokens.r12))
            .on_press(Message::SettingsSelectPreset(preset)),
        );
    }

    let quick_card = section_card(
        tokens,
        "Quick preset switch",
        column![
            text("Switch cleanly between calm Horizon surfaces and Pantheon layouts.")
                .font(FONT_REGULAR)
                .size(12)
                .color(secondary_fg),
            quick_presets,
        ]
        .spacing(10)
        .into(),
    );

    let shortcuts_card = section_card(
        tokens,
        "Keyboard shortcuts",
        column![
            shortcut_row(tokens, "Alt + Space", "Toggle Applications launcher"),
            divider(tokens),
            shortcut_row(tokens, "Alt + `", "Toggle Spaces overview"),
            divider(tokens),
            shortcut_row(
                tokens,
                "Ctrl + Alt + Shift + R",
                "Emergency restore Windows Taskbar"
            ),
        ]
        .spacing(10)
        .into(),
    );

    let status_note = text(if config.hide_native_taskbar {
        "Taskbar overlay active with background recovery watchdog."
    } else {
        "Companion mode active: Windows Taskbar remains visible."
    })
    .font(FONT_REGULAR)
    .size(11.5)
    .color(muted_fg);

    column![
        section_header(
            tokens,
            "Your desktop at a glance.",
            "Live overview of your shell preset, displays, and workspaces.",
        ),
        metrics_row_1,
        metrics_row_2,
        quick_card,
        shortcuts_card,
        status_note,
    ]
    .spacing(14)
    .width(Length::Fill)
    .into()
}

fn view_appearance<'a>(state: &SettingsState<'a>) -> Element<'a, Message> {
    let tokens = state.tokens;
    let config = state.config;
    let primary_fg = color(tokens.text_primary);
    let secondary_fg = color(tokens.text_secondary);

    // 2x2 Shell Presets
    let preset_card_btn = |preset: ShellPreset| {
        let active = config.preset == preset;
        let dot_color = if active {
            color(tokens.accent_primary)
        } else {
            color(tokens.border_control)
        };
        let badge = container(Space::new().width(8).height(8)).style(move |_| container::Style {
            background: Some(Background::Color(dot_color)),
            border: Border {
                radius: Radius::from(99.0),
                ..Border::default()
            },
            ..container::Style::default()
        });

        button(
            column![
                row![
                    text(preset.label())
                        .font(FONT_SEMIBOLD)
                        .size(13)
                        .color(primary_fg),
                    space::horizontal(),
                    badge,
                ]
                .align_y(Alignment::Center),
                text(preset.subtitle())
                    .font(FONT_REGULAR)
                    .size(11.5)
                    .color(secondary_fg),
            ]
            .spacing(4),
        )
        .width(Length::FillPortion(1))
        .padding([12, 14])
        .style(surface_card_button(tokens, active, tokens.r12))
        .on_press(Message::SettingsSelectPreset(preset))
    };

    let presets_grid = column![
        row![
            preset_card_btn(ShellPreset::HorizonLight),
            preset_card_btn(ShellPreset::HorizonDark),
        ]
        .spacing(10),
        row![
            preset_card_btn(ShellPreset::PantheonLight),
            preset_card_btn(ShellPreset::PantheonDark),
        ]
        .spacing(10),
    ]
    .spacing(10);

    let presets_card = section_card(tokens, "Shell presets", presets_grid.into());

    // Theme mode + Surface Material
    let mode_btn = |mode: ThemeMode, label: &'static str| {
        let active = config.theme_mode == mode;
        button(
            text(label)
                .font(if active { FONT_SEMIBOLD } else { FONT_MEDIUM })
                .size(12.5)
                .color(if active {
                    color(tokens.accent_primary)
                } else {
                    primary_fg
                }),
        )
        .width(Length::FillPortion(1))
        .padding([9, 12])
        .style(surface_card_button(tokens, active, tokens.r12))
        .on_press(Message::SettingsSelectThemeMode(mode))
    };

    let theme_row = row![
        mode_btn(ThemeMode::Light, "Light"),
        mode_btn(ThemeMode::Dark, "Dark"),
        mode_btn(ThemeMode::System, "Match Windows"),
    ]
    .spacing(8)
    .width(Length::Fill);

    let mat_btn = |mat: SurfaceMaterial, title: &'static str, desc: &'static str| {
        let active = config.surface_material == mat;
        button(
            column![
                text(title)
                    .font(FONT_SEMIBOLD)
                    .size(12.5)
                    .color(primary_fg),
                text(desc).font(FONT_REGULAR).size(11.5).color(secondary_fg),
            ]
            .spacing(2),
        )
        .width(Length::FillPortion(1))
        .padding([10, 12])
        .style(surface_card_button(tokens, active, tokens.r12))
        .on_press(Message::SettingsSelectMaterial(mat))
    };

    let material_row = row![
        mat_btn(
            SurfaceMaterial::Matte,
            "Matte surfaces",
            "Calm Pearl & Ink cards",
        ),
        mat_btn(
            SurfaceMaterial::Frosted,
            "Frosted glass",
            "Windows 11 Acrylic translucency",
        ),
    ]
    .spacing(10)
    .width(Length::Fill);

    let theme_material_card = section_card(
        tokens,
        "Theme & finish",
        column![theme_row, material_row].spacing(10).into(),
    );

    // 7 Circular Accent Swatches
    let mut swatch_row = row![].spacing(12).align_y(Alignment::Center);
    for accent in AccentPreset::ALL {
        let active = config.accent == accent;
        let fill = color(accent.swatch_color());
        let ring_color = if active {
            primary_fg
        } else {
            color(tokens.border_control)
        };

        let swatch_circle = container(Space::new().width(24).height(24)).style(move |_| container::Style {
            background: Some(Background::Color(fill)),
            border: Border {
                color: color(tokens.surface_elevated),
                width: 2.0,
                radius: Radius::from(99.0),
            },
            ..container::Style::default()
        });

        let swatch_btn = button(
            container(swatch_circle)
                .width(34)
                .height(34)
                .center_x(Length::Fill)
                .center_y(Length::Fill)
                .style(move |_| container::Style {
                    border: Border {
                        color: ring_color,
                        width: if active { 2.0 } else { 1.0 },
                        radius: Radius::from(99.0),
                    },
                    ..container::Style::default()
                }),
        )
        .padding(0)
        .style(|_, _| button::Style {
            background: None,
            ..button::Style::default()
        })
        .on_press(Message::SettingsSelectAccent(accent));

        swatch_row = swatch_row.push(swatch_btn);
    }

    let accent_card = section_card(
        tokens,
        "Accent ink",
        row![
            swatch_row,
            space::horizontal(),
            container(
                text(config.accent.label())
                    .font(FONT_MEDIUM)
                    .size(12)
                    .color(secondary_fg),
            )
            .padding([5, 12])
            .style({
                let tokens = *tokens;
                move |_| container::Style {
                    background: Some(Background::Color(color(tokens.surface_subtle))),
                    border: Border {
                        color: color(tokens.border_subtle),
                        width: 1.0,
                        radius: Radius::from(tokens.pill),
                    },
                    snap: true,
                    ..container::Style::default()
                }
            }),
        ]
        .align_y(Alignment::Center)
        .into(),
    );

    // Accessibility
    let a11y_card = section_card(
        tokens,
        "Accessibility",
        column![
            toggle_row(
                tokens,
                "Reduce motion",
                "Use gentle fades instead of spatial spring magnification.",
                config.accessibility.reduce_motion,
                Message::SettingsToggleReduceMotion,
            ),
            divider(tokens),
            toggle_row(
                tokens,
                "Increase contrast",
                "Strengthen control borders and elevate label legibility (>= 3:1).",
                config.accessibility.increase_contrast,
                Message::SettingsToggleIncreaseContrast,
            ),
        ]
        .spacing(12)
        .into(),
    );

    // Window Decorator
    let deco = &config.decorator;
    let btn_style_card = |style: DecoratorButtonStyle, title: &'static str, desc: &'static str| {
        let active = deco.button_style == style;
        button(
            column![
                text(title).font(FONT_SEMIBOLD).size(12.5).color(primary_fg),
                text(desc).font(FONT_REGULAR).size(11).color(secondary_fg),
            ]
            .spacing(2),
        )
        .width(Length::FillPortion(1))
        .padding([9, 12])
        .style(surface_card_button(tokens, active, tokens.r12))
        .on_press(Message::SettingsSelectDecoratorButtonStyle(style))
    };

    let decorator_card = section_card(
        tokens,
        "Window decorator",
        column![
            toggle_row(
                tokens,
                "Enable custom window chrome",
                "Theme native titlebars, borders, and controls across application windows.",
                deco.enabled,
                Message::SettingsToggleDecoratorEnabled,
            ),
            divider(tokens),
            row![
                btn_style_card(
                    DecoratorButtonStyle::Horizon,
                    "Horizon buttons",
                    "Crisp 1.7px SVG minimize, maximize & close pills",
                ),
                btn_style_card(
                    DecoratorButtonStyle::TrafficLights,
                    "Traffic lights",
                    "Calm circular amber, sage & coral window controls",
                ),
            ]
            .spacing(10),
            divider(tokens),
            toggle_row(
                tokens,
                "DWM border & caption sync",
                "Apply active accent borders and Pearl/Ink caption colors on Windows 11.",
                deco.dwm_theme_sync,
                Message::SettingsToggleDecoratorDwmSync,
            ),
            divider(tokens),
            toggle_row(
                tokens,
                "Custom floating header & control overlay",
                "Overlay custom drag frames on standard windows and compact pills on tabbed apps.",
                deco.custom_overlay,
                Message::SettingsToggleDecoratorOverlay,
            ),
            divider(tokens),
            toggle_row(
                tokens,
                "Maximized controls in Top Bar (GNOME Unite)",
                "Show window minimize/restore/close controls directly in the Top Bar when maximized.",
                deco.maximized_topbar_controls,
                Message::SettingsToggleDecoratorMaxTopBar,
            ),
            divider(tokens),
            toggle_row(
                tokens,
                "Hide maximized window title bar",
                "Tuck maximized app title bars behind the Top Bar like macOS.",
                deco.hide_maximized_titlebar,
                Message::SettingsToggleHideMaximizedTitlebar,
            ),
            divider(tokens),
            column![
                row![
                    column![
                        text("Title bar height to hide")
                            .font(FONT_MEDIUM)
                            .size(13)
                            .color(primary_fg),
                        text("Match your apps' title bar so the Top Bar covers it exactly.")
                            .font(FONT_REGULAR)
                            .size(11.5)
                            .color(secondary_fg),
                    ]
                    .spacing(2),
                    space::horizontal(),
                    text(format!("{} px", deco.maximized_titlebar_height))
                        .font(FONT_MONO)
                        .size(12)
                        .color(secondary_fg),
                ]
                .align_y(Alignment::Center),
                slider(24.0..=44.0, deco.maximized_titlebar_height as f32, |v| {
                    Message::SettingsMaximizedTitlebarHeightChanged(v.round() as u32)
                })
                .step(1.0_f32)
                .style(warm_accent_slider_style(tokens)),
            ]
            .spacing(6),
        ]
        .spacing(12)
        .into(),
    );

    column![
        section_header(
            tokens,
            "Make yourself at home.",
            "Choose a preset, surface finish, accent color, window chrome, and accessibility.",
        ),
        presets_card,
        theme_material_card,
        accent_card,
        decorator_card,
        a11y_card,
    ]
    .spacing(14)
    .width(Length::Fill)
    .into()
}

fn view_bar_and_dock<'a>(state: &SettingsState<'a>) -> Element<'a, Message> {
    let tokens = state.tokens;
    let config = state.config;
    let primary_fg = color(tokens.text_primary);
    let secondary_fg = color(tokens.text_secondary);

    let bar_style_btn = |style: TopBarStyle, title: &'static str, desc: &'static str| {
        let active = config.top_bar.style == style;
        button(
            column![
                text(title)
                    .font(FONT_SEMIBOLD)
                    .size(13)
                    .color(primary_fg),
                text(desc).font(FONT_REGULAR).size(11.5).color(secondary_fg),
            ]
            .spacing(3),
        )
        .width(Length::FillPortion(1))
        .padding([12, 14])
        .style(surface_card_button(tokens, active, tokens.r12))
        .on_press(Message::SettingsSelectTopBarStyle(style))
    };

    let height_btn = |h: u32| {
        let active = config.top_bar.height == h;
        button(
            text(format!("{h} px"))
                .font(if active { FONT_SEMIBOLD } else { FONT_MONO })
                .size(12)
                .wrapping(iced::widget::text::Wrapping::None)
                .color(if active {
                    color(tokens.accent_primary)
                } else {
                    primary_fg
                }),
        )
        .padding([6, 11])
        .style(surface_card_button(tokens, active, tokens.r12))
        .on_press(Message::SettingsSelectTopBarHeight(h))
    };

    let topbar_card = section_card(
        tokens,
        "Top bar",
        column![
            row![
                bar_style_btn(
                    TopBarStyle::FloatingPills,
                    "Floating pills",
                    "Dux Horizon capsules with edge breathing room",
                ),
                bar_style_btn(
                    TopBarStyle::FullWidth,
                    "Full-width bar",
                    "Pantheon-inspired translucent top strip",
                ),
            ]
            .spacing(10),
            divider(tokens),
            row![
                column![
                    text("Bar height").font(FONT_MEDIUM).size(13).color(primary_fg),
                    text("Reserve GNOME-style top work area across all connected displays.")
                        .font(FONT_REGULAR)
                        .size(11.5)
                        .color(secondary_fg),
                ]
                .spacing(2)
                .width(Length::Fill),
                row![height_btn(34), height_btn(40), height_btn(46)].spacing(6),
            ]
            .spacing(12)
            .align_y(Alignment::Center),
            divider(tokens),
            toggle_row(
                tokens,
                "Show Global Menu in Top Bar",
                "Display active app menus (App, File, Edit, View) in the left top bar.",
                config.top_bar.show_global_menu,
                Message::SettingsToggleGlobalMenu,
            ),
            divider(tokens),
            toggle_row(
                tokens,
                "Show seconds",
                "Include seconds in the center top bar clock.",
                config.top_bar.show_seconds,
                Message::SettingsToggleShowSeconds,
            ),
        ]
        .spacing(12)
        .into(),
    );

    let vis_btn = |vis: DockVisibility, title: &'static str| {
        let active = config.dock.visibility == vis;
        button(
            text(title)
                .font(if active { FONT_SEMIBOLD } else { FONT_MEDIUM })
                .size(12)
                .color(if active {
                    color(tokens.accent_primary)
                } else {
                    primary_fg
                }),
        )
        .width(Length::FillPortion(1))
        .padding([8, 10])
        .style(surface_card_button(tokens, active, tokens.r12))
        .on_press(Message::SettingsSelectDockVisibility(vis))
    };

    let icon_val = config.dock.icon_size;
    let dock_h_val = config.dock.effective_pill_height().round() as u32;
    let mag_val = config.dock.magnification_scale;
    let mag_label = if mag_val <= 1.02 {
        "Off".to_string()
    } else {
        format!("{mag_val:.2}x")
    };

    let dock_card = section_card(
        tokens,
        "Dock behavior",
        column![
            column![
                text("Dock visibility")
                    .font(FONT_MEDIUM)
                    .size(13)
                    .color(primary_fg),
                row![
                    vis_btn(DockVisibility::IntelliHide, "Intelli-hide"),
                    vis_btn(DockVisibility::AlwaysVisible, "Always visible"),
                    vis_btn(DockVisibility::AutoHide, "Auto-hide"),
                ]
                .spacing(8)
                .width(Length::Fill),
            ]
            .spacing(6),
            divider(tokens),
            toggle_row(
                tokens,
                "Show Dock on all displays",
                "Display an independent magnifying dock on every connected monitor.",
                config.dock.all_monitors,
                Message::SettingsToggleDockAllMonitors,
            ),
            divider(tokens),
            toggle_row(
                tokens,
                "Reserve bottom space for maximized windows",
                "Push maximized windows above an always-visible dock.",
                config.dock.reserve_work_area,
                Message::SettingsToggleDockReserveWorkArea,
            ),
            divider(tokens),
            column![
                row![
                    text("Icon size").font(FONT_MEDIUM).size(13).color(primary_fg),
                    space::horizontal(),
                    text(format!("{icon_val} px"))
                        .font(FONT_MONO)
                        .size(12)
                        .color(secondary_fg),
                ]
                .align_y(Alignment::Center),
                slider(28.0..=64.0, icon_val as f32, |v| {
                    Message::SettingsDockIconSizeChanged(v.round() as u32)
                })
                .step(2.0_f32)
                .style(warm_accent_slider_style(tokens)),
            ]
            .spacing(6),
            divider(tokens),
            column![
                row![
                    text("Dock height").font(FONT_MEDIUM).size(13).color(primary_fg),
                    space::horizontal(),
                    text(format!("{dock_h_val} px"))
                        .font(FONT_MONO)
                        .size(12)
                        .color(secondary_fg),
                ]
                .align_y(Alignment::Center),
                slider(36.0..=84.0, dock_h_val as f32, |v| {
                    Message::SettingsDockHeightChanged(v.round() as u32)
                })
                .step(2.0_f32)
                .style(warm_accent_slider_style(tokens)),
            ]
            .spacing(6),
            divider(tokens),
            column![
                row![
                    text("Hover magnification")
                        .font(FONT_MEDIUM)
                        .size(13)
                        .color(primary_fg),
                    space::horizontal(),
                    text(mag_label)
                        .font(FONT_MONO)
                        .size(12)
                        .color(secondary_fg),
                ]
                .align_y(Alignment::Center),
                slider(1.00..=1.60, mag_val, |v| {
                    Message::SettingsDockMagnifyChanged((v * 50.0).round() / 50.0)
                })
                .step(0.02_f32)
                .style(warm_accent_slider_style(tokens)),
            ]
            .spacing(6),
            divider(tokens),
            toggle_row(
                tokens,
                "Show Space chip",
                "Show current space name button on the left side of the dock.",
                config.dock.show_space_chip,
                Message::SettingsToggleDockSpaceChip,
            ),
            divider(tokens),
            toggle_row(
                tokens,
                "Show Resume chip",
                "Show subtle Resume chip on the right side of the dock.",
                config.dock.show_resume_chip,
                Message::SettingsToggleDockResumeChip,
            ),
        ]
        .spacing(12)
        .into(),
    );

    let bg_style_btn = |style: DockBackgroundStyle| {
        let active = config.dock.background_style == style;
        button(
            text(style.label())
                .font(if active { FONT_SEMIBOLD } else { FONT_MEDIUM })
                .size(11.5)
                .color(if active {
                    color(tokens.accent_primary)
                } else {
                    primary_fg
                }),
        )
        .width(Length::FillPortion(1))
        .padding([8, 10])
        .style(surface_card_button(tokens, active, tokens.r12))
        .on_press(Message::SettingsSelectDockBackgroundStyle(style))
    };

    let bg_styles_row_1 = row![
        bg_style_btn(DockBackgroundStyle::ThemeDefault),
        bg_style_btn(DockBackgroundStyle::FrostedGlass),
        bg_style_btn(DockBackgroundStyle::DeepInk),
        bg_style_btn(DockBackgroundStyle::Pearl),
    ]
    .spacing(8)
    .width(Length::Fill);

    let bg_styles_row_2 = row![
        bg_style_btn(DockBackgroundStyle::AccentTint),
        bg_style_btn(DockBackgroundStyle::CustomColor),
        bg_style_btn(DockBackgroundStyle::Transparent),
        Space::new().width(Length::FillPortion(1)),
    ]
    .spacing(8)
    .width(Length::Fill);

    let dock_swatches = [
        "#16192B", "#1E2638", "#2B233D", "#3B232E", "#1D312C", "#F4F5F9", "#E8DFD5",
    ];
    let mut dock_swatch_row = row![].spacing(8).align_y(Alignment::Center);
    for hex in dock_swatches {
        let active = config.dock.background_style == DockBackgroundStyle::CustomColor
            && config.dock.background_color_hex.eq_ignore_ascii_case(hex);
        let fill = color(RgbaColor::from_hex(hex));
        let ring = if active {
            primary_fg
        } else {
            color(tokens.border_control)
        };
        let circle = container(Space::new().width(20).height(20)).style(move |_| container::Style {
            background: Some(Background::Color(fill)),
            border: Border {
                color: color(tokens.surface_elevated),
                width: 2.0,
                radius: Radius::from(99.0),
            },
            ..container::Style::default()
        });
        let btn = button(
            container(circle)
                .width(26)
                .height(26)
                .center_x(Length::Fill)
                .center_y(Length::Fill)
                .style(move |_| container::Style {
                    border: Border {
                        color: ring,
                        width: if active { 2.0 } else { 1.0 },
                        radius: Radius::from(99.0),
                    },
                    ..container::Style::default()
                }),
        )
        .padding(0)
        .style(|_, _| button::Style {
            background: None,
            ..button::Style::default()
        })
        .on_press(Message::SettingsDockBackgroundColorChanged(hex.to_string()));
        dock_swatch_row = dock_swatch_row.push(btn);
    }

    let opacity_val = config.dock.background_opacity;
    let radius_val = config.dock.corner_radius;

    let dock_appearance_card = section_card(
        tokens,
        "Dock appearance",
        column![
            column![
                text("Background style")
                    .font(FONT_MEDIUM)
                    .size(13)
                    .color(primary_fg),
                bg_styles_row_1,
                bg_styles_row_2,
            ]
            .spacing(8),
            divider(tokens),
            row![
                column![
                    text("Custom color")
                        .font(FONT_MEDIUM)
                        .size(13)
                        .color(primary_fg),
                    text("Pick a preset tint or enter any hex color.")
                        .font(FONT_REGULAR)
                        .size(11.5)
                        .color(secondary_fg),
                ]
                .spacing(2)
                .width(Length::Fill),
                dock_swatch_row,
                container(clean_text_input(
                    tokens,
                    "#1E2238",
                    &config.dock.background_color_hex,
                    Message::SettingsDockBackgroundColorChanged,
                ))
                .width(106),
            ]
            .spacing(10)
            .align_y(Alignment::Center)
            .width(Length::Fill),
            divider(tokens),
            column![
                row![
                    text("Background opacity")
                        .font(FONT_MEDIUM)
                        .size(13)
                        .color(primary_fg),
                    space::horizontal(),
                    text(format!("{opacity_val}%"))
                        .font(FONT_MONO)
                        .size(12)
                        .color(secondary_fg),
                ]
                .align_y(Alignment::Center),
                slider(0.0..=100.0, opacity_val as f32, |v| {
                    Message::SettingsDockBackgroundOpacityChanged(v.round() as u8)
                })
                .step(5.0_f32)
                .style(warm_accent_slider_style(tokens)),
            ]
            .spacing(6),
            divider(tokens),
            column![
                row![
                    text("Corner radius")
                        .font(FONT_MEDIUM)
                        .size(13)
                        .color(primary_fg),
                    space::horizontal(),
                    text(format!("{radius_val} px"))
                        .font(FONT_MONO)
                        .size(12)
                        .color(secondary_fg),
                ]
                .align_y(Alignment::Center),
                slider(6.0..=34.0, radius_val as f32, |v| {
                    Message::SettingsDockCornerRadiusChanged(v.round() as u32)
                })
                .step(2.0_f32)
                .style(warm_accent_slider_style(tokens)),
            ]
            .spacing(6),
            divider(tokens),
            toggle_row(
                tokens,
                "Show dock border",
                "Subtle 1px outline around the dock container.",
                config.dock.show_border,
                Message::SettingsToggleDockBorder,
            ),
        ]
        .spacing(12)
        .into(),
    );

    column![
        section_header(
            tokens,
            "Bar & Dock",
            "Tune your top bar geometry, clock formatting, and dock appearance.",
        ),
        topbar_card,
        dock_card,
        dock_appearance_card,
    ]
    .spacing(14)
    .width(Length::Fill)
    .into()
}

fn view_calm_zone<'a>(state: &SettingsState<'a>) -> Element<'a, Message> {
    let tokens = state.tokens;
    let config = state.config;
    let primary_fg = color(tokens.text_primary);
    let secondary_fg = color(tokens.text_secondary);

    let greeting_card = section_card(
        tokens,
        "Desktop greeting widget",
        column![
            toggle_row(
                tokens,
                "Enable Calm Zone",
                "Show the low-noise time and greeting widget at desktop depth.",
                config.calm_zone.enabled,
                Message::SettingsToggleCalmZone,
            ),
            divider(tokens),
            column![
                text("Your name").font(FONT_MEDIUM).size(12.5).color(primary_fg),
                clean_text_input(
                    tokens,
                    "Alex",
                    &config.calm_zone.user_name,
                    Message::SettingsCalmUserNameChanged,
                ),
            ]
            .spacing(6),
        ]
        .spacing(12)
        .into(),
    );

    let thread_card = section_card(
        tokens,
        "Personal Thread card",
        column![
            toggle_row(
                tokens,
                "Show Personal Thread card",
                "Keep a gentle focus prompt anchored beneath your greeting.",
                config.calm_zone.show_thread_widget,
                Message::SettingsToggleCalmThread,
            ),
            divider(tokens),
            column![
                text("Thread title")
                    .font(FONT_MEDIUM)
                    .size(12.5)
                    .color(primary_fg),
                clean_text_input(
                    tokens,
                    "Focus session ready",
                    &config.calm_zone.thread_title,
                    Message::SettingsCalmThreadTitleChanged,
                ),
            ]
            .spacing(6),
            column![
                text("Thread subtitle")
                    .font(FONT_MEDIUM)
                    .size(12.5)
                    .color(secondary_fg),
                clean_text_input(
                    tokens,
                    "Alt + Space to launch · Alt + ` for Spaces",
                    &config.calm_zone.thread_subtitle,
                    Message::SettingsCalmThreadSubtitleChanged,
                ),
            ]
            .spacing(6),
        ]
        .spacing(12)
        .into(),
    );

    column![
        section_header(
            tokens,
            "Calm Zone",
            "Customize the ambient desktop greeting and personal focus card.",
        ),
        greeting_card,
        thread_card,
    ]
    .spacing(14)
    .width(Length::Fill)
    .into()
}

fn view_system<'a>(state: &SettingsState<'a>) -> Element<'a, Message> {
    let tokens = state.tokens;
    let config = state.config;
    let primary_fg = color(tokens.text_primary);
    let secondary_fg = color(tokens.text_secondary);
    let b = state.primary_monitor.physical_bounds;
    let dpi_pct = (state.primary_monitor.dpi_scale * 100.0).round() as u32;

    let taskbar_card = section_card(
        tokens,
        "Windows shell integration",
        toggle_row(
            tokens,
            "Hide Windows Taskbar while running",
            "Uses a non-destructive overlay with an out-of-process recovery watchdog.",
            config.hide_native_taskbar,
            Message::SettingsToggleHideTaskbar,
        ),
    );

    let action_btn = |label: &'static str, desc: &'static str, msg: Message, danger: bool| {
        button(
            column![
                text(label)
                    .font(FONT_SEMIBOLD)
                    .size(12.5)
                    .color(if danger {
                        color(tokens.status_error)
                    } else {
                        primary_fg
                    }),
                text(desc).font(FONT_REGULAR).size(11).color(secondary_fg),
            ]
            .spacing(2),
        )
        .width(Length::FillPortion(1))
        .padding([10, 12])
        .style(surface_card_button(tokens, false, tokens.r12))
        .on_press(msg)
    };

    let actions_card = section_card(
        tokens,
        "Configuration & tools",
        row![
            action_btn(
                "Open config.toml",
                "Edit preferences in your editor",
                Message::SettingsOpenConfigFile,
                false,
            ),
            action_btn(
                "Windows Settings",
                "Open native OS preferences",
                Message::OpenTargetAndDismiss("ms-settings:".to_string()),
                false,
            ),
            action_btn(
                "Reset defaults",
                "Restore Horizon Light defaults",
                Message::SettingsResetDefaults,
                true,
            ),
        ]
        .spacing(10)
        .into(),
    );

    let diag_card = section_card(
        tokens,
        "Runtime diagnostics",
        column![
            mono_diag_row(tokens, "Config path", state.config_path),
            divider(tokens),
            mono_diag_row(
                tokens,
                "Primary monitor",
                &format!(
                    "{} ({}x{} @ {}%)",
                    state.primary_monitor.name.trim_start_matches(r"\\.\"),
                    b.width,
                    b.height,
                    dpi_pct
                ),
            ),
            divider(tokens),
            mono_diag_row(
                tokens,
                "Displays active",
                &format!("{} monitor(s)", state.monitor_count),
            ),
            divider(tokens),
            mono_diag_row(
                tokens,
                "Recovery CLI",
                "Ctrl+Alt+Shift+R  |  vita-shell.exe --restore-taskbar",
            ),
        ]
        .spacing(10)
        .into(),
    );

    column![
        section_header(
            tokens,
            "System & Safety",
            "Control taskbar overlay behavior, configuration files, and diagnostics.",
        ),
        taskbar_card,
        actions_card,
        diag_card,
    ]
    .spacing(14)
    .width(Length::Fill)
    .into()
}

fn section_header<'a>(
    tokens: &'a ThemeTokens,
    title: &'a str,
    subtitle: &'a str,
) -> Element<'a, Message> {
    column![
        text(title)
            .font(FONT_SEMIBOLD)
            .size(20)
            .color(color(tokens.text_primary)),
        text(subtitle)
            .font(FONT_REGULAR)
            .size(12.5)
            .color(color(tokens.text_secondary)),
    ]
    .spacing(4)
    .into()
}

fn section_card<'a>(
    tokens: &'a ThemeTokens,
    heading: &'a str,
    body: Element<'a, Message>,
) -> Element<'a, Message> {
    let tokens_copy = *tokens;
    container(
        column![
            text(heading)
                .font(FONT_SEMIBOLD)
                .size(13)
                .color(color(tokens.text_primary)),
            body,
        ]
        .spacing(12)
        .width(Length::Fill),
    )
    .width(Length::Fill)
    .padding([16, 18])
    .style(move |_| container::Style {
        background: Some(Background::Color(color(tokens_copy.surface_elevated))),
        border: Border {
            color: color(tokens_copy.border_control),
            width: 1.0,
            radius: Radius::from(tokens_copy.r16),
        },
        snap: true,
        ..container::Style::default()
    })
    .into()
}

fn metric_card<'a>(
    tokens: &'a ThemeTokens,
    label: &'a str,
    value: String,
    caption: String,
    mono_value: bool,
) -> Element<'a, Message> {
    let tokens_copy = *tokens;
    container(
        column![
            text(label)
                .font(FONT_MEDIUM)
                .size(11.5)
                .color(color(tokens.text_muted)),
            text(value)
                .font(if mono_value { FONT_MONO } else { FONT_SEMIBOLD })
                .size(if mono_value { 14 } else { 15 })
                .color(color(tokens.text_primary)),
            text(caption)
                .font(FONT_REGULAR)
                .size(11.5)
                .color(color(tokens.text_secondary)),
        ]
        .spacing(4),
    )
    .width(Length::FillPortion(1))
    .padding([14, 16])
    .style(move |_| container::Style {
        background: Some(Background::Color(color(tokens_copy.surface_elevated))),
        border: Border {
            color: color(tokens_copy.border_control),
            width: 1.0,
            radius: Radius::from(tokens_copy.r16),
        },
        snap: true,
        ..container::Style::default()
    })
    .into()
}

fn toggle_row<'a>(
    tokens: &'a ThemeTokens,
    label: &'a str,
    subtitle: &'a str,
    active: bool,
    on_press: Message,
) -> Element<'a, Message> {
    let track_bg = if active {
        color(tokens.accent_primary)
    } else {
        color(tokens.surface_subtle)
    };
    let track_border = if active {
        color(tokens.accent_primary)
    } else {
        color(tokens.border_control)
    };
    let thumb_color = if active {
        color(tokens.text_on_accent)
    } else {
        color(tokens.text_muted)
    };

    let thumb = container(Space::new().width(16).height(16)).style(move |_| container::Style {
        background: Some(Background::Color(thumb_color)),
        border: Border {
            radius: Radius::from(99.0),
            ..Border::default()
        },
        ..container::Style::default()
    });

    let switch_inner = if active {
        row![space::horizontal(), thumb].align_y(Alignment::Center)
    } else {
        row![thumb, space::horizontal()].align_y(Alignment::Center)
    };

    let switch_btn = button(
        container(switch_inner)
            .width(44)
            .height(24)
            .padding([0, 4])
            .align_y(Alignment::Center)
            .style(move |_| container::Style {
                background: Some(Background::Color(track_bg)),
                border: Border {
                    color: track_border,
                    width: 1.0,
                    radius: Radius::from(99.0),
                },
                snap: true,
                ..container::Style::default()
            }),
    )
    .padding(0)
    .style(|_, _| button::Style {
        background: None,
        ..button::Style::default()
    })
    .on_press(on_press);

    row![
        column![
            text(label)
                .font(FONT_MEDIUM)
                .size(13)
                .color(color(tokens.text_primary)),
            text(subtitle)
                .font(FONT_REGULAR)
                .size(11.5)
                .color(color(tokens.text_secondary)),
        ]
        .spacing(2)
        .width(Length::Fill),
        switch_btn,
    ]
    .spacing(12)
    .align_y(Alignment::Center)
    .width(Length::Fill)
    .into()
}

fn clean_text_input<'a>(
    tokens: &'a ThemeTokens,
    placeholder: &'a str,
    value: &'a str,
    on_input: impl Fn(String) -> Message + 'a,
) -> Element<'a, Message> {
    let tokens_copy = *tokens;
    let input = text_input(placeholder, value)
        .font(FONT_REGULAR)
        .size(13)
        .padding([7, 4])
        .width(Length::Fill)
        .on_input(on_input)
        .style(move |_theme, _status| text_input::Style {
            background: Background::Color(Color::TRANSPARENT),
            border: Border::default(),
            icon: color(tokens_copy.text_muted),
            placeholder: color(tokens_copy.text_muted),
            value: color(tokens_copy.text_primary),
            selection: color(tokens_copy.accent_soft),
        });

    container(input)
        .width(Length::Fill)
        .height(36)
        .padding([0, 10])
        .align_y(Alignment::Center)
        .style(move |_| container::Style {
            background: Some(Background::Color(color(tokens_copy.surface_base))),
            border: Border {
                color: color(tokens_copy.border_control),
                width: 1.0,
                radius: Radius::from(tokens_copy.r12),
            },
            snap: true,
            ..container::Style::default()
        })
        .into()
}

fn shortcut_row<'a>(
    tokens: &'a ThemeTokens,
    keys: &'a str,
    action: &'a str,
) -> Element<'a, Message> {
    let tokens_copy = *tokens;
    row![
        text(action)
            .font(FONT_REGULAR)
            .size(12.5)
            .color(color(tokens.text_primary)),
        space::horizontal(),
        container(
            text(keys)
                .font(FONT_MONO)
                .size(11.5)
                .color(color(tokens.text_secondary)),
        )
        .padding([4, 10])
        .style(move |_| container::Style {
            background: Some(Background::Color(color(tokens_copy.surface_subtle))),
            border: Border {
                color: color(tokens_copy.border_subtle),
                width: 1.0,
                radius: Radius::from(tokens_copy.r8),
            },
            snap: true,
            ..container::Style::default()
        }),
    ]
    .align_y(Alignment::Center)
    .width(Length::Fill)
    .into()
}

fn mono_diag_row<'a>(tokens: &'a ThemeTokens, key: &'a str, val: &str) -> Element<'a, Message> {
    row![
        text(key)
            .font(FONT_MEDIUM)
            .size(12)
            .color(color(tokens.text_secondary)),
        space::horizontal(),
        text(val.to_string())
            .font(FONT_MONO)
            .size(11.5)
            .color(color(tokens.text_primary)),
    ]
    .spacing(12)
    .align_y(Alignment::Center)
    .width(Length::Fill)
    .into()
}

fn divider<'a>(tokens: &'a ThemeTokens) -> Element<'a, Message> {
    let tokens_copy = *tokens;
    container(Space::new().width(Length::Fill).height(1))
        .width(Length::Fill)
        .height(1)
        .style(move |_| hairline_divider(&tokens_copy))
        .into()
}

fn sidebar_nav_button(
    tokens: &ThemeTokens,
    active: bool,
) -> impl Fn(&iced::Theme, button::Status) -> button::Style {
    let tokens = *tokens;
    move |_theme, status| {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        let bg = if active {
            Some(Background::Color(color(tokens.accent_soft)))
        } else if hovered {
            Some(Background::Color(color(tokens.surface_elevated.with_alpha(0.7))))
        } else {
            None
        };
        let border_color = if active {
            color(tokens.accent_primary.with_alpha(0.30))
        } else {
            Color::TRANSPARENT
        };
        button::Style {
            background: bg,
            text_color: color(tokens.text_primary),
            border: Border {
                color: border_color,
                width: 1.0,
                radius: Radius::from(tokens.r12),
            },
            ..button::Style::default()
        }
    }
}
