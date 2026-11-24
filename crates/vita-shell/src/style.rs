use iced::border::Radius;
use iced::font::{Family, Weight};
use iced::widget::{button, container, scrollable, slider};
use iced::{Background, Border, Color, Font, Shadow, Vector};
use vita_core::{RgbaColor, SurfaceMaterial, ThemeTokens};

pub const FONT_POPPINS_REGULAR_BYTES: &[u8] =
    include_bytes!("../../../assets/fonts/Poppins-Regular.ttf");
pub const FONT_POPPINS_MEDIUM_BYTES: &[u8] =
    include_bytes!("../../../assets/fonts/Poppins-Medium.ttf");
pub const FONT_POPPINS_SEMIBOLD_BYTES: &[u8] =
    include_bytes!("../../../assets/fonts/Poppins-SemiBold.ttf");
pub const FONT_JETBRAINS_MONO_BYTES: &[u8] =
    include_bytes!("../../../assets/fonts/JetBrainsMono.ttf");

pub const FONT_REGULAR: Font = Font::with_name("Poppins");
pub const FONT_MEDIUM: Font = Font {
    family: Family::Name("Poppins"),
    weight: Weight::Medium,
    ..Font::DEFAULT
};
pub const FONT_SEMIBOLD: Font = Font {
    family: Family::Name("Poppins"),
    weight: Weight::Semibold,
    ..Font::DEFAULT
};
pub const FONT_MONO: Font = Font::with_name("JetBrains Mono");

pub const fn color(c: RgbaColor) -> Color {
    Color {
        r: c.r,
        g: c.g,
        b: c.b,
        a: c.a,
    }
}

pub fn full_width_topbar_style(tokens: &ThemeTokens) -> container::Style {
    container::Style {
        background: Some(Background::Color(color(tokens.surface_topbar))),
        text_color: Some(color(tokens.text_primary)),
        border: Border {
            color: color(tokens.border_subtle),
            width: 1.0,
            radius: Radius::from(0.0),
        },
        shadow: Shadow::default(),
        snap: true,
    }
}

pub fn floating_pill_surface(tokens: &ThemeTokens, radius: f32) -> container::Style {
    container::Style {
        background: Some(Background::Color(color(tokens.surface_topbar))),
        text_color: Some(color(tokens.text_primary)),
        border: Border {
            color: color(tokens.border_control),
            width: 1.0,
            radius: Radius::from(radius),
        },
        shadow: Shadow {
            color: color(tokens.shadow_color),
            offset: Vector::new(0.0, 4.0),
            blur_radius: 14.0,
        },
        snap: true,
    }
}

pub fn dock_pill_surface(
    tokens: &ThemeTokens,
    dock: &vita_core::DockConfig,
    reveal: f32,
) -> container::Style {
    use vita_core::DockBackgroundStyle;

    let base = match dock.background_style {
        DockBackgroundStyle::ThemeDefault => tokens.surface_dock,
        DockBackgroundStyle::FrostedGlass => {
            if tokens.is_dark {
                RgbaColor::from_hex("#191C2F")
            } else {
                RgbaColor::from_hex("#FFFFFF")
            }
        }
        DockBackgroundStyle::DeepInk => RgbaColor::from_hex("#16192B"),
        DockBackgroundStyle::Pearl => RgbaColor::from_hex("#F4F5F9"),
        DockBackgroundStyle::AccentTint => RgbaColor::rgba(
            tokens.surface_base.r * 0.76 + tokens.accent_primary.r * 0.24,
            tokens.surface_base.g * 0.76 + tokens.accent_primary.g * 0.24,
            tokens.surface_base.b * 0.76 + tokens.accent_primary.b * 0.24,
            1.0,
        ),
        DockBackgroundStyle::CustomColor => RgbaColor::from_hex(&dock.background_color_hex),
        DockBackgroundStyle::Transparent => tokens.surface_dock.with_alpha(0.0),
    };

    let base_alpha = if dock.background_style == DockBackgroundStyle::Transparent {
        0.0
    } else {
        (dock.background_opacity.clamp(0, 100) as f32) / 100.0
    };
    let reveal = reveal.clamp(0.0, 1.0);
    let alpha = base_alpha * reveal;
    let bg = base.with_alpha(alpha);
    let max_radius = (dock.effective_pill_height() * 0.5).floor();
    let radius = (dock.corner_radius.clamp(6, 36) as f32).min(max_radius);
    let border_width = if dock.show_border && alpha > 0.02 {
        1.0
    } else {
        0.0
    };
    let border_color = tokens
        .border_control
        .with_alpha(alpha.clamp(0.25, 1.0) * reveal);
    let shadow_alpha = tokens.shadow_color.a * alpha;

    container::Style {
        background: (alpha > 0.005).then_some(Background::Color(color(bg))),
        text_color: Some(color(tokens.text_primary.scale_alpha(reveal))),
        border: Border {
            color: color(border_color),
            width: border_width,
            radius: Radius::from(radius),
        },
        shadow: Shadow {
            color: color(tokens.shadow_color.with_alpha(shadow_alpha)),
            offset: Vector::new(0.0, 8.0),
            blur_radius: 22.0,
        },
        snap: true,
    }
}

pub fn popover_card_surface(tokens: &ThemeTokens, radius: f32) -> container::Style {
    let bg = match tokens.material {
        SurfaceMaterial::Matte => tokens.surface_base,
        SurfaceMaterial::Frosted => tokens.surface_dock.with_alpha(0.92),
    };
    container::Style {
        background: Some(Background::Color(color(bg))),
        text_color: Some(color(tokens.text_primary)),
        border: Border {
            color: color(tokens.border_control),
            width: 1.0,
            radius: Radius::from(radius),
        },
        shadow: Shadow {
            color: color(tokens.shadow_color),
            offset: Vector::new(0.0, 10.0),
            blur_radius: 26.0,
        },
        snap: true,
    }
}

pub fn tooltip_style(tokens: &ThemeTokens, opacity: f32) -> container::Style {
    let op = opacity.clamp(0.0, 1.0);
    container::Style {
        background: Some(Background::Color(color(
            tokens.surface_tooltip.scale_alpha(op),
        ))),
        text_color: Some(Color::from_rgba(0.969, 0.969, 0.980, op)),
        border: Border {
            color: color(tokens.border_subtle.scale_alpha(op)),
            width: 1.0,
            radius: Radius::from(tokens.r8),
        },
        shadow: Shadow {
            color: color(tokens.shadow_color.scale_alpha(op)),
            offset: Vector::new(0.0, 3.0),
            blur_radius: 10.0,
        },
        snap: true,
    }
}

pub fn transparent_container() -> container::Style {
    container::Style {
        background: None,
        border: Border::default(),
        shadow: Shadow::default(),
        text_color: None,
        snap: false,
    }
}

pub fn hairline_divider(tokens: &ThemeTokens) -> container::Style {
    container::Style {
        background: Some(Background::Color(color(tokens.divider))),
        ..container::Style::default()
    }
}

pub fn subtle_pill_button(
    tokens: &ThemeTokens,
    active: bool,
    radius: f32,
) -> impl Fn(&iced::Theme, button::Status) -> button::Style {
    let tokens = *tokens;
    move |_theme, status| {
        let bg = match (active, status) {
            (true, _) => Some(Background::Color(color(tokens.accent_soft))),
            (false, button::Status::Hovered | button::Status::Pressed) => {
                Some(Background::Color(color(tokens.surface_subtle)))
            }
            _ => None,
        };
        let border_color = match (active, status) {
            (true, _) => color(tokens.accent_primary.with_alpha(0.35)),
            _ => Color::TRANSPARENT,
        };
        button::Style {
            background: bg,
            text_color: color(tokens.text_primary),
            border: Border {
                color: border_color,
                width: if active { 1.0 } else { 0.0 },
                radius: Radius::from(radius),
            },
            shadow: Shadow::default(),
            snap: true,
        }
    }
}

pub fn dock_icon_button() -> impl Fn(&iced::Theme, button::Status) -> button::Style {
    |_theme, _status| button::Style {
        background: None,
        text_color: Color::WHITE,
        border: Border::default(),
        shadow: Shadow::default(),
        snap: false,
    }
}

pub fn fallback_app_tile_style(tokens: &ThemeTokens) -> container::Style {
    container::Style {
        background: Some(Background::Color(color(tokens.surface_elevated))),
        text_color: Some(color(tokens.text_primary)),
        border: Border {
            color: color(tokens.border_control),
            width: 1.0,
            radius: Radius::from(tokens.r12),
        },
        shadow: Shadow::default(),
        snap: true,
    }
}

pub fn segmented_shell_style(tokens: &ThemeTokens) -> container::Style {
    container::Style {
        background: Some(Background::Color(color(tokens.surface_subtle))),
        border: Border {
            color: color(tokens.border_control),
            width: 1.0,
            radius: Radius::from(tokens.r12),
        },
        snap: true,
        ..container::Style::default()
    }
}

pub fn segmented_item_button(
    tokens: &ThemeTokens,
    active: bool,
) -> impl Fn(&iced::Theme, button::Status) -> button::Style {
    let tokens = *tokens;
    move |_theme, status| {
        let bg = if active {
            Some(Background::Color(color(tokens.surface_elevated)))
        } else if matches!(status, button::Status::Hovered | button::Status::Pressed) {
            Some(Background::Color(color(tokens.surface_elevated.with_alpha(0.55))))
        } else {
            None
        };
        button::Style {
            background: bg,
            text_color: color(if active {
                tokens.text_primary
            } else {
                tokens.text_secondary
            }),
            border: Border {
                color: if active {
                    color(tokens.border_control)
                } else {
                    Color::TRANSPARENT
                },
                width: if active { 1.0 } else { 0.0 },
                radius: Radius::from(tokens.r8),
            },
            shadow: Shadow::default(),
            snap: true,
        }
    }
}

pub fn launcher_tile_button(
    tokens: &ThemeTokens,
    selected: bool,
) -> impl Fn(&iced::Theme, button::Status) -> button::Style {
    let tokens = *tokens;
    move |_theme, status| {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        let bg = if selected {
            Some(Background::Color(color(tokens.accent_soft)))
        } else if hovered {
            Some(Background::Color(color(tokens.surface_subtle)))
        } else {
            None
        };
        button::Style {
            background: bg,
            text_color: color(tokens.text_primary),
            border: Border {
                color: if selected {
                    color(tokens.border_focus)
                } else {
                    Color::TRANSPARENT
                },
                width: if selected { 1.5 } else { 0.0 },
                radius: Radius::from(tokens.r16),
            },
            shadow: Shadow::default(),
            snap: true,
        }
    }
}

pub fn menu_row_button(
    tokens: &ThemeTokens,
    destructive: bool,
) -> impl Fn(&iced::Theme, button::Status) -> button::Style {
    let tokens = *tokens;
    move |_theme, status| {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        let bg = if hovered {
            let c = if destructive {
                tokens.status_error.with_alpha(0.12)
            } else {
                tokens.surface_subtle
            };
            Some(Background::Color(color(c)))
        } else {
            None
        };
        let fg = if destructive {
            tokens.status_error
        } else {
            tokens.text_primary
        };
        button::Style {
            background: bg,
            text_color: color(fg),
            border: Border {
                radius: Radius::from(tokens.r8),
                ..Border::default()
            },
            shadow: Shadow::default(),
            snap: true,
        }
    }
}

pub fn calm_scrollable_style(
    tokens: &ThemeTokens,
) -> impl Fn(&iced::Theme, scrollable::Status) -> scrollable::Style {
    let tokens = *tokens;
    move |theme, status| {
        let mut base = scrollable::default(theme, status);
        base.container = transparent_container();
        base.vertical_rail.background = None;
        base.vertical_rail.border = Border::default();
        base.vertical_rail.scroller.background = Background::Color(color(tokens.border_control));
        base.vertical_rail.scroller.border = Border {
            radius: Radius::from(4.0),
            ..Border::default()
        };
        base
    }
}

pub fn quick_tile_button(
    tokens: &ThemeTokens,
    active: bool,
) -> impl Fn(&iced::Theme, button::Status) -> button::Style {
    let tokens = *tokens;
    move |_theme, status| {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        let bg = if active {
            color(tokens.accent_soft)
        } else if hovered {
            color(tokens.surface_subtle)
        } else {
            color(tokens.surface_elevated)
        };
        let border_color = if active {
            color(tokens.accent_primary.with_alpha(0.42))
        } else {
            color(tokens.border_control)
        };
        button::Style {
            background: Some(Background::Color(bg)),
            text_color: color(tokens.text_primary),
            border: Border {
                color: border_color,
                width: 1.0,
                radius: Radius::from(tokens.r16),
            },
            shadow: Shadow::default(),
            snap: true,
        }
    }
}

pub fn surface_card_button(
    tokens: &ThemeTokens,
    active: bool,
    radius: f32,
) -> impl Fn(&iced::Theme, button::Status) -> button::Style {
    let tokens = *tokens;
    move |_theme, status| {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        let bg = if active {
            color(tokens.accent_soft)
        } else if hovered {
            color(tokens.surface_subtle)
        } else {
            color(tokens.surface_elevated)
        };
        button::Style {
            background: Some(Background::Color(bg)),
            text_color: color(tokens.text_primary),
            border: Border {
                color: color(if active {
                    tokens.accent_primary.with_alpha(0.45)
                } else {
                    tokens.border_control
                }),
                width: 1.0,
                radius: Radius::from(radius),
            },
            shadow: Shadow::default(),
            snap: true,
        }
    }
}

pub fn warm_accent_slider_style(
    tokens: &ThemeTokens,
) -> impl Fn(&iced::Theme, slider::Status) -> slider::Style {
    let tokens = *tokens;
    move |theme, status| {
        let mut base = slider::default(theme, status);
        base.rail.backgrounds = (
            Background::Color(color(tokens.accent_warm)),
            Background::Color(color(tokens.surface_subtle)),
        );
        base.rail.width = 6.0;
        base.rail.border = Border {
            color: color(tokens.border_subtle),
            width: 0.5,
            radius: Radius::from(3.0),
        };
        base.handle = slider::Handle {
            shape: slider::HandleShape::Circle { radius: 8.0 },
            background: Background::Color(color(tokens.surface_elevated)),
            border_width: 2.0,
            border_color: color(tokens.accent_warm),
        };
        base
    }
}
