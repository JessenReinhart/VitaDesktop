use crate::config::PinnedApp;
use crate::theme::RgbaColor;
use std::collections::HashMap;
use std::sync::Arc;

/// Physical display rectangle in Win32 pixel coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PhysicalRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// DPI-independent logical rectangle used by Iced windows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LogicalRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl PhysicalRect {
    pub fn to_logical(self, dpi_scale: f32) -> LogicalRect {
        let s = dpi_scale.max(0.5);
        LogicalRect {
            x: self.x as f32 / s,
            y: self.y as f32 / s,
            width: self.width as f32 / s,
            height: self.height as f32 / s,
        }
    }

    pub fn right(self) -> i32 {
        self.x + self.width
    }

    pub fn bottom(self) -> i32 {
        self.y + self.height
    }

    pub fn contains_point(self, x: i32, y: i32) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }

    pub fn intersects(self, other: Self) -> bool {
        self.width > 0
            && self.height > 0
            && other.width > 0
            && other.height > 0
            && self.x < other.right()
            && self.right() > other.x
            && self.y < other.bottom()
            && self.bottom() > other.y
    }
}

impl LogicalRect {
    pub fn to_physical(self, dpi_scale: f32) -> PhysicalRect {
        let s = dpi_scale.max(0.5);
        PhysicalRect {
            x: (self.x * s).round() as i32,
            y: (self.y * s).round() as i32,
            width: (self.width * s).round() as i32,
            height: (self.height * s).round() as i32,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MonitorInfo {
    pub name: String,
    pub is_primary: bool,
    pub physical_bounds: PhysicalRect,
    pub physical_workarea: PhysicalRect,
    pub dpi_scale: f32,
}

impl MonitorInfo {
    pub fn logical_bounds(&self) -> LogicalRect {
        self.physical_bounds.to_logical(self.dpi_scale)
    }
}

/// Extracted RGBA8 icon buffer suitable for `iced::widget::image::Handle::from_rgba`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RgbaIcon {
    pub width: u32,
    pub height: u32,
    pub pixels: Arc<[u8]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AppCategory {
    All,
    Accessories,
    Developer,
    Graphics,
    Internet,
    Media,
    Office,
    System,
    Games,
    Other,
}

impl AppCategory {
    pub const ALL_FILTERABLE: [Self; 10] = [
        Self::All,
        Self::Accessories,
        Self::Developer,
        Self::Graphics,
        Self::Internet,
        Self::Media,
        Self::Office,
        Self::System,
        Self::Games,
        Self::Other,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::All => "All Apps",
            Self::Accessories => "Accessories",
            Self::Developer => "Developer",
            Self::Graphics => "Graphics",
            Self::Internet => "Internet",
            Self::Media => "Media",
            Self::Office => "Office",
            Self::System => "System",
            Self::Games => "Games",
            Self::Other => "Other",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledApp {
    pub name: String,
    pub aumid: String,
    pub category: AppCategory,
    pub icon_cache_path: Option<String>,
    pub icon: Option<RgbaIcon>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunningWindow {
    pub hwnd: isize,
    pub title: String,
    pub exe_name: String,
    pub exe_path: String,
    pub pid: u32,
    pub is_focused: bool,
    pub is_minimized: bool,
    pub is_maximized: bool,
    pub frame_rect: PhysicalRect,
    pub monitor_idx: usize,
    pub icon: Option<RgbaIcon>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WindowMenuAction {
    NewWindow { exe_path: String },
    Minimize(isize),
    ToggleMaximize(isize),
    ToggleAlwaysOnTop(isize),
    MoveToNextSpace(isize),
    QuitApp(isize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GlobalMenuAction {
    Win32Command {
        hwnd: isize,
        command_id: u32,
    },
    UiaInvoke {
        hwnd: isize,
        menu_index: usize,
        item_index: usize,
    },
    SyntheticMenuClick {
        hwnd: isize,
        index: usize,
    },
    WindowAction(WindowMenuAction),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobalMenuItem {
    pub label: String,
    pub shortcut: Option<String>,
    pub enabled: bool,
    pub checked: bool,
    pub is_separator: bool,
    pub action: Option<GlobalMenuAction>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobalMenuCategory {
    pub label: String,
    pub items: Vec<GlobalMenuItem>,
    pub lazy_uia_index: Option<usize>,
    pub loaded: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveAppMenu {
    pub hwnd: isize,
    pub app_name: String,
    pub exe_path: String,
    pub monitor_idx: usize,
    pub categories: Vec<GlobalMenuCategory>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecoratorMode {
    FullHeader,
    ControlsOnly,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DecoratedWindowState {
    pub hwnd: isize,
    pub title: String,
    pub exe_name: String,
    pub frame_rect: PhysicalRect,
    pub dpi_scale: f32,
    pub is_maximized: bool,
    pub is_fullscreen: bool,
    pub mode: DecoratorMode,
    pub monitor_idx: usize,
    pub titlebar_bg: Option<RgbaColor>,
    pub icon: Option<RgbaIcon>,
}

/// Merged representation of a slot on the bottom dock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DockItem {
    pub id: String,
    pub label: String,
    pub exe_name: String,
    pub launch_target: Option<String>,
    pub is_pinned: bool,
    pub windows: Vec<RunningWindow>,
    pub icon: Option<RgbaIcon>,
}

impl DockItem {
    pub fn is_running(&self) -> bool {
        !self.windows.is_empty()
    }

    pub fn is_focused(&self) -> bool {
        self.windows.iter().any(|w| w.is_focused)
    }

    pub fn primary_hwnd(&self) -> Option<isize> {
        self.windows
            .iter()
            .find(|w| w.is_focused)
            .or_else(|| self.windows.first())
            .map(|w| w.hwnd)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SystemSummary {
    pub battery_pct: Option<u8>,
    pub is_charging: bool,
    pub battery_remaining_secs: Option<u32>,
    pub volume_pct: u8,
    pub is_muted: bool,
    pub brightness_pct: Option<u8>,
    pub wifi_connected: bool,
    pub wifi_ssid: Option<String>,
    pub bluetooth_on: Option<bool>,
}

impl Default for SystemSummary {
    fn default() -> Self {
        Self {
            battery_pct: None,
            is_charging: false,
            battery_remaining_secs: None,
            volume_pct: 50,
            is_muted: false,
            brightness_pct: None,
            wifi_connected: false,
            wifi_ssid: None,
            bluetooth_on: None,
        }
    }
}

impl SystemSummary {
    pub fn battery_status_text(&self) -> Option<String> {
        let pct = self.battery_pct?;
        if self.is_charging {
            if pct >= 100 {
                return Some("100%  ·  Fully charged".to_string());
            }
            return Some(format!("{pct}%  ·  Charging"));
        }
        if let Some(secs) = self.battery_remaining_secs
            && secs > 0
        {
            let mins = secs / 60;
            let hrs = mins / 60;
            let rem_mins = mins % 60;
            if hrs > 0 {
                return Some(format!("{pct}%  ·  About {hrs}h {rem_mins}m left"));
            }
            return Some(format!("{pct}%  ·  About {rem_mins}m left"));
        }
        Some(format!("{pct}%  ·  On battery"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpaceInfo {
    pub id: String,
    pub index: usize,
    pub name: String,
    pub tagline: String,
    pub is_current: bool,
    pub window_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationLevel {
    Info,
    Success,
    Caution,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToastNotification {
    pub id: u64,
    pub app_name: String,
    pub title: String,
    pub body: String,
    pub timestamp: String,
    pub level: NotificationLevel,
}

fn normalize_exe_key(exe_name: &str) -> String {
    exe_name.trim().to_ascii_lowercase()
}

/// Combines pinned apps with live running windows, grouping multiple windows of the same app cleanly.
pub fn merge_dock_items(
    pinned: &[PinnedApp],
    running: &[RunningWindow],
    pinned_icons: &HashMap<String, RgbaIcon>,
) -> Vec<DockItem> {
    let mut grouped_running: HashMap<String, Vec<RunningWindow>> = HashMap::new();
    let mut unpinned_order: Vec<String> = Vec::new();

    for win in running {
        let key = normalize_exe_key(&win.exe_name);
        if key.is_empty() {
            continue;
        }
        grouped_running
            .entry(key.clone())
            .or_insert_with(|| {
                unpinned_order.push(key);
                Vec::new()
            })
            .push(win.clone());
    }

    let mut items = Vec::with_capacity(pinned.len() + unpinned_order.len());
    let mut consumed_keys = Vec::new();

    for app in pinned {
        let key = normalize_exe_key(&app.exe_name);
        let windows = grouped_running.remove(&key).unwrap_or_default();
        if !windows.is_empty() {
            consumed_keys.push(key.clone());
        }
        let icon = windows
            .iter()
            .find_map(|w| w.icon.clone())
            .or_else(|| pinned_icons.get(&key).cloned());
        items.push(DockItem {
            id: format!("pin:{key}"),
            label: app.name.clone(),
            exe_name: app.exe_name.clone(),
            launch_target: Some(app.target.clone()),
            is_pinned: true,
            windows,
            icon,
        });
    }

    for key in unpinned_order {
        if let Some(windows) = grouped_running.remove(&key) {
            let first = &windows[0];
            let label = first
                .exe_name
                .strip_suffix(".exe")
                .or_else(|| first.exe_name.strip_suffix(".EXE"))
                .unwrap_or(&first.exe_name)
                .to_string();
            let icon = windows
                .iter()
                .find_map(|w| w.icon.clone())
                .or_else(|| pinned_icons.get(&key).cloned());
            items.push(DockItem {
                id: format!("run:{key}"),
                label,
                exe_name: first.exe_name.clone(),
                launch_target: (!first.exe_path.is_empty()).then(|| first.exe_path.clone()),
                is_pinned: false,
                windows,
                icon,
            });
        }
    }

    items
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dpi_roundtrip_across_scales() {
        let logical = LogicalRect {
            x: 120.0,
            y: 40.0,
            width: 800.0,
            height: 48.0,
        };
        for scale in [1.0, 1.25, 1.5, 2.0] {
            let physical = logical.to_physical(scale);
            let back = physical.to_logical(scale);
            assert!((back.x - logical.x).abs() <= 1.0);
            assert!((back.y - logical.y).abs() <= 1.0);
            assert!((back.width - logical.width).abs() <= 1.0);
            assert!((back.height - logical.height).abs() <= 1.0);
        }
    }

    #[test]
    fn dock_merge_groups_pinned_and_running_case_insensitively() {
        let pinned = vec![PinnedApp {
            name: "Terminal".into(),
            target: "wt.exe".into(),
            exe_name: "WindowsTerminal.exe".into(),
        }];
        let running = vec![
            RunningWindow {
                hwnd: 101,
                title: "PowerShell".into(),
                exe_name: "windowsterminal.exe".into(),
                exe_path: "C:\\wt.exe".into(),
                pid: 10,
                is_focused: true,
                is_minimized: false,
                is_maximized: false,
                frame_rect: PhysicalRect::default(),
                monitor_idx: 0,
                icon: None,
            },
            RunningWindow {
                hwnd: 202,
                title: "Code".into(),
                exe_name: "Code.exe".into(),
                exe_path: "C:\\Code.exe".into(),
                pid: 20,
                is_focused: false,
                is_minimized: false,
                is_maximized: false,
                frame_rect: PhysicalRect::default(),
                monitor_idx: 0,
                icon: None,
            },
        ];

        let items = merge_dock_items(&pinned, &running, &HashMap::new());
        assert_eq!(items.len(), 2);
        assert!(items[0].is_pinned && items[0].is_running() && items[0].is_focused());
        assert!(!items[1].is_pinned && items[1].is_running());
        assert_eq!(items[1].label, "Code");
    }

    #[test]
    fn battery_status_formats_cleanly() {
        let mut s = SystemSummary::default();
        assert_eq!(s.battery_status_text(), None);

        s.battery_pct = Some(100);
        s.is_charging = true;
        assert_eq!(
            s.battery_status_text().as_deref(),
            Some("100%  ·  Fully charged")
        );

        s.battery_pct = Some(84);
        s.is_charging = false;
        s.battery_remaining_secs = Some(4 * 3600 + 20 * 60);
        assert_eq!(
            s.battery_status_text().as_deref(),
            Some("84%  ·  About 4h 20m left")
        );
    }

    #[test]
    fn physical_rect_geometry_helpers_work() {
        let a = PhysicalRect {
            x: 1920,
            y: 0,
            width: 1920,
            height: 1080,
        };
        assert!(a.contains_point(1920, 0));
        assert!(a.contains_point(3000, 500));
        assert!(!a.contains_point(1919, 0));
        assert!(!a.contains_point(3840, 500));

        let dock_band = PhysicalRect {
            x: 2400,
            y: 990,
            width: 600,
            height: 90,
        };
        assert!(a.intersects(dock_band));

        let primary_only = PhysicalRect {
            x: 0,
            y: 0,
            width: 1920,
            height: 1200,
        };
        assert!(!dock_band.intersects(primary_only));
    }
}
