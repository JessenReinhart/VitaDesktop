pub mod config;
pub mod models;
pub mod search;
pub mod spring;
pub mod theme;

pub use config::{
    AccessibilityConfig, CalmZoneConfig, DecoratorButtonStyle, DockBackgroundStyle, DockConfig,
    DockVisibility, LauncherConfig, LauncherViewMode, PinnedApp, ShellConfig, TopBarConfig,
    TopBarStyle, WindowDecoratorConfig,
};
pub use models::{
    ActiveAppMenu, AppCategory, DecoratedWindowState, DecoratorMode, DockItem, GlobalMenuAction,
    GlobalMenuCategory, GlobalMenuItem, InstalledApp, LogicalRect, MonitorInfo, NotificationLevel,
    PhysicalRect, RgbaIcon, RunningWindow, SpaceInfo, SystemSummary, ToastNotification,
    WindowMenuAction, merge_dock_items,
};
pub use search::{classify_app_category, filter_and_sort_apps, fuzzy_score};
pub use spring::{DockMagnifier, Spring};
pub use theme::{AccentPreset, RgbaColor, ShellPreset, SurfaceMaterial, ThemeMode, ThemeTokens};
