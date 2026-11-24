pub mod appbar;
pub mod apps;
pub mod decorator;
pub mod global_menu;
pub mod hotkeys;
pub mod monitors;
pub mod notifications;
pub mod surface;
pub mod system;
pub mod taskbar;
pub mod telemetry;
pub mod thumbnails;
pub mod tray;
pub mod watchdog;
pub mod windows;
pub mod workspaces;

pub use appbar::{AppBarGuard, WorkAreaGuard, restore_saved_workareas};
pub use apps::{enumerate_installed_apps, launch_app_aumid, resolve_aumid_icon};
pub use decorator::{
    DecoratorTracker, DwmWindowThemer, begin_drag_window, inspect_foreground_window,
    is_window_always_on_top, minimize_window, rgb_to_colorref, toggle_always_on_top,
    toggle_maximize_window,
};
pub use global_menu::GlobalMenuWorker;
pub use hotkeys::{HotkeyEvent, HotkeyGuard};
pub use monitors::{
    cursor_position, enumerate_monitors, monitor_index_for_rect, monitor_index_from_cursor,
    primary_monitor, primary_monitor_index, system_prefers_dark,
};
pub use notifications::{
    SystemNotification, WindowsNotificationWatcher, read_windows_notifications,
};
pub use surface::{
    ShellWindowRole, configure_shell_window, position_window_bottom, reposition_window_physical,
    set_window_visible,
};
pub use system::{
    lock_workstation, restart_system, set_display_brightness, set_master_mute, set_master_volume,
    shutdown_system, sleep_system,
};
pub use taskbar::{TaskbarGuard, find_taskbar_hwnds, hide_all_taskbars, restore_all_taskbars};
pub use telemetry::read_system_summary;
pub use thumbnails::{DwmThumbnailHost, capture_window_thumbnail, fit_aspect_rect};
pub use tray::{
    TrayClickKind, TrayIconInfo, TrayTracker, activate_tray_icon, enumerate_tray_icons,
};
pub use watchdog::{run_watchdog, spawn_watchdog};
pub use windows::{
    WindowTracker, close_window, enumerate_running_windows, focus_or_toggle_window, launch_target,
    resolve_pinned_app_icon,
};
pub use workspaces::{create_new_space, enumerate_spaces, switch_to_space};
