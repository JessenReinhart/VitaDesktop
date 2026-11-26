pub mod appbar;
pub mod apps;
pub mod autostart;
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
pub use autostart::{read_launch_entry, set_launch_at_login};
pub use decorator::{
    DecoratorTracker, DwmWindowThemer, begin_drag_window, dominant_bgra_color,
    inspect_foreground_window, is_window_always_on_top, minimize_window, rgb_to_colorref,
    sample_maximized_window_bg, toggle_always_on_top, toggle_maximize_window,
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
    refresh_tray_icons,
};
pub use watchdog::{run_restart_wait, run_watchdog, spawn_restart_helper, spawn_watchdog};
pub use windows::{
    WindowTracker, close_window, enumerate_running_windows, focus_or_toggle_window, launch_target,
    resolve_pinned_app_icon,
};
pub use workspaces::{create_new_space, enumerate_spaces, switch_to_space};
