use crate::theme::{AccentPreset, ShellPreset, SurfaceMaterial, ThemeMode};
use directories::BaseDirs;
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TopBarStyle {
    #[default]
    FloatingPills,
    FullWidth,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TopBarConfig {
    pub style: TopBarStyle,
    pub height: u32,
    pub show_seconds: bool,
    pub show_global_menu: bool,
}

impl Default for TopBarConfig {
    fn default() -> Self {
        Self {
            style: TopBarStyle::FloatingPills,
            height: 46,
            show_seconds: false,
            show_global_menu: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PinnedApp {
    pub name: String,
    pub target: String,
    pub exe_name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum DockVisibility {
    #[default]
    IntelliHide,
    AlwaysVisible,
    AutoHide,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum DockBackgroundStyle {
    #[default]
    ThemeDefault,
    FrostedGlass,
    DeepInk,
    Pearl,
    AccentTint,
    CustomColor,
    Transparent,
}

impl DockBackgroundStyle {
    pub const ALL: [Self; 7] = [
        Self::ThemeDefault,
        Self::FrostedGlass,
        Self::DeepInk,
        Self::Pearl,
        Self::AccentTint,
        Self::CustomColor,
        Self::Transparent,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::ThemeDefault => "Match theme",
            Self::FrostedGlass => "Frosted glass",
            Self::DeepInk => "Deep Ink",
            Self::Pearl => "Pearl Light",
            Self::AccentTint => "Accent tint",
            Self::CustomColor => "Custom color",
            Self::Transparent => "Clear",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DockConfig {
    pub icon_size: u32,
    pub height: u32,
    pub bottom_margin: u32,
    pub reserve_work_area: bool,
    pub background_style: DockBackgroundStyle,
    pub background_opacity: u8,
    pub background_color_hex: String,
    pub corner_radius: u32,
    pub show_border: bool,
    pub magnification_scale: f32,
    pub max_magnify_radius: f32,
    pub show_space_chip: bool,
    pub show_resume_chip: bool,
    pub auto_hide: bool,
    pub visibility: DockVisibility,
    pub all_monitors: bool,
    pub pinned_apps: Vec<PinnedApp>,
}

impl Default for DockConfig {
    fn default() -> Self {
        Self {
            icon_size: 48,
            height: 60,
            bottom_margin: 10,
            reserve_work_area: false,
            background_style: DockBackgroundStyle::ThemeDefault,
            background_opacity: 94,
            background_color_hex: "#1E2238".to_string(),
            corner_radius: 22,
            show_border: true,
            magnification_scale: 1.38,
            max_magnify_radius: 128.0,
            show_space_chip: true,
            show_resume_chip: true,
            auto_hide: false,
            visibility: DockVisibility::IntelliHide,
            all_monitors: true,
            pinned_apps: default_pinned_apps(),
        }
    }
}

impl DockConfig {
    pub fn effective_pill_height(&self) -> f32 {
        let min_for_icon = self.icon_size.clamp(28, 64) + 8;
        self.height.clamp(36, 84).max(min_for_icon) as f32
    }
}

pub fn default_pinned_apps() -> Vec<PinnedApp> {
    vec![
        PinnedApp {
            name: "Files".to_string(),
            target: "explorer.exe".to_string(),
            exe_name: "explorer.exe".to_string(),
        },
        PinnedApp {
            name: "Edge".to_string(),
            target: "msedge.exe".to_string(),
            exe_name: "msedge.exe".to_string(),
        },
        PinnedApp {
            name: "Terminal".to_string(),
            target: "wt.exe".to_string(),
            exe_name: "WindowsTerminal.exe".to_string(),
        },
        PinnedApp {
            name: "Notepad".to_string(),
            target: "notepad.exe".to_string(),
            exe_name: "notepad.exe".to_string(),
        },
        PinnedApp {
            name: "Settings".to_string(),
            target: "ms-settings:".to_string(),
            exe_name: "SystemSettings.exe".to_string(),
        },
    ]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum LauncherViewMode {
    #[default]
    Grid,
    Category,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LauncherConfig {
    pub default_view: LauncherViewMode,
    pub cols: usize,
    pub rows: usize,
}

impl Default for LauncherConfig {
    fn default() -> Self {
        Self {
            default_view: LauncherViewMode::Grid,
            cols: 5,
            rows: 3,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CalmZoneConfig {
    pub enabled: bool,
    pub user_name: String,
    pub show_thread_widget: bool,
    pub thread_title: String,
    pub thread_subtitle: String,
}

impl Default for CalmZoneConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            user_name: String::new(),
            show_thread_widget: true,
            thread_title: "Weekend ideas".to_string(),
            thread_subtitle: "Notes  ·  Ready when you are".to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AccessibilityConfig {
    pub reduce_motion: bool,
    pub increase_contrast: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum DecoratorButtonStyle {
    #[default]
    Horizon,
    TrafficLights,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowDecoratorConfig {
    pub enabled: bool,
    pub dwm_theme_sync: bool,
    pub custom_overlay: bool,
    pub maximized_topbar_controls: bool,
    pub button_style: DecoratorButtonStyle,
}

impl Default for WindowDecoratorConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            dwm_theme_sync: true,
            custom_overlay: true,
            maximized_topbar_controls: true,
            button_style: DecoratorButtonStyle::Horizon,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ShellConfig {
    pub preset: ShellPreset,
    pub theme_mode: ThemeMode,
    pub accent: AccentPreset,
    pub surface_material: SurfaceMaterial,
    pub hide_native_taskbar: bool,
    pub top_bar: TopBarConfig,
    pub dock: DockConfig,
    pub launcher: LauncherConfig,
    pub calm_zone: CalmZoneConfig,
    pub decorator: WindowDecoratorConfig,
    pub accessibility: AccessibilityConfig,
}

impl Default for ShellConfig {
    fn default() -> Self {
        Self {
            preset: ShellPreset::HorizonLight,
            theme_mode: ThemeMode::Light,
            accent: AccentPreset::Terracotta,
            surface_material: SurfaceMaterial::Matte,
            hide_native_taskbar: true,
            top_bar: TopBarConfig::default(),
            dock: DockConfig::default(),
            launcher: LauncherConfig::default(),
            calm_zone: CalmZoneConfig::default(),
            decorator: WindowDecoratorConfig::default(),
            accessibility: AccessibilityConfig::default(),
        }
    }
}

impl ShellConfig {
    pub fn config_dir() -> PathBuf {
        BaseDirs::new()
            .map(|b| b.config_dir().join("VitaDesktop"))
            .unwrap_or_else(|| PathBuf::from("."))
    }

    pub fn config_path() -> PathBuf {
        Self::config_dir().join("config.toml")
    }

    pub fn from_toml_or_default(contents: &str) -> Self {
        toml::from_str(contents).unwrap_or_default()
    }

    pub fn load_or_default() -> Self {
        let path = Self::config_path();
        match fs::read_to_string(&path) {
            Ok(raw) => match toml::from_str::<Self>(&raw) {
                Ok(cfg) => cfg,
                Err(err) => {
                    tracing::warn!(?err, "Malformed config.toml, falling back to default");
                    Self::default()
                }
            },
            Err(_) => {
                let cfg = Self::default();
                let _ = cfg.save_atomic_to(&path);
                cfg
            }
        }
    }

    pub fn save_atomic(&self) -> std::io::Result<()> {
        self.save_atomic_to(&Self::config_path())
    }

    pub fn save_atomic_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let body = toml::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let tmp = path.with_extension("toml.tmp");
        fs::write(&tmp, body)?;
        fs::rename(tmp, path)?;
        Ok(())
    }

    pub fn apply_preset(&mut self, preset: ShellPreset) {
        self.preset = preset;
        match preset {
            ShellPreset::HorizonLight => {
                self.theme_mode = ThemeMode::Light;
                self.accent = AccentPreset::Terracotta;
                self.surface_material = SurfaceMaterial::Matte;
                self.top_bar.style = TopBarStyle::FloatingPills;
                self.top_bar.height = 46;
                self.dock.show_space_chip = true;
                self.dock.show_resume_chip = true;
                self.dock.magnification_scale = 1.38;
                self.dock.background_style = DockBackgroundStyle::ThemeDefault;
                self.dock.background_opacity = 96;
                self.calm_zone.enabled = true;
            }
            ShellPreset::HorizonDark => {
                self.theme_mode = ThemeMode::Dark;
                self.accent = AccentPreset::Lilac;
                self.surface_material = SurfaceMaterial::Matte;
                self.top_bar.style = TopBarStyle::FloatingPills;
                self.top_bar.height = 46;
                self.dock.show_space_chip = true;
                self.dock.show_resume_chip = true;
                self.dock.magnification_scale = 1.38;
                self.dock.background_style = DockBackgroundStyle::ThemeDefault;
                self.dock.background_opacity = 96;
                self.calm_zone.enabled = true;
            }
            ShellPreset::PantheonLight => {
                self.theme_mode = ThemeMode::Light;
                self.accent = AccentPreset::Iris;
                self.surface_material = SurfaceMaterial::Frosted;
                self.top_bar.style = TopBarStyle::FullWidth;
                self.top_bar.height = 34;
                self.dock.show_space_chip = false;
                self.dock.show_resume_chip = false;
                self.dock.magnification_scale = 1.42;
                self.dock.background_style = DockBackgroundStyle::FrostedGlass;
                self.dock.background_opacity = 82;
                self.calm_zone.enabled = false;
            }
            ShellPreset::PantheonDark => {
                self.theme_mode = ThemeMode::Dark;
                self.accent = AccentPreset::Sky;
                self.surface_material = SurfaceMaterial::Frosted;
                self.top_bar.style = TopBarStyle::FullWidth;
                self.top_bar.height = 34;
                self.dock.show_space_chip = false;
                self.dock.show_resume_chip = false;
                self.dock.magnification_scale = 1.42;
                self.dock.background_style = DockBackgroundStyle::FrostedGlass;
                self.dock.background_opacity = 82;
                self.calm_zone.enabled = false;
            }
            ShellPreset::Custom => {}
        }
    }

    pub fn toggle_light_dark(&mut self) {
        let next_dark = match self.theme_mode {
            ThemeMode::Light => true,
            ThemeMode::Dark | ThemeMode::System => false,
        };
        match (self.top_bar.style, next_dark) {
            (TopBarStyle::FloatingPills, false) => self.apply_preset(ShellPreset::HorizonLight),
            (TopBarStyle::FloatingPills, true) => self.apply_preset(ShellPreset::HorizonDark),
            (TopBarStyle::FullWidth, false) => self.apply_preset(ShellPreset::PantheonLight),
            (TopBarStyle::FullWidth, true) => self.apply_preset(ShellPreset::PantheonDark),
        }
    }

    pub fn is_pinned(&self, exe_or_target: &str) -> bool {
        let needle = exe_or_target.trim().to_ascii_lowercase();
        if needle.is_empty() {
            return false;
        }
        self.dock.pinned_apps.iter().any(|p| {
            p.exe_name.eq_ignore_ascii_case(&needle) || p.target.eq_ignore_ascii_case(&needle)
        })
    }

    pub fn pin_app(&mut self, app: PinnedApp) {
        if !self.is_pinned(&app.exe_name) && !self.is_pinned(&app.target) {
            self.dock.pinned_apps.push(app);
        }
    }

    pub fn unpin_app(&mut self, exe_or_target: &str) {
        let needle = exe_or_target.trim().to_ascii_lowercase();
        self.dock.pinned_apps.retain(|p| {
            !p.exe_name.eq_ignore_ascii_case(&needle) && !p.target.eq_ignore_ascii_case(&needle)
        });
    }
}

/// Watches `config.toml` parent directory and calls `on_reload` after debouncing writes.
pub fn spawn_config_watcher<F>(path: PathBuf, mut on_reload: F) -> notify::Result<RecommendedWatcher>
where
    F: FnMut(ShellConfig) + Send + 'static,
{
    let watch_dir = path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let _ = fs::create_dir_all(&watch_dir);

    let (tx, rx) = mpsc::channel();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(event) = res
            && matches!(
                event.kind,
                EventKind::Modify(_) | EventKind::Create(_) | EventKind::Any
            )
        {
            let _ = tx.send(());
        }
    })?;
    watcher.watch(&watch_dir, RecursiveMode::NonRecursive)?;

    std::thread::Builder::new()
        .name("vita-config-watcher".into())
        .spawn(move || {
            let mut last_fire = Instant::now() - Duration::from_secs(1);
            while rx.recv().is_ok() {
                // Coalesce burst editor / atomic rename events.
                while rx.recv_timeout(Duration::from_millis(120)).is_ok() {}
                if last_fire.elapsed() < Duration::from_millis(200) {
                    continue;
                }
                if let Ok(raw) = fs::read_to_string(&path)
                    && let Ok(parsed) = toml::from_str::<ShellConfig>(&raw)
                {
                    last_fire = Instant::now();
                    on_reload(parsed);
                }
            }
        })
        .map_err(notify::Error::io)?;

    Ok(watcher)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toml_roundtrip_and_malformed_fallback() {
        let mut cfg = ShellConfig::default();
        cfg.apply_preset(ShellPreset::PantheonDark);
        cfg.accessibility.increase_contrast = true;
        cfg.dock.visibility = DockVisibility::AlwaysVisible;
        cfg.decorator.button_style = DecoratorButtonStyle::TrafficLights;

        let dir = std::env::temp_dir().join(format!("vita_cfg_test_{}", std::process::id()));
        let file = dir.join("config.toml");
        cfg.save_atomic_to(&file).expect("save config");

        let read_back: ShellConfig =
            toml::from_str(&fs::read_to_string(&file).unwrap()).expect("parse back");
        assert_eq!(cfg, read_back);

        // Partial legacy TOML deserializes cleanly with defaults for new fields.
        let legacy: ShellConfig = toml::from_str("preset = \"HorizonDark\"\nhide_native_taskbar = true\n")
            .expect("parse legacy config");
        assert!(legacy.top_bar.show_global_menu);
        assert_eq!(legacy.dock.visibility, DockVisibility::IntelliHide);
        assert!(legacy.dock.all_monitors);
        assert!(legacy.decorator.enabled);

        let fallback = ShellConfig::from_toml_or_default("not = [valid = toml");
        assert_eq!(fallback, ShellConfig::default());

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn presets_match_top_bar_styles() {
        let mut cfg = ShellConfig::default();
        cfg.apply_preset(ShellPreset::PantheonLight);
        assert_eq!(cfg.top_bar.style, TopBarStyle::FullWidth);
        assert_eq!(cfg.surface_material, SurfaceMaterial::Frosted);
        assert!(!cfg.dock.show_space_chip);

        cfg.apply_preset(ShellPreset::HorizonDark);
        assert_eq!(cfg.top_bar.style, TopBarStyle::FloatingPills);
        assert_eq!(cfg.surface_material, SurfaceMaterial::Matte);
        assert!(cfg.dock.show_space_chip);
    }

    #[test]
    fn dock_effective_height_and_background_style_roundtrip() {
        let mut dock = DockConfig {
            icon_size: 32,
            height: 44,
            background_style: DockBackgroundStyle::CustomColor,
            background_color_hex: "#2B233D".to_string(),
            background_opacity: 75,
            corner_radius: 16,
            show_border: false,
            ..DockConfig::default()
        };
        assert_eq!(dock.effective_pill_height(), 44.0);

        dock.height = 36;
        dock.icon_size = 38;
        // Never shrinks below icon_size + 8px padding.
        assert_eq!(dock.effective_pill_height(), 46.0);

        let serialized = toml::to_string(&dock).expect("serialize dock config");
        let parsed: DockConfig = toml::from_str(&serialized).expect("deserialize dock config");
        assert_eq!(dock, parsed);
    }
}
