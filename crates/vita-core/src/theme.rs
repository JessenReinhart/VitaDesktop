use serde::{Deserialize, Serialize};

/// Linear-friendly RGBA color with channels normalized to `0.0..=1.0`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RgbaColor {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl RgbaColor {
    pub const fn rgba(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    pub fn from_hex(hex: &str) -> Self {
        let cleaned = hex.trim().trim_start_matches('#');
        let parse_byte = |slice: &str| u8::from_str_radix(slice, 16).unwrap_or(0) as f32 / 255.0;
        match cleaned.len() {
            6 => Self::rgba(
                parse_byte(&cleaned[0..2]),
                parse_byte(&cleaned[2..4]),
                parse_byte(&cleaned[4..6]),
                1.0,
            ),
            8 => Self::rgba(
                parse_byte(&cleaned[0..2]),
                parse_byte(&cleaned[2..4]),
                parse_byte(&cleaned[4..6]),
                parse_byte(&cleaned[6..8]),
            ),
            _ => Self::rgba(0.0, 0.0, 0.0, 1.0),
        }
    }

    pub const fn with_alpha(self, alpha: f32) -> Self {
        Self {
            r: self.r,
            g: self.g,
            b: self.b,
            a: alpha,
        }
    }

    /// WCAG 2.2 relative luminance calculation.
    pub fn relative_luminance(self) -> f32 {
        fn linearize(c: f32) -> f32 {
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        }
        0.2126 * linearize(self.r) + 0.7152 * linearize(self.g) + 0.0722 * linearize(self.b)
    }

    /// WCAG 2.2 contrast ratio between two opaque colors (`1.0 ..= 21.0`).
    pub fn contrast_ratio(self, other: Self) -> f32 {
        let l1 = self.relative_luminance();
        let l2 = other.relative_luminance();
        let (hi, lo) = if l1 >= l2 { (l1, l2) } else { (l2, l1) };
        (hi + 0.05) / (lo + 0.05)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ThemeMode {
    #[default]
    Light,
    Dark,
    System,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SurfaceMaterial {
    #[default]
    Matte,
    Frosted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum AccentPreset {
    #[default]
    Terracotta,
    Iris,
    Apricot,
    Lilac,
    Sky,
    Butter,
    Mist,
}

impl AccentPreset {
    pub const ALL: [Self; 7] = [
        Self::Terracotta,
        Self::Iris,
        Self::Apricot,
        Self::Lilac,
        Self::Sky,
        Self::Butter,
        Self::Mist,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Terracotta => "Terracotta",
            Self::Iris => "Iris",
            Self::Apricot => "Apricot",
            Self::Lilac => "Lilac",
            Self::Sky => "Sky",
            Self::Butter => "Butter",
            Self::Mist => "Mist",
        }
    }

    pub fn swatch_color(self) -> RgbaColor {
        match self {
            Self::Terracotta => primitives::TERRACOTTA_LIGHT,
            Self::Iris => primitives::IRIS_500,
            Self::Apricot => primitives::APRICOT,
            Self::Lilac => primitives::LILAC,
            Self::Sky => primitives::SKY,
            Self::Butter => primitives::BUTTER,
            Self::Mist => primitives::MIST,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ShellPreset {
    #[default]
    HorizonLight,
    HorizonDark,
    PantheonLight,
    PantheonDark,
    Custom,
}

impl ShellPreset {
    pub const BUILTINS: [Self; 4] = [
        Self::HorizonLight,
        Self::HorizonDark,
        Self::PantheonLight,
        Self::PantheonDark,
    ];

    pub fn next_cycle(self) -> Self {
        match self {
            Self::HorizonLight => Self::HorizonDark,
            Self::HorizonDark => Self::PantheonLight,
            Self::PantheonLight => Self::PantheonDark,
            Self::PantheonDark | Self::Custom => Self::HorizonLight,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::HorizonLight => "Horizon Light",
            Self::HorizonDark => "Horizon Dark",
            Self::PantheonLight => "Pantheon Light",
            Self::PantheonDark => "Pantheon Dark",
            Self::Custom => "Custom",
        }
    }

    pub fn subtitle(self) -> &'static str {
        match self {
            Self::HorizonLight => "Matte Pearl · Floating pills · Calm Zone",
            Self::HorizonDark => "Matte Deep Ink · Lilac · Floating pills",
            Self::PantheonLight => "Frosted Acrylic · Full bar · Classic dock",
            Self::PantheonDark => "Frosted Dark · Full bar · Sky accent",
            Self::Custom => "Customized workspace mix",
        }
    }
}

/// Dux Neutral Pearl -> Deep Ink primitives.
pub mod primitives {
    use super::RgbaColor;

    pub const PEARL_50: RgbaColor = RgbaColor::rgba(0.969, 0.969, 0.980, 1.0); // #F7F7FA
    pub const PEARL_100: RgbaColor = RgbaColor::rgba(0.941, 0.941, 0.965, 1.0); // #F0F0F6
    pub const PEARL_200: RgbaColor = RgbaColor::rgba(0.863, 0.867, 0.906, 1.0); // #DCDDE7
    pub const PEARL_300: RgbaColor = RgbaColor::rgba(0.745, 0.757, 0.824, 1.0); // #BEC1D2
    pub const INK_400: RgbaColor = RgbaColor::rgba(0.608, 0.616, 0.698, 1.0); // #9B9DB2
    pub const INK_500: RgbaColor = RgbaColor::rgba(0.498, 0.514, 0.612, 1.0); // #7F839C
    pub const INK_600: RgbaColor = RgbaColor::rgba(0.384, 0.404, 0.502, 1.0); // #626780
    pub const INK_700: RgbaColor = RgbaColor::rgba(0.290, 0.314, 0.416, 1.0); // #4A506A
    pub const INK_800: RgbaColor = RgbaColor::rgba(0.212, 0.235, 0.337, 1.0); // #363C56
    pub const INK_850: RgbaColor = RgbaColor::rgba(0.137, 0.153, 0.239, 1.0); // #23273D
    pub const INK_900: RgbaColor = RgbaColor::rgba(0.161, 0.176, 0.278, 1.0); // #292D47
    pub const INK_950: RgbaColor = RgbaColor::rgba(0.098, 0.110, 0.184, 1.0); // #191C2F
    pub const WHITE: RgbaColor = RgbaColor::rgba(1.0, 1.0, 1.0, 1.0);

    pub const IRIS_300: RgbaColor = RgbaColor::rgba(0.780, 0.776, 1.0, 1.0); // #C7C6FF
    pub const IRIS_500: RgbaColor = RgbaColor::rgba(0.514, 0.522, 0.824, 1.0); // #8385D2
    pub const IRIS_700: RgbaColor = RgbaColor::rgba(0.325, 0.345, 0.651, 1.0); // #5358A6

    pub const TERRACOTTA_LIGHT: RgbaColor = RgbaColor::rgba(0.761, 0.294, 0.137, 1.0); // #C24B23 (>=4.8:1 vs white)
    pub const TERRACOTTA_DARK: RgbaColor = RgbaColor::rgba(0.980, 0.710, 0.651, 1.0); // #FAB5A6

    pub const APRICOT: RgbaColor = RgbaColor::rgba(0.973, 0.788, 0.655, 1.0); // #F8C9A7
    pub const LILAC: RgbaColor = RgbaColor::rgba(0.867, 0.827, 0.961, 1.0); // #DDD3F5
    pub const SKY: RgbaColor = RgbaColor::rgba(0.792, 0.863, 0.969, 1.0); // #CADCF7
    pub const BUTTER: RgbaColor = RgbaColor::rgba(0.961, 0.886, 0.639, 1.0); // #F5E2A3
    pub const MIST: RgbaColor = RgbaColor::rgba(0.788, 0.902, 0.886, 1.0); // #C9E6E2

    // High-contrast accessible foreground accents for light backgrounds (>= 3:1 UI / >= 4.5:1 text)
    pub const CORAL_LIGHT_TEXT: RgbaColor = RgbaColor::rgba(0.678, 0.259, 0.224, 1.0); // #AD4239
    pub const AMBER_LIGHT_TEXT: RgbaColor = RgbaColor::rgba(0.557, 0.431, 0.063, 1.0); // #8E6E10
    pub const SAGE_LIGHT_TEXT: RgbaColor = RgbaColor::rgba(0.137, 0.392, 0.294, 1.0); // #23644B
    pub const SAGE_DARK_TEXT: RgbaColor = RgbaColor::rgba(0.545, 0.827, 0.694, 1.0); // #8BD3B1
}

/// Complete resolved token set driving every widget and surface in `vita-shell`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThemeTokens {
    pub is_dark: bool,
    pub material: SurfaceMaterial,
    pub high_contrast: bool,

    // Surfaces
    pub surface_base: RgbaColor,
    pub surface_elevated: RgbaColor,
    pub surface_subtle: RgbaColor,
    pub surface_topbar: RgbaColor,
    pub surface_dock: RgbaColor,
    pub surface_tooltip: RgbaColor,

    // Borders & Focus
    pub border_control: RgbaColor,
    pub border_subtle: RgbaColor,
    pub border_focus: RgbaColor,
    pub divider: RgbaColor,

    // Typography
    pub text_primary: RgbaColor,
    pub text_secondary: RgbaColor,
    pub text_muted: RgbaColor,
    pub text_on_accent: RgbaColor,

    // Accents & Dock Indicators
    pub accent_primary: RgbaColor,
    pub accent_warm: RgbaColor,
    pub accent_soft: RgbaColor,
    pub dock_dot_focused: RgbaColor,
    pub dock_dot_running: RgbaColor,

    // Semantic Status
    pub status_error: RgbaColor,
    pub status_caution: RgbaColor,
    pub status_success: RgbaColor,

    // Shadow
    pub shadow_color: RgbaColor,

    // Radii (4-unit Horizon system)
    pub r8: f32,
    pub r12: f32,
    pub r16: f32,
    pub r24: f32,
    pub pill: f32,

    // Spacing (4-unit grid)
    pub s4: f32,
    pub s8: f32,
    pub s12: f32,
    pub s16: f32,
    pub s20: f32,
    pub s24: f32,
}

impl ThemeTokens {
    /// Resolves semantic tokens for the requested mode, accent, material, and accessibility settings.
    pub fn resolve(
        mode: ThemeMode,
        system_prefers_dark: bool,
        accent: AccentPreset,
        material: SurfaceMaterial,
        high_contrast: bool,
    ) -> Self {
        use primitives::*;

        let is_dark = match mode {
            ThemeMode::Light => false,
            ThemeMode::Dark => true,
            ThemeMode::System => system_prefers_dark,
        };

        let (accent_swatch, accent_interactive_light, accent_interactive_dark) = match accent {
            AccentPreset::Terracotta => (TERRACOTTA_LIGHT, TERRACOTTA_LIGHT, TERRACOTTA_DARK),
            AccentPreset::Iris => (IRIS_700, IRIS_700, IRIS_300),
            AccentPreset::Apricot => (APRICOT, TERRACOTTA_LIGHT, APRICOT),
            AccentPreset::Lilac => (LILAC, IRIS_700, LILAC),
            AccentPreset::Sky => (SKY, IRIS_700, SKY),
            AccentPreset::Butter => (BUTTER, AMBER_LIGHT_TEXT, BUTTER),
            AccentPreset::Mist => (MIST, SAGE_LIGHT_TEXT, MIST),
        };

        let (surface_base, surface_elevated, surface_subtle) = if is_dark {
            (INK_950, INK_850, INK_900)
        } else {
            (PEARL_50, WHITE, PEARL_100)
        };

        let (surface_topbar, surface_dock) = match (is_dark, material) {
            (false, SurfaceMaterial::Matte) => (PEARL_50, PEARL_50),
            (true, SurfaceMaterial::Matte) => (INK_950, INK_950),
            (false, SurfaceMaterial::Frosted) => {
                (WHITE.with_alpha(0.84), WHITE.with_alpha(0.82))
            }
            (true, SurfaceMaterial::Frosted) => {
                (INK_950.with_alpha(0.78), INK_950.with_alpha(0.76))
            }
        };

        let (border_control, border_subtle, divider) = match (is_dark, high_contrast) {
            (false, false) => (
                PEARL_200,
                if matches!(material, SurfaceMaterial::Frosted) {
                    WHITE.with_alpha(0.55)
                } else {
                    PEARL_200.with_alpha(0.7)
                },
                PEARL_200,
            ),
            (false, true) => (INK_500, INK_500, INK_500),
            (true, false) => (INK_800, INK_800.with_alpha(0.75), INK_800),
            (true, true) => (PEARL_300, PEARL_300, PEARL_300),
        };

        let (text_primary, text_secondary, text_muted) = if is_dark {
            (
                PEARL_50,
                PEARL_300,
                if high_contrast { PEARL_300 } else { INK_400 },
            )
        } else {
            (
                INK_950,
                INK_700,
                if high_contrast { INK_700 } else { INK_600 },
            )
        };

        let accent_primary = if is_dark {
            accent_interactive_dark
        } else {
            accent_interactive_light
        };

        // Pick dark or light text on accent background automatically for >= 4.5:1 contrast.
        let text_on_accent = if accent_primary.contrast_ratio(INK_950) >= accent_primary.contrast_ratio(WHITE) {
            INK_950
        } else {
            WHITE
        };

        let accent_warm = if is_dark { TERRACOTTA_DARK } else { TERRACOTTA_LIGHT };
        let accent_soft = accent_swatch.with_alpha(if is_dark { 0.20 } else { 0.16 });
        let border_focus = if is_dark { IRIS_300 } else { IRIS_700 };

        Self {
            is_dark,
            material,
            high_contrast,
            surface_base,
            surface_elevated,
            surface_subtle,
            surface_topbar,
            surface_dock,
            surface_tooltip: if is_dark { INK_850 } else { INK_950 },
            border_control,
            border_subtle,
            border_focus,
            divider,
            text_primary,
            text_secondary,
            text_muted,
            text_on_accent,
            accent_primary,
            accent_warm,
            accent_soft,
            dock_dot_focused: accent_warm,
            dock_dot_running: if is_dark { PEARL_300 } else { INK_500 },
            status_error: if is_dark { TERRACOTTA_DARK } else { CORAL_LIGHT_TEXT },
            status_caution: if is_dark { BUTTER } else { AMBER_LIGHT_TEXT },
            status_success: if is_dark { SAGE_DARK_TEXT } else { SAGE_LIGHT_TEXT },
            shadow_color: if is_dark {
                RgbaColor::rgba(0.02, 0.03, 0.08, 0.42)
            } else {
                INK_950.with_alpha(0.12)
            },
            r8: 8.0,
            r12: 12.0,
            r16: 16.0,
            r24: 24.0,
            pill: 9999.0,
            s4: 4.0,
            s8: 8.0,
            s12: 12.0,
            s16: 16.0,
            s20: 20.0,
            s24: 24.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wcag_text_and_focus_contrast_passes_all_presets() {
        for mode in [ThemeMode::Light, ThemeMode::Dark] {
            for high_contrast in [false, true] {
                for accent in AccentPreset::ALL {
                    let tokens = ThemeTokens::resolve(
                        mode,
                        false,
                        accent,
                        SurfaceMaterial::Matte,
                        high_contrast,
                    );

                    let primary_ratio = tokens.text_primary.contrast_ratio(tokens.surface_base);
                    assert!(
                        primary_ratio >= 10.0,
                        "Expected >= 10:1 primary contrast for {mode:?}/{accent:?}, got {primary_ratio:.2}"
                    );

                    let secondary_ratio = tokens.text_secondary.contrast_ratio(tokens.surface_base);
                    assert!(
                        secondary_ratio >= 4.5,
                        "Expected >= 4.5:1 secondary contrast for {mode:?}/{accent:?}, got {secondary_ratio:.2}"
                    );

                    let focus_ratio = tokens.border_focus.contrast_ratio(tokens.surface_base);
                    assert!(
                        focus_ratio >= 3.0,
                        "Expected >= 3.0:1 focus contrast for {mode:?}/{accent:?}, got {focus_ratio:.2}"
                    );

                    let on_accent_ratio = tokens.text_on_accent.contrast_ratio(tokens.accent_primary);
                    assert!(
                        on_accent_ratio >= 4.5,
                        "Expected >= 4.5:1 on-accent contrast for {mode:?}/{accent:?}, got {on_accent_ratio:.2}"
                    );

                    if high_contrast {
                        let hc_border_ratio =
                            tokens.border_control.contrast_ratio(tokens.surface_base);
                        assert!(
                            hc_border_ratio >= 3.0,
                            "Expected >= 3.0:1 HC border contrast for {mode:?}/{accent:?}, got {hc_border_ratio:.2}"
                        );
                    }
                }
            }
        }
    }
}
