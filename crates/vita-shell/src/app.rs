use crate::message::Message;
use crate::style::color;
use crate::surfaces::{
    calm_zone, calendar, dock, dock_menu, dock_thumbnails, global_menu, launcher, notifications,
    quick_controls, settings, spaces, top_bar, tray, window_decorator,
};
use chrono::{DateTime, Datelike, Local, NaiveDate};
use iced::futures::SinkExt;
use iced::futures::Stream;
use iced::futures::channel::mpsc as futures_mpsc;
use iced::keyboard::{self, Key, key::Named};
use iced::widget::{container, image, operation};
use iced::{Color, Element, Length, Padding, Point, Size, Subscription, Task, stream, theme, window};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use vita_core::{
    ActiveAppMenu, AppCategory, BackdropTransition, CalendarStore, DecoratedWindowState,
    DecoratorMode, DockBackgroundStyle, DockItem, DockMagnifier, DockVisibility,
    GlobalMenuCategory, GlobalMenuItem, InstalledApp, LauncherViewMode, MonitorInfo,
    NotificationLevel, PhysicalRect, PinnedApp, RgbaColor, RgbaIcon, RunningWindow, ShellConfig,
    ShellPreset, SpaceInfo, Spring, SystemSummary, ThemeTokens, ToastNotification, TopBarStyle,
    config::spawn_config_watcher, filter_and_sort_apps, merge_dock_items, next_month, prev_month,
};
use vita_win32::{
    DecoratorTracker, DwmThumbnailHost, DwmWindowThemer, GlobalMenuWorker, HotkeyEvent,
    HotkeyGuard, ShellWindowRole, SystemNotification, TaskbarGuard, TrayClickKind, TrayIconInfo,
    TrayTracker, WindowTracker, WindowsNotificationWatcher, WorkAreaGuard, activate_tray_icon,
    begin_drag_window, capture_window_thumbnail, close_window, configure_shell_window,
    create_new_space, cursor_position, enumerate_installed_apps, enumerate_monitors,
    enumerate_spaces, enumerate_tray_icons, focus_or_toggle_window, launch_app_aumid,
    launch_target, lock_workstation, minimize_window, monitor_index_from_cursor,
    position_window_bottom, primary_monitor, primary_monitor_index, read_system_summary,
    reposition_window_physical, resolve_aumid_icon, resolve_pinned_app_icon, restart_system,
    restore_all_taskbars, rgb_to_colorref, sample_maximized_window_bg, set_display_brightness,
    set_master_mute, set_master_volume, set_window_visible, shutdown_system, sleep_system,
    spawn_watchdog, switch_to_space, system_prefers_dark, toggle_maximize_window,
};

static GLOBAL_MENU_RX: Mutex<Option<mpsc::UnboundedReceiver<Option<ActiveAppMenu>>>> =
    Mutex::new(None);
static WORK_AREA_SHARED: Mutex<Option<Arc<Mutex<WorkAreaGuard>>>> = Mutex::new(None);
static DEBUG_FORCE_TUCK: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Forces the maximized-title-bar tuck on every monitor (used by `--debug-tuck` for diagnostics).
pub fn set_debug_force_tuck(on: bool) {
    DEBUG_FORCE_TUCK.store(on, std::sync::atomic::Ordering::Relaxed);
}

fn is_top_tab_browser(exe_name: &str) -> bool {
    matches!(
        exe_name.to_ascii_lowercase().as_str(),
        "msedge.exe" | "chrome.exe" | "firefox.exe" | "brave.exe" | "vivaldi.exe" | "opera.exe" | "arc.exe"
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct MonitorMaximizedState {
    frontmost_hwnd: Option<isize>,
    should_tuck: bool,
}

/// Resolves the frontmost non-fullscreen maximized HWND and title-bar tuck flag per monitor
/// from the authoritative foreground window and top-to-bottom Z-ordered `running_windows`.
fn resolve_monitor_maximized_states(
    monitor_count: usize,
    decorated: Option<&DecoratedWindowState>,
    running_windows: &[RunningWindow],
) -> Vec<MonitorMaximizedState> {
    (0..monitor_count)
        .map(|mon_idx| {
            if let Some(d) = decorated
                && d.monitor_idx == mon_idx
            {
                if d.is_fullscreen {
                    return MonitorMaximizedState::default();
                }
                if d.is_maximized {
                    return MonitorMaximizedState {
                        frontmost_hwnd: Some(d.hwnd),
                        should_tuck: !is_top_tab_browser(&d.exe_name),
                    };
                }
            }

            let stale_non_max_hwnd = decorated
                .filter(|d| !d.is_maximized || d.monitor_idx != mon_idx)
                .map(|d| d.hwnd);

            for w in running_windows.iter() {
                if w.monitor_idx != mon_idx || w.is_minimized || !w.is_maximized {
                    continue;
                }
                if stale_non_max_hwnd == Some(w.hwnd) {
                    continue;
                }
                return MonitorMaximizedState {
                    frontmost_hwnd: Some(w.hwnd),
                    should_tuck: !is_top_tab_browser(&w.exe_name),
                };
            }

            MonitorMaximizedState::default()
        })
        .collect()
}

/// Resolves per-monitor title-bar tuck flags from the authoritative foreground window and
/// top-to-bottom Z-ordered `running_windows` (`EnumWindows` order).
#[cfg(test)]
fn resolve_tuck_flags(
    monitor_count: usize,
    decorated: Option<&DecoratedWindowState>,
    running_windows: &[RunningWindow],
) -> Vec<bool> {
    resolve_monitor_maximized_states(monitor_count, decorated, running_windows)
        .into_iter()
        .map(|s| s.should_tuck)
        .collect()
}

#[derive(Debug, Clone, Copy)]
enum HardwareCmd {
    Volume(u8),
    Mute(bool),
    Brightness(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SurfaceKind {
    TopBar,
    Dock,
    Launcher,
    DockPopover,
    QuickControls,
    Spaces,
    CalmZone,
    Notifications,
    Toast,
    TrayPopover,
    Calendar,
    Settings,
    GlobalMenu,
    WindowDecorator,
}

struct SurfaceAnim {
    progress: Spring,
    closing: bool,
    slide_px: f32,
    from_top: bool,
    tokens: ThemeTokens,
}

fn wrap_surface_slide<'a>(
    el: Element<'a, Message>,
    anim: Option<&SurfaceAnim>,
) -> Element<'a, Message> {
    let Some(anim) = anim else {
        return el;
    };
    if anim.slide_px <= 0.0 || anim.progress.value >= 0.999 {
        return el;
    }
    let t = anim.progress.value.clamp(0.0, 1.0);
    let offset = (1.0 - t) * anim.slide_px;
    let padding = if anim.from_top {
        Padding {
            top: offset,
            ..Padding::ZERO
        }
    } else {
        Padding {
            top: offset * 0.65,
            bottom: offset * 0.35,
            ..Padding::ZERO
        }
    };
    container(el)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(padding)
        .into()
}

struct MonitorShell {
    info: MonitorInfo,
    top_bar_id: window::Id,
    dock_id: Option<window::Id>,
    dock_magnifier: DockMagnifier,
    hovered_dock_idx: Option<usize>,
    dock_hidden: bool,
    dock_hwnd_shown: bool,
    dock_last_trigger: Option<Instant>,
    maximized_bg_hwnd: Option<isize>,
    backdrop_transition: BackdropTransition,
    pantheon_solid: Spring,
    dock_reveal: Spring,
    dock_tokens: ThemeTokens,
    dock_tooltip_opacity: Spring,
    dock_last_tooltip_idx: Option<usize>,
}

pub struct VitaShell {
    config: ShellConfig,
    tokens: ThemeTokens,
    tokens_from: ThemeTokens,
    tokens_target: ThemeTokens,
    theme_transition: Spring,
    system_dark: bool,
    monitor_shells: Vec<MonitorShell>,
    top_bar_to_mon: HashMap<window::Id, usize>,
    dock_to_mon: HashMap<window::Id, usize>,
    primary_monitor_idx: usize,
    active_monitor_idx: usize,
    work_area_guard: Arc<Mutex<WorkAreaGuard>>,
    taskbar_guard: Arc<Mutex<TaskbarGuard>>,
    watchdog_spawned: bool,
    companion_mode: bool,
    cli_preset_locked: bool,
    config_path_str: String,

    launcher_id: Option<window::Id>,
    dock_popover_id: Option<window::Id>,
    dock_popover_monitor_idx: usize,
    quick_controls_id: Option<window::Id>,
    spaces_id: Option<window::Id>,
    calm_zone_id: Option<window::Id>,
    notifications_id: Option<window::Id>,
    toast_id: Option<window::Id>,
    tray_popover_id: Option<window::Id>,
    calendar_id: Option<window::Id>,
    settings_id: Option<window::Id>,
    settings_tab: settings::SettingsTab,
    settings_positioned: bool,
    global_menu_id: Option<window::Id>,
    open_global_menu: Option<(usize, usize)>,
    global_menu_opened_at: Option<Instant>,
    decorator_id: Option<window::Id>,
    windows: HashMap<window::Id, SurfaceKind>,
    hwnds: HashMap<window::Id, u64>,

    now: DateTime<Local>,
    summary: SystemSummary,
    focus_mode: bool,
    power_confirm_open: bool,
    hw_tx: mpsc::UnboundedSender<HardwareCmd>,
    running_windows: Vec<RunningWindow>,
    spaces: Vec<SpaceInfo>,
    notifications: Vec<ToastNotification>,
    active_toast: Option<ToastNotification>,
    next_notif_id: u64,
    wpn_seeded: bool,
    seen_wpn_ids: HashSet<String>,
    dismissed_wpn_ids: HashSet<String>,
    wpn_to_notif_id: HashMap<String, u64>,
    notif_id_to_wpn: HashMap<u64, String>,
    pinned_icons: HashMap<String, RgbaIcon>,
    dock_items: Vec<DockItem>,
    icon_sources: HashMap<String, RgbaIcon>,
    icon_handles: HashMap<String, image::Handle>,

    tray_icons: Vec<TrayIconInfo>,
    tray_icon_sources: HashMap<String, RgbaIcon>,
    tray_icon_handles: HashMap<String, image::Handle>,
    tray_popover_opened_at: Option<Instant>,

    calendar_opened_at: Option<Instant>,
    calendar_store: CalendarStore,
    calendar_year: i32,
    calendar_month: u32,
    calendar_selected_date: NaiveDate,
    calendar_draft: String,

    installed_apps: Vec<InstalledApp>,
    app_icon_handles: HashMap<String, image::Handle>,
    apps_loading: bool,

    launcher_opened_at: Option<Instant>,
    launcher_query: String,
    launcher_view_mode: LauncherViewMode,
    launcher_category: AppCategory,
    launcher_page: usize,
    launcher_selected_idx: usize,

    dock_popover_opened_at: Option<Instant>,
    dock_popover_target: Option<dock_menu::DockPopoverTarget>,
    dock_preview_layout: Option<dock_thumbnails::PreviewStripLayout>,
    dwm_thumbnails: DwmThumbnailHost,
    dock_fallback_thumbs: HashMap<isize, image::Handle>,
    dock_hover_preview: Option<(usize, usize, Instant, bool)>,
    dock_hover_outside_since: Option<Instant>,
    dock_popover_opened_by_hover: bool,

    quick_controls_opened_at: Option<Instant>,
    spaces_opened_at: Option<Instant>,
    notifications_opened_at: Option<Instant>,

    last_anim_tick: Option<Instant>,
    window_anims: HashMap<window::Id, SurfaceAnim>,
    dock_window_width: f32,
    dock_window_height: f32,

    global_menu_worker: GlobalMenuWorker,
    active_app_menu: Option<ActiveAppMenu>,
    last_inspected_menu_hwnd: Option<isize>,
    decorated_window: Option<DecoratedWindowState>,
    decorator_icon: Option<(isize, image::Handle)>,
    dwm_themer: DwmWindowThemer,
    last_decorator_press: Option<(isize, Instant)>,
    tucked_monitors: Vec<bool>,
    tuck_height_applied: u32,
    window_bg_cache: HashMap<isize, RgbaColor>,

    screenshot_dir: Option<PathBuf>,
    open_launcher_on_boot: bool,
    open_quick_controls_on_boot: bool,
    open_spaces_on_boot: bool,
    open_notifications_on_boot: bool,
    open_settings_on_boot: bool,
    open_global_menu_on_boot: bool,
    open_decorator_preview_on_boot: bool,
    open_calendar_on_boot: bool,
    open_tray_on_boot: bool,
    open_dock_thumbnails_on_boot: bool,
    screenshots_saved: usize,
    screenshots_expected: usize,
}

impl VitaShell {
    #[allow(clippy::too_many_arguments)]
    pub fn boot(
        initial_preset: Option<ShellPreset>,
        companion_mode: bool,
        screenshot_dir: Option<PathBuf>,
        open_launcher_on_boot: bool,
        open_quick_controls_on_boot: bool,
        open_spaces_on_boot: bool,
        open_notifications_on_boot: bool,
        open_settings_on_boot: bool,
        open_global_menu_on_boot: bool,
        open_decorator_preview_on_boot: bool,
        open_calendar_on_boot: bool,
        open_tray_on_boot: bool,
        open_dock_thumbnails_on_boot: bool,
        initial_settings_tab: settings::SettingsTab,
        taskbar_guard: Arc<Mutex<TaskbarGuard>>,
        watchdog_spawned: bool,
    ) -> (Self, Task<Message>) {
        let mut config = ShellConfig::load_or_default();
        let cli_preset_locked = initial_preset.is_some();
        if let Some(preset) = initial_preset {
            config.apply_preset(preset);
        }
        if companion_mode {
            config.hide_native_taskbar = false;
        }

        let system_dark = system_prefers_dark();
        let tokens = resolve_tokens(&config, system_dark);
        let mut monitors = enumerate_monitors();
        if monitors.is_empty() {
            monitors.push(primary_monitor());
        }
        let primary_idx = primary_monitor_index(&monitors);

        let reserve_workarea = screenshot_dir.is_none();
        let mut work_area = WorkAreaGuard::new(&monitors, reserve_workarea);
        let bottom_reserve = bottom_reserve_for_config(&config);
        work_area.apply(
            &monitors,
            config.top_bar.height,
            bottom_reserve,
            config.hide_native_taskbar,
        );
        let work_area_guard = Arc::new(Mutex::new(work_area));
        if let Ok(mut slot) = WORK_AREA_SHARED.lock() {
            *slot = Some(Arc::clone(&work_area_guard));
        }

        let initial_items = merge_dock_items(&config.dock.pinned_apps, &[], &HashMap::new());
        let (dock_w, slot_centers) = dock::compute_dock_layout(&config, initial_items.len());
        let dock_h = dock::window_height(&config);

        let mut windows = HashMap::new();
        let mut top_bar_to_mon = HashMap::new();
        let mut dock_to_mon = HashMap::new();
        let mut monitor_shells = Vec::with_capacity(monitors.len());
        let mut open_shell_tasks = Vec::new();

        for (idx, mon) in monitors.into_iter().enumerate() {
            let logical = mon.logical_bounds();
            let (tb_id, open_tb) = window::open(window::Settings {
                size: Size::new(logical.width, config.top_bar.height as f32),
                position: window::Position::Specific(Point::new(logical.x, logical.y)),
                decorations: false,
                transparent: true,
                resizable: false,
                level: window::Level::AlwaysOnTop,
                exit_on_close_request: false,
                ..window::Settings::default()
            });
            windows.insert(tb_id, SurfaceKind::TopBar);
            top_bar_to_mon.insert(tb_id, idx);
            open_shell_tasks.push(open_tb.map(Message::WindowOpened));

            let mut magnifier =
                DockMagnifier::new(config.dock.magnification_scale, config.dock.max_magnify_radius);
            magnifier.sync_slots(slot_centers.clone());

            let should_open_dock = config.dock.all_monitors || idx == primary_idx;
            let dock_id = if should_open_dock {
                let dock_x = logical.x + (logical.width - dock_w) * 0.5;
                let dock_y = logical.y + logical.height - dock_h;
                let (d_id, open_d) = window::open(window::Settings {
                    size: Size::new(dock_w, dock_h),
                    position: window::Position::Specific(Point::new(dock_x, dock_y)),
                    decorations: false,
                    transparent: true,
                    resizable: false,
                    level: window::Level::AlwaysOnTop,
                    exit_on_close_request: false,
                    ..window::Settings::default()
                });
                windows.insert(d_id, SurfaceKind::Dock);
                dock_to_mon.insert(d_id, idx);
                open_shell_tasks.push(open_d.map(Message::WindowOpened));
                Some(d_id)
            } else {
                None
            };

            monitor_shells.push(MonitorShell {
                info: mon,
                top_bar_id: tb_id,
                dock_id,
                dock_magnifier: magnifier,
                hovered_dock_idx: None,
                dock_hidden: false,
                dock_hwnd_shown: true,
                dock_last_trigger: None,
                maximized_bg_hwnd: None,
                backdrop_transition: BackdropTransition::new(tokens.surface_base),
                pantheon_solid: Spring::smooth(0.0),
                dock_reveal: Spring::new(1.0, 260.0, 28.0),
                dock_tokens: tokens,
                dock_tooltip_opacity: Spring::snappy(0.0),
                dock_last_tooltip_idx: None,
            });
        }

        let mut calm_zone_id = None;
        let mut open_calm_task = None;
        if config.calm_zone.enabled {
            let logical = monitor_shells[primary_idx].info.logical_bounds();
            let cz_x = logical.x + 28.0;
            let cz_y = logical.y + config.top_bar.height as f32 + 22.0;
            let (id, task) = window::open(window::Settings {
                size: Size::new(calm_zone::CALM_ZONE_WIDTH, calm_zone::CALM_ZONE_HEIGHT),
                position: window::Position::Specific(Point::new(cz_x, cz_y)),
                decorations: false,
                transparent: true,
                resizable: false,
                level: window::Level::Normal,
                exit_on_close_request: false,
                ..window::Settings::default()
            });
            calm_zone_id = Some(id);
            windows.insert(id, SurfaceKind::CalmZone);
            open_calm_task = Some(task.map(Message::WindowOpened));
        }

        let initial_spaces = enumerate_spaces(&[]);
        let pinned_for_icons = config.dock.pinned_apps.clone();
        let load_pinned_task = Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    let mut map = HashMap::new();
                    for app in pinned_for_icons {
                        if let Some(icon) = resolve_pinned_app_icon(&app.target, &app.exe_name) {
                            let key = if !app.exe_name.trim().is_empty() {
                                app.exe_name.to_ascii_lowercase()
                            } else {
                                app.target.to_ascii_lowercase()
                            };
                            map.insert(key, icon);
                        }
                    }
                    map
                })
                .await
                .unwrap_or_default()
            },
            Message::PinnedIconsLoaded,
        );

        let load_apps_task = Task::perform(
            async move {
                tokio::task::spawn_blocking(enumerate_installed_apps)
                    .await
                    .unwrap_or_default()
            },
            Message::InstalledAppsLoaded,
        );

        let (hw_tx, mut hw_rx) = mpsc::unbounded_channel::<HardwareCmd>();
        std::thread::Builder::new()
            .name("vita-hw-writer".into())
            .spawn(move || {
                while let Some(mut cmd) = hw_rx.blocking_recv() {
                    while let Ok(newer) = hw_rx.try_recv() {
                        match (cmd, newer) {
                            (HardwareCmd::Volume(_), HardwareCmd::Volume(_))
                            | (HardwareCmd::Brightness(_), HardwareCmd::Brightness(_))
                            | (HardwareCmd::Mute(_), HardwareCmd::Mute(_)) => {
                                cmd = newer;
                            }
                            _ => {
                                apply_hardware_cmd(cmd);
                                cmd = newer;
                            }
                        }
                    }
                    apply_hardware_cmd(cmd);
                }
            })
            .ok();

        let (menu_tx, menu_rx) = mpsc::unbounded_channel();
        if let Ok(mut slot) = GLOBAL_MENU_RX.lock() {
            *slot = Some(menu_rx);
        }
        let global_menu_worker = GlobalMenuWorker::start(menu_tx);

        let default_view = config.launcher.default_view;
        let screenshots_expected = 2
            + usize::from(calm_zone_id.is_some())
            + usize::from(open_launcher_on_boot)
            + usize::from(open_quick_controls_on_boot)
            + usize::from(open_spaces_on_boot)
            + usize::from(open_notifications_on_boot)
            + usize::from(open_settings_on_boot)
            + usize::from(open_global_menu_on_boot)
            + usize::from(open_decorator_preview_on_boot)
            + usize::from(open_calendar_on_boot)
            + usize::from(open_tray_on_boot)
            + usize::from(open_dock_thumbnails_on_boot);
        let config_path_str = ShellConfig::config_path().to_string_lossy().to_string();

        let now = Local::now();
        let today = now.date_naive();
        let calendar_store = CalendarStore::load_or_default();

        let mut initial_notifs = Vec::new();
        let mut next_notif_id = 1u64;
        if open_notifications_on_boot {
            initial_notifs.push(ToastNotification {
                id: next_notif_id,
                app_name: "VitaDesktop".to_string(),
                title: "A little win.".to_string(),
                body: "Your shell preferences and spaces are ready.".to_string(),
                timestamp: "Just now".to_string(),
                level: NotificationLevel::Success,
            });
            next_notif_id += 1;
            initial_notifs.push(ToastNotification {
                id: next_notif_id,
                app_name: "System".to_string(),
                title: "Horizon preset active".to_string(),
                body: "Press Alt + ` anytime to view Spaces.".to_string(),
                timestamp: "1m ago".to_string(),
                level: NotificationLevel::Info,
            });
            next_notif_id += 1;
        }

        let state = Self {
            config,
            tokens,
            tokens_from: tokens,
            tokens_target: tokens,
            theme_transition: Spring::smooth(1.0),
            system_dark,
            monitor_shells,
            top_bar_to_mon,
            dock_to_mon,
            primary_monitor_idx: primary_idx,
            active_monitor_idx: primary_idx,
            work_area_guard,
            taskbar_guard,
            watchdog_spawned,
            companion_mode,
            cli_preset_locked,
            config_path_str,
            launcher_id: None,
            dock_popover_id: None,
            dock_popover_monitor_idx: primary_idx,
            quick_controls_id: None,
            spaces_id: None,
            calm_zone_id,
            notifications_id: None,
            toast_id: None,
            tray_popover_id: None,
            calendar_id: None,
            settings_id: None,
            settings_tab: initial_settings_tab,
            settings_positioned: false,
            global_menu_id: None,
            open_global_menu: None,
            global_menu_opened_at: None,
            decorator_id: None,
            windows,
            hwnds: HashMap::new(),
            now,
            summary: read_system_summary(),
            focus_mode: false,
            power_confirm_open: false,
            hw_tx,
            running_windows: Vec::new(),
            spaces: initial_spaces,
            notifications: initial_notifs,
            active_toast: None,
            next_notif_id,
            wpn_seeded: false,
            seen_wpn_ids: HashSet::new(),
            dismissed_wpn_ids: HashSet::new(),
            wpn_to_notif_id: HashMap::new(),
            notif_id_to_wpn: HashMap::new(),
            pinned_icons: HashMap::new(),
            dock_items: initial_items,
            icon_sources: HashMap::new(),
            icon_handles: HashMap::new(),
            tray_icons: Vec::new(),
            tray_icon_sources: HashMap::new(),
            tray_icon_handles: HashMap::new(),
            tray_popover_opened_at: None,
            calendar_opened_at: None,
            calendar_store,
            calendar_year: today.year(),
            calendar_month: today.month(),
            calendar_selected_date: today,
            calendar_draft: String::new(),
            installed_apps: Vec::new(),
            app_icon_handles: HashMap::new(),
            apps_loading: true,
            launcher_opened_at: None,
            launcher_query: String::new(),
            launcher_view_mode: default_view,
            launcher_category: AppCategory::All,
            launcher_page: 0,
            launcher_selected_idx: 0,
            dock_popover_opened_at: None,
            dock_popover_target: None,
            dock_preview_layout: None,
            dwm_thumbnails: DwmThumbnailHost::new(),
            dock_fallback_thumbs: HashMap::new(),
            dock_hover_preview: None,
            dock_hover_outside_since: None,
            dock_popover_opened_by_hover: false,
            quick_controls_opened_at: None,
            spaces_opened_at: None,
            notifications_opened_at: None,
            last_anim_tick: None,
            window_anims: HashMap::new(),
            dock_window_width: dock_w,
            dock_window_height: dock_h,
            global_menu_worker,
            active_app_menu: None,
            last_inspected_menu_hwnd: None,
            decorated_window: None,
            decorator_icon: None,
            dwm_themer: DwmWindowThemer::default(),
            last_decorator_press: None,
            tucked_monitors: Vec::new(),
            tuck_height_applied: 0,
            window_bg_cache: HashMap::new(),
            screenshot_dir,
            open_launcher_on_boot,
            open_quick_controls_on_boot,
            open_spaces_on_boot,
            open_notifications_on_boot,
            open_settings_on_boot,
            open_global_menu_on_boot,
            open_decorator_preview_on_boot,
            open_calendar_on_boot,
            open_tray_on_boot,
            open_dock_thumbnails_on_boot,
            screenshots_saved: 0,
            screenshots_expected,
        };

        let mut task_list = open_shell_tasks;
        task_list.push(load_pinned_task);
        task_list.push(load_apps_task);
        if let Some(cz_task) = open_calm_task {
            task_list.push(cz_task);
        }
        if state.open_quick_controls_on_boot {
            task_list.push(Task::perform(
                tokio::time::sleep(Duration::from_millis(500)),
                |_| Message::ToggleQuickControls,
            ));
        }
        if state.open_spaces_on_boot {
            task_list.push(Task::perform(
                tokio::time::sleep(Duration::from_millis(500)),
                |_| Message::ToggleSpaces,
            ));
        }
        if state.open_notifications_on_boot {
            task_list.push(Task::perform(
                tokio::time::sleep(Duration::from_millis(500)),
                |_| Message::ToggleNotifications,
            ));
        }
        if state.open_settings_on_boot {
            task_list.push(Task::perform(
                tokio::time::sleep(Duration::from_millis(500)),
                |_| Message::OpenSettings,
            ));
        }
        if state.open_global_menu_on_boot
            || state.open_decorator_preview_on_boot
            || state.open_calendar_on_boot
            || state.open_tray_on_boot
            || state.open_dock_thumbnails_on_boot
        {
            task_list.push(Task::perform(
                tokio::time::sleep(Duration::from_millis(450)),
                |_| Message::Tick(Local::now()),
            ));
        }
        if state.screenshot_dir.is_some() && !state.open_launcher_on_boot {
            task_list.push(Task::perform(
                tokio::time::sleep(Duration::from_millis(2000)),
                |_| Message::CaptureScreenshots,
            ));
        }

        (state, Task::batch(task_list))
    }

    pub fn title(&self, id: window::Id) -> String {
        match self.windows.get(&id) {
            Some(SurfaceKind::TopBar) => "VitaDesktop TopBar".to_string(),
            Some(SurfaceKind::Dock) => "VitaDesktop Dock".to_string(),
            Some(SurfaceKind::Launcher) => "VitaDesktop Launcher".to_string(),
            Some(SurfaceKind::DockPopover) => "VitaDesktop Dock Menu".to_string(),
            Some(SurfaceKind::QuickControls) => "VitaDesktop Quick Controls".to_string(),
            Some(SurfaceKind::Spaces) => "VitaDesktop Spaces".to_string(),
            Some(SurfaceKind::CalmZone) => "VitaDesktop Calm Zone".to_string(),
            Some(SurfaceKind::Notifications) => "VitaDesktop Notifications".to_string(),
            Some(SurfaceKind::Toast) => "VitaDesktop Toast".to_string(),
            Some(SurfaceKind::TrayPopover) => "VitaDesktop System Tray".to_string(),
            Some(SurfaceKind::Calendar) => "VitaDesktop Calendar".to_string(),
            Some(SurfaceKind::Settings) => "Vita Settings".to_string(),
            Some(SurfaceKind::GlobalMenu) => "VitaDesktop Global Menu".to_string(),
            Some(SurfaceKind::WindowDecorator) => "VitaDesktop Decorator".to_string(),
            None => "VitaDesktop".to_string(),
        }
    }

    pub fn daemon_style(&self, _theme: &iced::Theme) -> theme::Style {
        theme::Style {
            background_color: Color::TRANSPARENT,
            text_color: color(self.tokens.text_primary),
        }
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::WindowOpened(id) => {
                self.start_surface_open_anim(id);
                window::raw_id::<u64>(id).map(move |raw| Message::WindowRawId(id, raw))
            }
            Message::WindowRawId(id, raw) => {
                self.hwnds.insert(id, raw);
                self.apply_surface_chrome_and_bounds(id);
                if self.settings_id == Some(id) {
                    self.settings_positioned = true;
                }
                if self.dock_popover_id == Some(id) {
                    return self.sync_dock_thumbnail_slots();
                }
                Task::none()
            }
            Message::WindowEvent(id, event) => match event {
                window::Event::CloseRequested => {
                    if self.settings_id == Some(id) {
                        return self.close_settings();
                    }
                    Task::none()
                }
                window::Event::Resized(_) => {
                    if self.dock_popover_id == Some(id) {
                        return self.sync_dock_thumbnail_slots();
                    }
                    Task::none()
                }
                window::Event::Closed => {
                    self.windows.remove(&id);
                    self.hwnds.remove(&id);
                    self.window_anims.remove(&id);
                    self.top_bar_to_mon.remove(&id);
                    self.dock_to_mon.remove(&id);
                    for shell in &mut self.monitor_shells {
                        if shell.dock_id == Some(id) {
                            shell.dock_id = None;
                        }
                    }
                    if self.launcher_id == Some(id) {
                        self.launcher_id = None;
                        self.launcher_opened_at = None;
                    }
                    if self.dock_popover_id == Some(id) {
                        self.dock_popover_id = None;
                        self.dock_popover_opened_at = None;
                        self.dock_popover_target = None;
                        self.dock_preview_layout = None;
                        self.dock_popover_opened_by_hover = false;
                        self.dock_hover_outside_since = None;
                        self.dwm_thumbnails.clear();
                        self.dock_fallback_thumbs.clear();
                    }
                    if self.quick_controls_id == Some(id) {
                        self.quick_controls_id = None;
                        self.quick_controls_opened_at = None;
                        self.power_confirm_open = false;
                    }
                    if self.spaces_id == Some(id) {
                        self.spaces_id = None;
                        self.spaces_opened_at = None;
                    }
                    if self.calm_zone_id == Some(id) {
                        self.calm_zone_id = None;
                    }
                    if self.notifications_id == Some(id) {
                        self.notifications_id = None;
                        self.notifications_opened_at = None;
                    }
                    if self.toast_id == Some(id) {
                        self.toast_id = None;
                        self.active_toast = None;
                    }
                    if self.tray_popover_id == Some(id) {
                        self.tray_popover_id = None;
                        self.tray_popover_opened_at = None;
                    }
                    if self.calendar_id == Some(id) {
                        self.calendar_id = None;
                        self.calendar_opened_at = None;
                    }
                    if self.settings_id == Some(id) {
                        self.settings_id = None;
                        self.settings_positioned = false;
                    }
                    if self.global_menu_id == Some(id) {
                        self.global_menu_id = None;
                        self.open_global_menu = None;
                        self.global_menu_opened_at = None;
                    }
                    if self.decorator_id == Some(id) {
                        self.decorator_id = None;
                    }
                    Task::none()
                }
                window::Event::Unfocused => {
                    if self.screenshot_dir.is_some() {
                        return Task::none();
                    }
                    if self.launcher_id == Some(id)
                        && self
                            .launcher_opened_at
                            .is_some_and(|t| t.elapsed() > Duration::from_millis(220))
                    {
                        return self.close_launcher();
                    }
                    if self.dock_popover_id == Some(id)
                        && !self.dock_popover_opened_by_hover
                        && self
                            .dock_popover_opened_at
                            .is_some_and(|t| t.elapsed() > Duration::from_millis(220))
                    {
                        return self.close_dock_popover();
                    }
                    if self.quick_controls_id == Some(id)
                        && self
                            .quick_controls_opened_at
                            .is_some_and(|t| t.elapsed() > Duration::from_millis(220))
                    {
                        return self.close_quick_controls();
                    }
                    if self.spaces_id == Some(id)
                        && self
                            .spaces_opened_at
                            .is_some_and(|t| t.elapsed() > Duration::from_millis(220))
                    {
                        return self.close_spaces();
                    }
                    if self.notifications_id == Some(id)
                        && self
                            .notifications_opened_at
                            .is_some_and(|t| t.elapsed() > Duration::from_millis(220))
                    {
                        return self.close_notifications();
                    }
                    if self.tray_popover_id == Some(id)
                        && self
                            .tray_popover_opened_at
                            .is_some_and(|t| t.elapsed() > Duration::from_millis(220))
                    {
                        return self.close_tray_popover();
                    }
                    if self.calendar_id == Some(id)
                        && self
                            .calendar_opened_at
                            .is_some_and(|t| t.elapsed() > Duration::from_millis(220))
                    {
                        return self.close_calendar();
                    }
                    if self.global_menu_id == Some(id)
                        && self
                            .global_menu_opened_at
                            .is_some_and(|t| t.elapsed() > Duration::from_millis(220))
                    {
                        return self.close_global_menu();
                    }
                    Task::none()
                }
                _ => Task::none(),
            },
            Message::KeyboardEvent(ev) => self.handle_keyboard(ev),
            Message::Tick(now) => {
                self.now = now;
                if self.screenshot_dir.is_some()
                    && ((self.open_global_menu_on_boot && self.global_menu_id.is_none())
                        || (self.open_decorator_preview_on_boot && self.decorator_id.is_none())
                        || (self.open_calendar_on_boot && self.calendar_id.is_none())
                        || (self.open_tray_on_boot && self.tray_popover_id.is_none())
                        || (self.open_dock_thumbnails_on_boot && self.dock_popover_id.is_none()))
                {
                    return self.setup_screenshot_preview_surfaces();
                }
                Task::none()
            }
            Message::DockSensorTick => {
                self.update_dock_visibility();
                self.evaluate_dock_hover_preview()
            }
            Message::TelemetryUpdated(summary) => {
                self.summary = summary;
                if self.screenshot_dir.is_none()
                    && self.config.top_bar.style == TopBarStyle::FloatingPills
                    && self.config.decorator.enabled
                    && self.config.decorator.hide_maximized_titlebar
                {
                    let targets: Vec<(usize, isize, MonitorInfo)> = self
                        .monitor_shells
                        .iter()
                        .enumerate()
                        .filter_map(|(mon_idx, shell)| {
                            shell
                                .maximized_bg_hwnd
                                .map(|hwnd| (mon_idx, hwnd, shell.info.clone()))
                        })
                        .collect();
                    return self.spawn_maximized_bg_sample_task(targets, 0, false);
                }
                Task::none()
            }
            Message::WindowsUpdated(wins) => {
                if self.screenshot_dir.is_some()
                    && self.open_dock_thumbnails_on_boot
                    && self.dock_popover_id.is_some()
                {
                    return Task::none();
                }
                self.running_windows = wins.clone();
                let running = &self.running_windows;
                let deco = self.decorated_window.as_ref();
                self.window_bg_cache.retain(|hwnd, _| {
                    running.iter().any(|w| w.hwnd == *hwnd) || deco.is_some_and(|d| d.hwnd == *hwnd)
                });
                let dock_resize_task = self.rebuild_dock_items();
                self.update_dock_visibility();
                let tuck_task = self.sync_workarea_tuck();
                let picker_task = self.reconcile_open_window_picker();
                let spaces_task = Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || enumerate_spaces(&wins))
                            .await
                            .unwrap_or_default()
                    },
                    Message::SpacesUpdated,
                );
                Task::batch([dock_resize_task, tuck_task, picker_task, spaces_task])
            }
            Message::MonitorsChanged(monitors) => self.reconcile_monitors(monitors),
            Message::SpacesUpdated(spaces) => {
                if !spaces.is_empty() {
                    self.spaces = spaces;
                }
                Task::none()
            }
            Message::PinnedIconsLoaded(icons) => {
                self.pinned_icons.extend(icons);
                self.rebuild_dock_items()
            }
            Message::InstalledAppsLoaded(apps) => {
                self.app_icon_handles.clear();
                for app in &apps {
                    if let Some(icon) = &app.icon {
                        let handle =
                            image::Handle::from_rgba(icon.width, icon.height, icon.pixels.to_vec());
                        self.app_icon_handles.insert(app.aumid.clone(), handle);
                    }
                }
                self.installed_apps = apps;
                self.apps_loading = false;

                if self.screenshot_dir.is_some() && self.open_launcher_on_boot {
                    let open_task = self.open_launcher_on(self.primary_monitor_idx);
                    let capture_task = Task::perform(
                        tokio::time::sleep(Duration::from_millis(750)),
                        |_| Message::CaptureScreenshots,
                    );
                    return Task::batch([open_task, capture_task]);
                }
                Task::none()
            }
            Message::TrayIconsUpdated(icons) => {
                if self.screenshot_dir.is_some() && self.open_tray_on_boot && icons.is_empty() {
                    return Task::none();
                }
                self.apply_tray_icons_snapshot(icons)
            }
            Message::SystemNotificationsUpdated(sys_list) => {
                self.merge_system_notifications(sys_list)
            }
            Message::DockFallbackThumbCaptured(hwnd, maybe_icon) => {
                if let Some(ic) = maybe_icon {
                    let handle =
                        image::Handle::from_rgba(ic.width, ic.height, ic.pixels.to_vec());
                    self.dock_fallback_thumbs.insert(hwnd, handle);
                }
                Task::none()
            }
            Message::MaximizedWindowColorsSampled {
                samples,
                allow_fallback,
            } => {
                let instant =
                    self.config.accessibility.reduce_motion || self.screenshot_dir.is_some();
                let fallback = self.tokens_target.surface_base;
                let mut should_arm = false;
                for (mon_idx, hwnd, maybe_color) in samples {
                    if let Some(shell) = self.monitor_shells.get_mut(mon_idx)
                        && shell.maximized_bg_hwnd == Some(hwnd)
                    {
                        if let Some(c) = maybe_color {
                            self.window_bg_cache.insert(hwnd, c);
                            shell.backdrop_transition.set_active_color(c, instant);
                            if !instant && shell.backdrop_transition.is_active() {
                                should_arm = true;
                            }
                        } else if allow_fallback
                            && !self.window_bg_cache.contains_key(&hwnd)
                            && shell.backdrop_transition.opacity() <= 0.03
                        {
                            shell.backdrop_transition.set_active_color(fallback, instant);
                            if !instant && shell.backdrop_transition.is_active() {
                                should_arm = true;
                            }
                        }
                    }
                }
                if should_arm {
                    self.arm_animation();
                }
                Task::none()
            }
            Message::ConfigReloaded(mut cfg) => {
                if self.cli_preset_locked {
                    cfg.apply_preset(self.config.preset);
                }
                self.apply_new_config(cfg, false)
            }
            Message::Hotkey(ev) => match ev {
                HotkeyEvent::EmergencyRestoreTaskbar => {
                    self.shutdown_shell_cleanup();
                    iced::exit()
                }
                HotkeyEvent::ToggleLauncher => {
                    let mon_idx = monitor_index_from_cursor(&self.monitor_infos());
                    self.toggle_launcher_on(mon_idx)
                }
                HotkeyEvent::ToggleSpaces => {
                    let mon_idx = monitor_index_from_cursor(&self.monitor_infos());
                    self.toggle_spaces_on(mon_idx)
                }
            },
            Message::ActiveAppMenuUpdated(menu) => {
                if self.screenshot_dir.is_some() && self.open_global_menu_on_boot {
                    return Task::none();
                }
                self.active_app_menu = menu;
                if let Some(gm_id) = self.global_menu_id {
                    self.apply_surface_chrome_and_bounds(gm_id);
                    let cat = self.selected_global_menu_category();
                    let h = global_menu::estimate_height(cat);
                    return window::resize(gm_id, Size::new(global_menu::GLOBAL_MENU_WIDTH, h));
                }
                Task::none()
            }
            Message::OpenGlobalMenu {
                monitor_idx,
                menu_index,
            } => {
                if self.open_global_menu == Some((monitor_idx, menu_index))
                    && self.global_menu_id.is_some()
                {
                    return self.close_global_menu();
                }
                self.open_global_menu_dropdown(monitor_idx, menu_index)
            }
            Message::GlobalMenuInvoke(action) => {
                let current_space_idx = self
                    .spaces
                    .iter()
                    .find(|s| s.is_current)
                    .map_or(0, |s| s.index);
                self.global_menu_worker.invoke(action, current_space_idx);
                self.close_global_menu()
            }
            Message::DecoratedWindowUpdated(maybe_state) => {
                if self.screenshot_dir.is_some()
                    && (self.open_decorator_preview_on_boot || self.open_global_menu_on_boot)
                {
                    return Task::none();
                }
                let prev_hwnd = self.decorated_window.as_ref().map(|d| d.hwnd);
                let prev_max = self.decorated_window.as_ref().map(|d| d.is_maximized);
                let new_hwnd = maybe_state.as_ref().map(|d| d.hwnd);
                let new_max = maybe_state.as_ref().map(|d| d.is_maximized);

                if let Some(st) = &maybe_state {
                    if self.decorator_icon.as_ref().is_none_or(|(h, _)| *h != st.hwnd) {
                        if let Some(ic) = &st.icon {
                            self.decorator_icon = Some((
                                st.hwnd,
                                image::Handle::from_rgba(ic.width, ic.height, ic.pixels.to_vec()),
                            ));
                        } else {
                            self.decorator_icon = None;
                        }
                    }

                    if self.last_inspected_menu_hwnd != Some(st.hwnd) || prev_max != new_max {
                        self.last_inspected_menu_hwnd = Some(st.hwnd);
                        self.global_menu_worker.inspect_window(st.hwnd, st.monitor_idx);
                    }
                } else if self.screenshot_dir.is_none() {
                    self.last_inspected_menu_hwnd = None;
                    if self.global_menu_id.is_none() {
                        self.active_app_menu = None;
                    }
                }

                if prev_hwnd != new_hwnd && self.global_menu_id.is_some() {
                    // Preserve active app menu if focus jumped briefly onto our own GlobalMenu dropdown.
                }

                self.decorated_window = maybe_state;
                if let Some(st) = &self.decorated_window
                    && let Some(rw) = self.running_windows.iter_mut().find(|w| w.hwnd == st.hwnd)
                {
                    rw.is_maximized = st.is_maximized;
                    rw.frame_rect = st.frame_rect;
                    rw.monitor_idx = st.monitor_idx;
                }
                self.apply_dwm_theming();
                self.update_dock_visibility();
                let tuck_task = self.sync_workarea_tuck();
                let deco_task = self.sync_decorator_window();
                Task::batch([tuck_task, deco_task])
            }
            Message::DecoratorHeaderPressed(hwnd) => {
                if hwnd == 0 {
                    return Task::none();
                }
                let is_double = self
                    .last_decorator_press
                    .is_some_and(|(h, t)| h == hwnd && t.elapsed() < Duration::from_millis(380));
                if is_double {
                    self.last_decorator_press = None;
                    toggle_maximize_window(hwnd);
                } else {
                    self.last_decorator_press = Some((hwnd, Instant::now()));
                    begin_drag_window(hwnd);
                }
                Task::none()
            }
            Message::DecoratorMinimize(hwnd) => {
                minimize_window(hwnd);
                Task::none()
            }
            Message::DecoratorToggleMaximize(hwnd) => {
                toggle_maximize_window(hwnd);
                Task::none()
            }
            Message::DecoratorClose(hwnd) => {
                close_window(hwnd);
                Task::none()
            }
            Message::DockPointerEntered(mon_idx) => {
                if let Some(shell) = self.monitor_shells.get_mut(mon_idx) {
                    shell.dock_last_trigger = Some(Instant::now());
                }
                self.arm_animation();
                Task::none()
            }
            Message::DockPointerMoved(mon_idx, pt) => {
                let item_count = self.dock_items.len();
                let base_size = self.config.dock.icon_size;
                let reduce = self.config.accessibility.reduce_motion;
                let hide_tip = self.dock_popover_id.is_some()
                    && self.dock_popover_monitor_idx == mon_idx;
                let hovered_idx = if let Some(shell) = self.monitor_shells.get_mut(mon_idx) {
                    let local_x = map_pointer_to_unscaled_x(
                        base_size,
                        item_count,
                        &shell.dock_magnifier,
                        pt.x,
                    );
                    shell.dock_magnifier.set_pointer_x(Some(local_x));
                    shell.hovered_dock_idx = closest_slot(base_size, item_count, local_x);
                    shell.dock_last_trigger = Some(Instant::now());
                    if !hide_tip && let Some(idx) = shell.hovered_dock_idx {
                        shell.dock_last_tooltip_idx = Some(idx);
                        shell.dock_tooltip_opacity.set_target(1.0);
                    } else {
                        shell.dock_tooltip_opacity.set_target(0.0);
                    }
                    if reduce {
                        shell.dock_magnifier.step(0.0, true);
                        shell.dock_tooltip_opacity.step(0.0, true);
                        if shell.hovered_dock_idx.is_none() || hide_tip {
                            shell.dock_last_tooltip_idx = None;
                        }
                    }
                    shell.hovered_dock_idx
                } else {
                    None
                };
                if !reduce {
                    self.arm_animation();
                }
                if let Some(idx) = hovered_idx
                    && self.dock_items.get(idx).is_some_and(DockItem::is_running)
                {
                    let same = self
                        .dock_hover_preview
                        .is_some_and(|(m, i, _, _)| m == mon_idx && i == idx);
                    if !same {
                        self.dock_hover_preview = Some((mon_idx, idx, Instant::now(), false));
                    }
                } else {
                    self.dock_hover_preview = None;
                }
                Task::none()
            }
            Message::DockPointerExited(mon_idx) => {
                let reduce = self.config.accessibility.reduce_motion;
                if let Some(shell) = self.monitor_shells.get_mut(mon_idx) {
                    shell.dock_magnifier.set_pointer_x(None);
                    shell.hovered_dock_idx = None;
                    shell.dock_tooltip_opacity.set_target(0.0);
                    if reduce {
                        shell.dock_magnifier.step(0.0, true);
                        shell.dock_tooltip_opacity.step(0.0, true);
                        shell.dock_last_tooltip_idx = None;
                    }
                }
                if self
                    .dock_hover_preview
                    .is_some_and(|(m, _, _, _)| m == mon_idx)
                {
                    self.dock_hover_preview = None;
                }
                if !reduce {
                    self.arm_animation();
                }
                Task::none()
            }
            Message::AnimationFrame(now) => {
                let dt = self
                    .last_anim_tick
                    .map_or(1.0 / 60.0, |prev| now.duration_since(prev).as_secs_f32());
                self.last_anim_tick = Some(now);
                let reduce = self.config.accessibility.reduce_motion;
                let mut any_moving = false;

                let theme_was_moving = !self.theme_transition.is_settled();
                if self.theme_transition.step(dt, reduce) {
                    any_moving = true;
                }
                let theme_moved = theme_was_moving || self.tokens != self.tokens_target;
                if theme_moved {
                    self.tokens = self
                        .tokens_from
                        .lerp(&self.tokens_target, self.theme_transition.value);
                }

                let hwnds = &self.hwnds;
                let base_tokens = self.tokens;
                for shell in &mut self.monitor_shells {
                    if shell.dock_magnifier.step(dt, reduce) {
                        any_moving = true;
                    }

                    let reveal_was_moving = !shell.dock_reveal.is_settled();
                    if shell.dock_reveal.step(dt, reduce) {
                        any_moving = true;
                    }
                    if reveal_was_moving || theme_moved {
                        shell.dock_tokens = base_tokens.with_opacity(shell.dock_reveal.value);
                    }
                    if shell.dock_hidden
                        && shell.dock_hwnd_shown
                        && shell.dock_reveal.is_settled()
                        && shell.dock_reveal.value <= 0.01
                        && let Some(dock_id) = shell.dock_id
                        && let Some(&dock_hwnd) = hwnds.get(&dock_id)
                    {
                        set_window_visible(dock_hwnd, false);
                        shell.dock_hwnd_shown = false;
                    }

                    if shell.dock_tooltip_opacity.step(dt, reduce) {
                        any_moving = true;
                    }
                    if shell.dock_tooltip_opacity.is_settled()
                        && shell.dock_tooltip_opacity.value <= 0.01
                    {
                        shell.dock_last_tooltip_idx = None;
                    }

                    if shell.backdrop_transition.step(dt, reduce) {
                        any_moving = true;
                    }
                    if shell.pantheon_solid.step(dt, reduce) {
                        any_moving = true;
                    }
                }

                let mut expired_toast_close = false;
                let toast_id = self.toast_id;
                let dock_pop_id = self.dock_popover_id;
                for (&win_id, anim) in &mut self.window_anims {
                    let was_moving = !anim.progress.is_settled();
                    if anim.progress.step(dt, reduce) {
                        any_moving = true;
                    }
                    let op = anim.progress.value.clamp(0.0, 1.0);
                    if was_moving || theme_moved {
                        anim.tokens = base_tokens.with_opacity(op);
                    }
                    if dock_pop_id == Some(win_id) {
                        let u8_op = if reduce {
                            255
                        } else {
                            (op * 255.0).round().clamp(0.0, 255.0) as u8
                        };
                        self.dwm_thumbnails.set_opacity(u8_op);
                    }
                    if anim.closing
                        && anim.progress.is_settled()
                        && anim.progress.value <= 0.02
                        && toast_id == Some(win_id)
                    {
                        expired_toast_close = true;
                    }
                }

                if !any_moving {
                    self.last_anim_tick = None;
                }
                let close_toast_task = if expired_toast_close {
                    self.close_toast()
                } else {
                    Task::none()
                };
                let hover_task = self.evaluate_dock_hover_preview();
                Task::batch([close_toast_task, hover_task])
            }
            Message::DockItemClicked(mon_idx, idx) => {
                self.dock_hover_preview = None;
                self.dock_hover_outside_since = None;
                let close_pop = self.close_dock_popover();
                if let Some(item) = self.dock_items.get(idx).cloned() {
                    if item.windows.len() > 1 {
                        let open_pop = self.open_dock_popover_on(
                            mon_idx,
                            idx,
                            dock_menu::DockPopoverMode::WindowPicker,
                        );
                        return Task::batch([close_pop, open_pop]);
                    } else if let Some(hwnd) = item.primary_hwnd() {
                        focus_or_toggle_window(hwnd);
                    } else if let Some(target) = &item.launch_target {
                        launch_app_or_target(target);
                    }
                }
                close_pop
            }
            Message::DockRightClicked(mon_idx) => {
                self.dock_hover_preview = None;
                self.dock_hover_outside_since = None;
                if let Some(idx) = self
                    .monitor_shells
                    .get(mon_idx)
                    .and_then(|s| s.hovered_dock_idx)
                {
                    return self.open_dock_popover_on(
                        mon_idx,
                        idx,
                        dock_menu::DockPopoverMode::ContextMenu,
                    );
                }
                Task::none()
            }
            Message::DockMenuLaunchNew(item_id) => {
                if let Some(item) = self.dock_items.iter().find(|i| i.id == item_id) {
                    if let Some(target) = &item.launch_target {
                        launch_app_or_target(target);
                    } else if let Some(first_win) = item.windows.first()
                        && !first_win.exe_path.is_empty()
                    {
                        launch_target(&first_win.exe_path);
                    }
                }
                self.close_dock_popover()
            }
            Message::DockMenuSwitchToPicker => {
                if let Some(target) = &mut self.dock_popover_target {
                    target.mode = dock_menu::DockPopoverMode::WindowPicker;
                }
                self.dock_popover_opened_by_hover = false;
                self.dock_hover_outside_since = None;
                self.refresh_dock_preview_layout();
                let mut tasks = Vec::new();
                if let Some(id) = self.dock_popover_id {
                    self.apply_surface_chrome_and_bounds(id);
                    if let Some(size) = self.dock_popover_logical_size() {
                        tasks.push(window::resize(id, size));
                    }
                }
                tasks.push(self.sync_dock_thumbnail_slots());
                Task::batch(tasks)
            }
            Message::DockMenuTogglePin(item_id) => {
                let mut sync_task = Task::none();
                if let Some(item) = self.dock_items.iter().find(|i| i.id == item_id).cloned() {
                    let mut next = self.config.clone();
                    if item.is_pinned {
                        let key = if !item.exe_name.is_empty() {
                            item.exe_name.as_str()
                        } else {
                            item.launch_target.as_deref().unwrap_or("")
                        };
                        next.unpin_app(key);
                    } else {
                        let target = item
                            .launch_target
                            .clone()
                            .or_else(|| item.windows.first().map(|w| w.exe_path.clone()))
                            .unwrap_or_else(|| item.exe_name.clone());
                        next.pin_app(PinnedApp {
                            name: item.label.clone(),
                            target,
                            exe_name: item.exe_name.clone(),
                        });
                    }
                    sync_task = self.apply_new_config(next, true);
                }
                Task::batch([sync_task, self.close_dock_popover()])
            }
            Message::DockMenuCloseAll(item_id) => {
                if let Some(item) = self.dock_items.iter().find(|i| i.id == item_id) {
                    for win in &item.windows {
                        close_window(win.hwnd);
                    }
                }
                self.close_dock_popover()
            }
            Message::FocusSpecificWindow(hwnd) => {
                focus_or_toggle_window(hwnd);
                self.close_dock_popover()
            }
            Message::CloseSpecificWindow(hwnd) => {
                close_window(hwnd);
                Task::none()
            }
            Message::ToggleLauncherOn(mon_idx) => self.toggle_launcher_on(mon_idx),
            Message::LauncherQueryChanged(q) => {
                self.launcher_query = q;
                self.launcher_page = 0;
                self.launcher_selected_idx = 0;
                Task::none()
            }
            Message::LauncherViewModeChanged(mode) => {
                self.launcher_view_mode = mode;
                self.launcher_page = 0;
                self.launcher_selected_idx = 0;
                let mut next = self.config.clone();
                next.launcher.default_view = mode;
                self.config = next;
                let _ = self.config.save_atomic();
                Task::none()
            }
            Message::LauncherCategorySelected(cat) => {
                self.launcher_category = cat;
                self.launcher_page = 0;
                self.launcher_selected_idx = 0;
                Task::none()
            }
            Message::LauncherPageChanged(page) => {
                let per_page = (self.config.launcher.cols * self.config.launcher.rows).max(1);
                let filtered = filter_and_sort_apps(
                    &self.installed_apps,
                    &self.launcher_query,
                    self.launcher_category,
                );
                let total_pages = filtered.len().div_ceil(per_page).max(1);
                self.launcher_page = page.min(total_pages.saturating_sub(1));
                self.launcher_selected_idx = self.launcher_page * per_page;
                Task::none()
            }
            Message::LauncherLaunchApp(aumid) => {
                launch_app_aumid(&aumid);
                self.close_launcher()
            }
            Message::LauncherLaunchSelected => {
                let filtered = filter_and_sort_apps(
                    &self.installed_apps,
                    &self.launcher_query,
                    self.launcher_category,
                );
                let app = filtered.get(self.launcher_selected_idx).copied().or_else(|| {
                    let per_page = (self.config.launcher.cols * self.config.launcher.rows).max(1);
                    filtered.get(self.launcher_page * per_page).copied()
                });
                if let Some(target_app) = app {
                    launch_app_aumid(&target_app.aumid);
                    return self.close_launcher();
                }
                Task::none()
            }
            Message::LauncherTogglePin(aumid) => {
                if let Some(app) = self
                    .installed_apps
                    .iter()
                    .find(|a| a.aumid == aumid)
                    .cloned()
                {
                    let mut next = self.config.clone();
                    if next.is_pinned(&app.aumid) {
                        next.unpin_app(&app.aumid);
                    } else {
                        let exe_guess = infer_exe_name_from_aumid(&app.aumid);
                        next.pin_app(PinnedApp {
                            name: app.name.clone(),
                            target: app.aumid.clone(),
                            exe_name: exe_guess.clone(),
                        });
                        if let Some(icon) = app.icon.clone().or_else(|| resolve_aumid_icon(&app.aumid)) {
                            let key = if !exe_guess.is_empty() {
                                exe_guess.to_ascii_lowercase()
                            } else {
                                app.aumid.to_ascii_lowercase()
                            };
                            self.pinned_icons.insert(key, icon);
                        }
                    }
                    return self.apply_new_config(next, true);
                }
                Task::none()
            }
            Message::CyclePreset => {
                self.cli_preset_locked = false;
                let mut next = self.config.clone();
                let next_preset = next.preset.next_cycle();
                next.apply_preset(next_preset);
                let label = next_preset.label();
                let sync_task = self.apply_new_config(next, true);
                let toast_task = self.push_notification(
                    "Appearance",
                    format!("{label} active"),
                    "Shell preset updated live.",
                    NotificationLevel::Success,
                );
                Task::batch([sync_task, toast_task])
            }
            Message::ToggleTheme => {
                self.cli_preset_locked = false;
                let mut next = self.config.clone();
                next.toggle_light_dark();
                self.apply_new_config(next, true)
            }
            Message::ToggleSpaces => self.toggle_spaces_on(self.active_monitor_idx),
            Message::ToggleSpacesOn(mon_idx) => self.toggle_spaces_on(mon_idx),
            Message::SwitchSpace(target_idx) => {
                let current_idx = self
                    .spaces
                    .iter()
                    .find(|s| s.is_current)
                    .map_or(0, |s| s.index);
                let close = self.close_spaces();
                let wins_clone = self.running_windows.clone();
                let refresh = Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            switch_to_space(current_idx, target_idx);
                            std::thread::sleep(Duration::from_millis(180));
                            enumerate_spaces(&wins_clone)
                        })
                        .await
                        .unwrap_or_default()
                    },
                    Message::SpacesUpdated,
                );
                Task::batch([close, refresh])
            }
            Message::CreateNewSpace => {
                let wins_clone = self.running_windows.clone();
                let refresh = Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            create_new_space();
                            std::thread::sleep(Duration::from_millis(220));
                            enumerate_spaces(&wins_clone)
                        })
                        .await
                        .unwrap_or_default()
                    },
                    Message::SpacesUpdated,
                );
                let toast = self.push_notification(
                    "Spaces",
                    "New space created",
                    "Use Win + Ctrl + ← / → to move between spaces.",
                    NotificationLevel::Success,
                );
                Task::batch([refresh, toast])
            }
            Message::ToggleNotifications => self.toggle_notifications_on(self.active_monitor_idx),
            Message::ToggleNotificationsOn(mon_idx) => self.toggle_notifications_on(mon_idx),
            Message::DismissNotification(id) => {
                if let Some(wpn_id) = self.notif_id_to_wpn.get(&id).cloned() {
                    self.dismissed_wpn_ids.insert(wpn_id);
                }
                self.notifications.retain(|n| n.id != id);
                if self.active_toast.as_ref().is_some_and(|t| t.id == id) {
                    return self.close_toast();
                }
                Task::none()
            }
            Message::ClearAllNotifications => {
                for n in &self.notifications {
                    if let Some(wpn_id) = self.notif_id_to_wpn.get(&n.id) {
                        self.dismissed_wpn_ids.insert(wpn_id.clone());
                    }
                }
                self.notifications.clear();
                self.close_toast()
            }
            Message::ToastExpired(id) => {
                if self.active_toast.as_ref().is_some_and(|t| t.id == id) {
                    if !self.config.accessibility.reduce_motion
                        && self.screenshot_dir.is_none()
                        && let Some(tid) = self.toast_id
                        && let Some(anim) = self.window_anims.get_mut(&tid)
                    {
                        anim.closing = true;
                        anim.progress.set_target(0.0);
                        self.arm_animation();
                        return Task::none();
                    }
                    return self.close_toast();
                }
                Task::none()
            }
            Message::ToggleTrayOn(mon_idx) => self.toggle_tray_on(mon_idx),
            Message::TrayIconLeftClicked(idx) => self.activate_tray_item(idx, TrayClickKind::LeftClick),
            Message::TrayIconRightClicked(idx) => {
                self.activate_tray_item(idx, TrayClickKind::RightClick)
            }
            Message::RefreshTrayIcons => Task::perform(
                async move {
                    tokio::task::spawn_blocking(enumerate_tray_icons)
                        .await
                        .unwrap_or_default()
                },
                Message::TrayIconsUpdated,
            ),
            Message::ToggleCalendarOn(mon_idx) => self.toggle_calendar_on(mon_idx),
            Message::CalendarPrevMonth => {
                let (y, m) = prev_month(self.calendar_year, self.calendar_month);
                self.calendar_year = y;
                self.calendar_month = m;
                Task::none()
            }
            Message::CalendarNextMonth => {
                let (y, m) = next_month(self.calendar_year, self.calendar_month);
                self.calendar_year = y;
                self.calendar_month = m;
                Task::none()
            }
            Message::CalendarToday => {
                let today = self.now.date_naive();
                self.calendar_year = today.year();
                self.calendar_month = today.month();
                self.calendar_selected_date = today;
                Task::none()
            }
            Message::CalendarSelectDate(date) => {
                self.calendar_selected_date = date;
                self.calendar_year = date.year();
                self.calendar_month = date.month();
                Task::none()
            }
            Message::CalendarDraftChanged(draft) => {
                self.calendar_draft = draft;
                Task::none()
            }
            Message::CalendarAddEvent => {
                if self
                    .calendar_store
                    .add_event(self.calendar_selected_date, &self.calendar_draft)
                    .is_some()
                {
                    self.calendar_draft.clear();
                    let _ = self.calendar_store.save_atomic();
                }
                Task::none()
            }
            Message::CalendarToggleEvent(id) => {
                self.calendar_store.toggle_event(id);
                let _ = self.calendar_store.save_atomic();
                Task::none()
            }
            Message::CalendarDeleteEvent(id) => {
                self.calendar_store.delete_event(id);
                let _ = self.calendar_store.save_atomic();
                Task::none()
            }
            Message::ToggleQuickControls => {
                self.toggle_quick_controls_on(self.active_monitor_idx)
            }
            Message::ToggleQuickControlsOn(mon_idx) => self.toggle_quick_controls_on(mon_idx),
            Message::VolumeChanged(pct) => {
                self.summary.volume_pct = pct;
                if pct > 0 {
                    self.summary.is_muted = false;
                }
                let _ = self.hw_tx.send(HardwareCmd::Volume(pct));
                Task::none()
            }
            Message::ToggleMute => {
                let next_mute = !self.summary.is_muted;
                self.summary.is_muted = next_mute;
                let _ = self.hw_tx.send(HardwareCmd::Mute(next_mute));
                Task::none()
            }
            Message::BrightnessChanged(pct) => {
                self.summary.brightness_pct = Some(pct);
                let _ = self.hw_tx.send(HardwareCmd::Brightness(pct));
                Task::none()
            }
            Message::ToggleFocusMode => {
                self.focus_mode = !self.focus_mode;
                if self.focus_mode {
                    return self.close_toast();
                }
                self.push_notification(
                    "Focus",
                    "Focus mode off",
                    "Notifications and updates resumed.",
                    NotificationLevel::Info,
                )
            }
            Message::TogglePowerConfirm => {
                self.power_confirm_open = !self.power_confirm_open;
                if let Some(id) = self.quick_controls_id {
                    self.apply_surface_chrome_and_bounds(id);
                    let h = quick_controls::estimate_height(&self.summary, self.power_confirm_open);
                    return window::resize(
                        id,
                        Size::new(quick_controls::QUICK_CONTROLS_WIDTH, h),
                    );
                }
                Task::none()
            }
            Message::PowerLock => {
                let close = self.close_quick_controls();
                lock_workstation();
                close
            }
            Message::PowerSleep => {
                let close = self.close_quick_controls();
                sleep_system();
                close
            }
            Message::PowerRestart => {
                self.shutdown_shell_cleanup();
                restart_system();
                iced::exit()
            }
            Message::PowerShutdown => {
                self.shutdown_shell_cleanup();
                shutdown_system();
                iced::exit()
            }
            Message::OpenTargetAndDismiss(target) => {
                launch_app_or_target(&target);
                Task::batch([
                    self.close_quick_controls(),
                    self.close_notifications(),
                    self.close_tray_popover(),
                    self.close_calendar(),
                    self.close_global_menu(),
                ])
            }
            Message::OpenSettings => self.open_settings(),
            Message::CloseSettings => self.close_settings(),
            Message::MinimizeSettings => {
                if let Some(id) = self.settings_id {
                    window::minimize(id, true)
                } else {
                    Task::none()
                }
            }
            Message::DragSettingsWindow => {
                if let Some(id) = self.settings_id {
                    window::drag(id)
                } else {
                    Task::none()
                }
            }
            Message::SettingsTabSelected(tab) => {
                self.settings_tab = tab;
                Task::none()
            }
            Message::SettingsSelectPreset(preset) => {
                self.cli_preset_locked = false;
                let mut next = self.config.clone();
                next.apply_preset(preset);
                self.apply_new_config(next, true)
            }
            Message::SettingsSelectThemeMode(mode) => {
                self.cli_preset_locked = false;
                let mut next = self.config.clone();
                next.theme_mode = mode;
                next.preset = ShellPreset::Custom;
                self.apply_new_config(next, true)
            }
            Message::SettingsSelectMaterial(mat) => {
                self.cli_preset_locked = false;
                let mut next = self.config.clone();
                next.surface_material = mat;
                next.preset = ShellPreset::Custom;
                self.apply_new_config(next, true)
            }
            Message::SettingsSelectAccent(accent) => {
                self.cli_preset_locked = false;
                let mut next = self.config.clone();
                next.accent = accent;
                next.preset = ShellPreset::Custom;
                self.apply_new_config(next, true)
            }
            Message::SettingsToggleReduceMotion => {
                self.cli_preset_locked = false;
                let mut next = self.config.clone();
                next.accessibility.reduce_motion = !next.accessibility.reduce_motion;
                let snap = next.accessibility.reduce_motion;
                let task = self.apply_new_config(next, true);
                if snap {
                    self.last_anim_tick = None;
                    self.theme_transition.snap(1.0);
                    self.tokens = self.tokens_target;
                    self.tokens_from = self.tokens_target;
                    let hwnds = &self.hwnds;
                    for shell in &mut self.monitor_shells {
                        shell.dock_magnifier.step(0.0, true);
                        shell.dock_reveal.step(0.0, true);
                        if shell.dock_hidden
                            && shell.dock_hwnd_shown
                            && let Some(dock_id) = shell.dock_id
                            && let Some(&dock_hwnd) = hwnds.get(&dock_id)
                        {
                            set_window_visible(dock_hwnd, false);
                            shell.dock_hwnd_shown = false;
                        }
                        shell.dock_tooltip_opacity.step(0.0, true);
                        if shell.dock_tooltip_opacity.value <= 0.01 {
                            shell.dock_last_tooltip_idx = None;
                        }
                        shell.backdrop_transition.step(0.0, true);
                        shell.pantheon_solid.step(0.0, true);
                    }
                    for anim in self.window_anims.values_mut() {
                        anim.progress.step(0.0, true);
                    }
                    let close_closing_toast = if self
                        .toast_id
                        .is_some_and(|tid| self.window_anims.get(&tid).is_some_and(|a| a.closing))
                    {
                        self.close_toast()
                    } else {
                        Task::none()
                    };
                    self.dwm_thumbnails.set_opacity(255);
                    self.sync_all_surface_anim_tokens();
                    return Task::batch([task, close_closing_toast]);
                }
                task
            }
            Message::SettingsToggleIncreaseContrast => {
                self.cli_preset_locked = false;
                let mut next = self.config.clone();
                next.accessibility.increase_contrast = !next.accessibility.increase_contrast;
                self.apply_new_config(next, true)
            }
            Message::SettingsSelectTopBarStyle(style) => {
                self.cli_preset_locked = false;
                let mut next = self.config.clone();
                next.top_bar.style = style;
                next.top_bar.height = match style {
                    TopBarStyle::FloatingPills => 46,
                    TopBarStyle::FullWidth => 34,
                };
                next.preset = ShellPreset::Custom;
                self.apply_new_config(next, true)
            }
            Message::SettingsSelectTopBarHeight(height) => {
                self.cli_preset_locked = false;
                let mut next = self.config.clone();
                next.top_bar.height = height.clamp(28, 60);
                next.preset = ShellPreset::Custom;
                self.apply_new_config(next, true)
            }
            Message::SettingsToggleShowSeconds => {
                self.cli_preset_locked = false;
                let mut next = self.config.clone();
                next.top_bar.show_seconds = !next.top_bar.show_seconds;
                self.apply_new_config(next, true)
            }
            Message::SettingsToggleGlobalMenu => {
                let mut next = self.config.clone();
                next.top_bar.show_global_menu = !next.top_bar.show_global_menu;
                let close = self.close_global_menu();
                let sync = self.apply_new_config(next, true);
                Task::batch([close, sync])
            }
            Message::SettingsDockIconSizeChanged(px) => {
                self.cli_preset_locked = false;
                let mut next = self.config.clone();
                let old_pad = next.dock.height.saturating_sub(next.dock.icon_size).clamp(8, 28);
                let new_icon = px.clamp(28, 64);
                next.dock.icon_size = new_icon;
                next.dock.height = (new_icon + old_pad).clamp(36, 84);
                self.apply_new_config(next, true)
            }
            Message::SettingsDockHeightChanged(h) => {
                self.cli_preset_locked = false;
                let mut next = self.config.clone();
                let new_h = h.clamp(36, 84);
                next.dock.height = new_h;
                if next.dock.icon_size + 8 > new_h {
                    next.dock.icon_size = new_h.saturating_sub(8).clamp(28, 64);
                }
                self.apply_new_config(next, true)
            }
            Message::SettingsDockMagnifyChanged(scale) => {
                self.cli_preset_locked = false;
                let mut next = self.config.clone();
                next.dock.magnification_scale = scale.clamp(1.0, 1.6);
                self.apply_new_config(next, true)
            }
            Message::SettingsSelectDockVisibility(vis) => {
                let mut next = self.config.clone();
                next.dock.visibility = vis;
                next.dock.auto_hide = vis == DockVisibility::AutoHide;
                self.apply_new_config(next, true)
            }
            Message::SettingsToggleDockReserveWorkArea => {
                let mut next = self.config.clone();
                next.dock.reserve_work_area = !next.dock.reserve_work_area;
                self.apply_new_config(next, true)
            }
            Message::SettingsToggleDockAllMonitors => {
                let mut next = self.config.clone();
                next.dock.all_monitors = !next.dock.all_monitors;
                self.apply_new_config(next, true)
            }
            Message::SettingsSelectDockBackgroundStyle(style) => {
                self.cli_preset_locked = false;
                let mut next = self.config.clone();
                next.dock.background_style = style;
                next.preset = ShellPreset::Custom;
                self.apply_new_config(next, true)
            }
            Message::SettingsDockBackgroundOpacityChanged(op) => {
                self.cli_preset_locked = false;
                let mut next = self.config.clone();
                next.dock.background_opacity = op.clamp(0, 100);
                next.preset = ShellPreset::Custom;
                self.apply_new_config(next, true)
            }
            Message::SettingsDockBackgroundColorChanged(hex) => {
                self.cli_preset_locked = false;
                let mut next = self.config.clone();
                next.dock.background_color_hex = hex.chars().take(9).collect();
                next.dock.background_style = DockBackgroundStyle::CustomColor;
                next.preset = ShellPreset::Custom;
                self.apply_new_config(next, true)
            }
            Message::SettingsDockCornerRadiusChanged(r) => {
                self.cli_preset_locked = false;
                let mut next = self.config.clone();
                next.dock.corner_radius = r.clamp(6, 34);
                self.apply_new_config(next, true)
            }
            Message::SettingsToggleDockBorder => {
                self.cli_preset_locked = false;
                let mut next = self.config.clone();
                next.dock.show_border = !next.dock.show_border;
                self.apply_new_config(next, true)
            }
            Message::SettingsToggleDockSpaceChip => {
                self.cli_preset_locked = false;
                let mut next = self.config.clone();
                next.dock.show_space_chip = !next.dock.show_space_chip;
                next.preset = ShellPreset::Custom;
                self.apply_new_config(next, true)
            }
            Message::SettingsToggleDockResumeChip => {
                self.cli_preset_locked = false;
                let mut next = self.config.clone();
                next.dock.show_resume_chip = !next.dock.show_resume_chip;
                next.preset = ShellPreset::Custom;
                self.apply_new_config(next, true)
            }
            Message::SettingsToggleDecoratorEnabled => {
                let mut next = self.config.clone();
                next.decorator.enabled = !next.decorator.enabled;
                self.apply_new_config(next, true)
            }
            Message::SettingsToggleDecoratorDwmSync => {
                let mut next = self.config.clone();
                next.decorator.dwm_theme_sync = !next.decorator.dwm_theme_sync;
                self.apply_new_config(next, true)
            }
            Message::SettingsToggleDecoratorOverlay => {
                let mut next = self.config.clone();
                next.decorator.custom_overlay = !next.decorator.custom_overlay;
                self.apply_new_config(next, true)
            }
            Message::SettingsToggleDecoratorMaxTopBar => {
                let mut next = self.config.clone();
                next.decorator.maximized_topbar_controls = !next.decorator.maximized_topbar_controls;
                self.apply_new_config(next, true)
            }
            Message::SettingsToggleHideMaximizedTitlebar => {
                let mut next = self.config.clone();
                next.decorator.hide_maximized_titlebar = !next.decorator.hide_maximized_titlebar;
                self.apply_new_config(next, true)
            }
            Message::SettingsMaximizedTitlebarHeightChanged(px) => {
                let mut next = self.config.clone();
                next.decorator.maximized_titlebar_height = px.clamp(24, 44);
                self.apply_new_config(next, true)
            }
            Message::SettingsSelectDecoratorButtonStyle(style) => {
                let mut next = self.config.clone();
                next.decorator.button_style = style;
                self.apply_new_config(next, true)
            }
            Message::SettingsToggleCalmZone => {
                self.cli_preset_locked = false;
                let mut next = self.config.clone();
                next.calm_zone.enabled = !next.calm_zone.enabled;
                next.preset = ShellPreset::Custom;
                self.apply_new_config(next, true)
            }
            Message::SettingsCalmUserNameChanged(name) => {
                self.cli_preset_locked = false;
                let mut next = self.config.clone();
                next.calm_zone.user_name = name.chars().take(32).collect();
                self.apply_new_config(next, true)
            }
            Message::SettingsToggleCalmThread => {
                self.cli_preset_locked = false;
                let mut next = self.config.clone();
                next.calm_zone.show_thread_widget = !next.calm_zone.show_thread_widget;
                self.apply_new_config(next, true)
            }
            Message::SettingsCalmThreadTitleChanged(title) => {
                self.cli_preset_locked = false;
                let mut next = self.config.clone();
                next.calm_zone.thread_title = title.chars().take(48).collect();
                self.apply_new_config(next, true)
            }
            Message::SettingsCalmThreadSubtitleChanged(sub) => {
                self.cli_preset_locked = false;
                let mut next = self.config.clone();
                next.calm_zone.thread_subtitle = sub.chars().take(72).collect();
                self.apply_new_config(next, true)
            }
            Message::SettingsToggleHideTaskbar => {
                let want_hide = !self.config.hide_native_taskbar;
                if want_hide && !self.watchdog_spawned {
                    match spawn_watchdog() {
                        Ok(_) => self.watchdog_spawned = true,
                        Err(err) => {
                            tracing::error!(?err, "Refusing to hide taskbar without recovery watchdog");
                            return self.push_notification(
                                "Taskbar Guard",
                                "Taskbar overlay unavailable",
                                "Failed to start recovery watchdog; keeping Windows Taskbar visible.",
                                NotificationLevel::Caution,
                            );
                        }
                    }
                }
                if let Ok(mut guard) = self.taskbar_guard.lock() {
                    guard.set_hidden(want_hide);
                }
                let mut next = self.config.clone();
                next.hide_native_taskbar = want_hide;
                self.apply_new_config(next, true)
            }
            Message::SettingsOpenConfigFile => {
                let _ = self.config.save_atomic();
                launch_target(&self.config_path_str);
                Task::none()
            }
            Message::SettingsResetDefaults => {
                self.cli_preset_locked = false;
                let mut next = ShellConfig::default();
                if self.companion_mode {
                    next.hide_native_taskbar = false;
                }
                if let Ok(mut guard) = self.taskbar_guard.lock() {
                    guard.set_hidden(next.hide_native_taskbar);
                }
                let sync = self.apply_new_config(next, true);
                let toast = self.push_notification(
                    "Settings",
                    "Defaults restored",
                    "Horizon Light preset and default layout restored.",
                    NotificationLevel::Success,
                );
                Task::batch([sync, toast])
            }
            Message::CaptureScreenshots => {
                let top_bar_id = self.primary_shell().top_bar_id;
                let mut tasks = vec![
                    window::screenshot(top_bar_id)
                        .map(|shot| Message::ScreenshotCaptured("topbar", shot)),
                ];
                if let Some(dock_id) = self.primary_shell().dock_id {
                    tasks.push(
                        window::screenshot(dock_id)
                            .map(|shot| Message::ScreenshotCaptured("dock", shot)),
                    );
                }
                if let Some(czid) = self.calm_zone_id {
                    tasks.push(
                        window::screenshot(czid)
                            .map(|shot| Message::ScreenshotCaptured("calm_zone", shot)),
                    );
                }
                if let Some(lid) = self.launcher_id {
                    tasks.push(
                        window::screenshot(lid)
                            .map(|shot| Message::ScreenshotCaptured("launcher", shot)),
                    );
                }
                if let Some(qcid) = self.quick_controls_id {
                    tasks.push(
                        window::screenshot(qcid)
                            .map(|shot| Message::ScreenshotCaptured("quick_controls", shot)),
                    );
                }
                if let Some(sid) = self.spaces_id {
                    tasks.push(
                        window::screenshot(sid)
                            .map(|shot| Message::ScreenshotCaptured("spaces", shot)),
                    );
                }
                if let Some(nid) = self.notifications_id {
                    tasks.push(
                        window::screenshot(nid)
                            .map(|shot| Message::ScreenshotCaptured("notifications", shot)),
                    );
                }
                if let Some(stid) = self.settings_id {
                    tasks.push(
                        window::screenshot(stid)
                            .map(|shot| Message::ScreenshotCaptured("settings", shot)),
                    );
                }
                if let Some(gmid) = self.global_menu_id {
                    tasks.push(
                        window::screenshot(gmid)
                            .map(|shot| Message::ScreenshotCaptured("global_menu", shot)),
                    );
                }
                if let Some(did) = self.decorator_id {
                    tasks.push(
                        window::screenshot(did)
                            .map(|shot| Message::ScreenshotCaptured("window_decorator", shot)),
                    );
                }
                if let Some(cid) = self.calendar_id {
                    tasks.push(
                        window::screenshot(cid)
                            .map(|shot| Message::ScreenshotCaptured("calendar", shot)),
                    );
                }
                if let Some(tid) = self.tray_popover_id {
                    tasks.push(
                        window::screenshot(tid)
                            .map(|shot| Message::ScreenshotCaptured("tray", shot)),
                    );
                }
                if let Some(dpid) = self.dock_popover_id {
                    tasks.push(
                        window::screenshot(dpid)
                            .map(|shot| Message::ScreenshotCaptured("dock_thumbnails", shot)),
                    );
                }
                Task::batch(tasks)
            }
            Message::ScreenshotCaptured(kind, shot) => {
                if let Some(dir) = &self.screenshot_dir {
                    let _ = std::fs::create_dir_all(dir);
                    let preset_slug = format!("{:?}", self.config.preset).to_ascii_lowercase();
                    let raw_path = dir.join(format!("{preset_slug}_{kind}_raw.png"));
                    let _ = ::image::save_buffer(
                        raw_path,
                        &shot.rgba,
                        shot.size.width,
                        shot.size.height,
                        ::image::ColorType::Rgba8,
                    );
                }
                self.screenshots_saved += 1;
                if self.screenshots_saved >= self.screenshots_expected {
                    self.shutdown_shell_cleanup();
                    iced::exit()
                } else {
                    Task::none()
                }
            }
            Message::ExitRequested => {
                self.shutdown_shell_cleanup();
                iced::exit()
            }
        }
    }

    pub fn view(&self, id: window::Id) -> Element<'_, Message> {
        let active_space_name = self
            .spaces
            .iter()
            .find(|s| s.is_current)
            .map_or("Personal", |s| s.name.as_str());

        match self.windows.get(&id) {
            Some(SurfaceKind::TopBar) => {
                let mon_idx = self
                    .top_bar_to_mon
                    .get(&id)
                    .copied()
                    .unwrap_or(self.primary_monitor_idx);
                let active_menu = self
                    .active_app_menu
                    .as_ref()
                    .filter(|_| self.config.top_bar.show_global_menu);
                let max_win = self
                    .decorated_window
                    .as_ref()
                    .filter(|d| d.is_maximized && d.monitor_idx == mon_idx);
                let shell = self.monitor_shells.get(mon_idx);

                top_bar::view(top_bar::TopBarState {
                    config: &self.config,
                    tokens: &self.tokens,
                    summary: &self.summary,
                    now: self.now,
                    monitor_idx: mon_idx,
                    focus_mode: self.focus_mode,
                    quick_controls_open: self.quick_controls_id.is_some()
                        && self.active_monitor_idx == mon_idx,
                    notifications_open: self.notifications_id.is_some()
                        && self.active_monitor_idx == mon_idx,
                    tray_open: self.tray_popover_id.is_some()
                        && self.active_monitor_idx == mon_idx,
                    calendar_open: self.calendar_id.is_some()
                        && self.active_monitor_idx == mon_idx,
                    has_unread_notifs: !self.notifications.is_empty(),
                    active_menu,
                    open_global_menu: self.open_global_menu,
                    maximized_window: max_win,
                    backdrop_color: shell.and_then(|s| s.backdrop_transition.current_color()),
                    backdrop_opacity: shell.map_or(0.0, |s| s.backdrop_transition.opacity()),
                    pantheon_solid_progress: shell
                        .map_or(0.0, |s| s.pantheon_solid.value.clamp(0.0, 1.0)),
                })
            }
            Some(SurfaceKind::Dock) => {
                let mon_idx = self
                    .dock_to_mon
                    .get(&id)
                    .copied()
                    .unwrap_or(self.primary_monitor_idx);
                let shell = &self.monitor_shells[mon_idx];
                let hide_tooltip =
                    self.dock_popover_id.is_some() && self.dock_popover_monitor_idx == mon_idx;
                let reveal = shell.dock_reveal.value;
                let tooltip_opacity = shell.dock_tooltip_opacity.value;
                let tooltip_idx = if !hide_tooltip && tooltip_opacity > 0.01 {
                    shell.hovered_dock_idx.or(shell.dock_last_tooltip_idx)
                } else {
                    None
                };
                dock::view(
                    &self.config,
                    &shell.dock_tokens,
                    &self.dock_items,
                    &shell.dock_magnifier,
                    tooltip_idx,
                    &self.icon_handles,
                    active_space_name,
                    mon_idx,
                    hide_tooltip,
                    reveal,
                    tooltip_opacity,
                )
            }
            Some(SurfaceKind::Launcher) => {
                let anim = self.window_anims.get(&id);
                let tokens = anim.map_or(&self.tokens, |a| &a.tokens);
                wrap_surface_slide(
                    launcher::view(launcher::LauncherState {
                        config: &self.config,
                        tokens,
                        apps: &self.installed_apps,
                        app_icons: &self.app_icon_handles,
                        query: &self.launcher_query,
                        view_mode: self.launcher_view_mode,
                        selected_category: self.launcher_category,
                        page: self.launcher_page,
                        selected_idx: self.launcher_selected_idx,
                        apps_loading: self.apps_loading,
                    }),
                    anim,
                )
            }
            Some(SurfaceKind::DockPopover) => {
                let anim = self.window_anims.get(&id);
                let tokens = anim.map_or(&self.tokens, |a| &a.tokens);
                if let Some(target) = &self.dock_popover_target
                    && let Some(item) = self
                        .dock_items
                        .iter()
                        .find(|i| i.id == target.dock_item_id)
                {
                    match target.mode {
                        dock_menu::DockPopoverMode::ContextMenu => {
                            dock_menu::view(tokens, item, target.mode, &self.icon_handles)
                        }
                        dock_menu::DockPopoverMode::WindowPicker => {
                            if let Some(layout) = &self.dock_preview_layout {
                                let live_dwm_active =
                                    self.screenshot_dir.is_none() && self.hwnds.contains_key(&id);
                                dock_thumbnails::view(
                                    tokens,
                                    item,
                                    layout,
                                    &self.icon_handles,
                                    &self.dock_fallback_thumbs,
                                    live_dwm_active,
                                    Message::FocusSpecificWindow,
                                    Message::CloseSpecificWindow,
                                )
                            } else {
                                iced::widget::Space::new().into()
                            }
                        }
                    }
                } else {
                    iced::widget::Space::new().into()
                }
            }
            Some(SurfaceKind::QuickControls) => {
                let anim = self.window_anims.get(&id);
                let tokens = anim.map_or(&self.tokens, |a| &a.tokens);
                wrap_surface_slide(
                    quick_controls::view(
                        tokens,
                        &self.summary,
                        self.focus_mode,
                        self.power_confirm_open,
                    ),
                    anim,
                )
            }
            Some(SurfaceKind::Spaces) => {
                let anim = self.window_anims.get(&id);
                let tokens = anim.map_or(&self.tokens, |a| &a.tokens);
                wrap_surface_slide(spaces::view(tokens, &self.spaces), anim)
            }
            Some(SurfaceKind::CalmZone) => calm_zone::view(&self.config, &self.tokens, self.now),
            Some(SurfaceKind::Notifications) => {
                let anim = self.window_anims.get(&id);
                let tokens = anim.map_or(&self.tokens, |a| &a.tokens);
                wrap_surface_slide(
                    notifications::view_center(tokens, &self.notifications, self.focus_mode),
                    anim,
                )
            }
            Some(SurfaceKind::Toast) => {
                let anim = self.window_anims.get(&id);
                let tokens = anim.map_or(&self.tokens, |a| &a.tokens);
                if let Some(notif) = &self.active_toast {
                    wrap_surface_slide(notifications::view_toast(tokens, notif), anim)
                } else {
                    iced::widget::Space::new().into()
                }
            }
            Some(SurfaceKind::TrayPopover) => {
                let anim = self.window_anims.get(&id);
                let tokens = anim.map_or(&self.tokens, |a| &a.tokens);
                wrap_surface_slide(
                    tray::view(
                        tokens,
                        &self.tray_icons,
                        &self.tray_icon_handles,
                        Message::TrayIconLeftClicked,
                        Message::TrayIconRightClicked,
                        Message::RefreshTrayIcons,
                    ),
                    anim,
                )
            }
            Some(SurfaceKind::Calendar) => {
                let anim = self.window_anims.get(&id);
                let tokens = anim.map_or(&self.tokens, |a| &a.tokens);
                wrap_surface_slide(
                    calendar::view(
                        tokens,
                        self.now,
                        self.calendar_year,
                        self.calendar_month,
                        self.calendar_selected_date,
                        self.calendar_store.monday_first,
                        &self.calendar_store.events,
                        &self.calendar_draft,
                        Message::CalendarPrevMonth,
                        Message::CalendarNextMonth,
                        Message::CalendarToday,
                        Message::CalendarSelectDate,
                        Message::CalendarDraftChanged,
                        Message::CalendarAddEvent,
                        Message::CalendarToggleEvent,
                        Message::CalendarDeleteEvent,
                    ),
                    anim,
                )
            }
            Some(SurfaceKind::Settings) => settings::view(settings::SettingsState {
                config: &self.config,
                tokens: &self.tokens,
                primary_monitor: &self.primary_shell().info,
                monitor_count: self.monitor_shells.len(),
                running_window_count: self.running_windows.len(),
                space_count: self.spaces.len(),
                installed_app_count: self.installed_apps.len(),
                active_tab: self.settings_tab,
                config_path: &self.config_path_str,
            }),
            Some(SurfaceKind::GlobalMenu) => {
                let anim = self.window_anims.get(&id);
                let tokens = anim.map_or(&self.tokens, |a| &a.tokens);
                let cat = self.selected_global_menu_category();
                wrap_surface_slide(global_menu::view(tokens, cat), anim)
            }
            Some(SurfaceKind::WindowDecorator) => {
                let anim = self.window_anims.get(&id);
                let tokens = anim.map_or(&self.tokens, |a| &a.tokens);
                if let Some(st) = &self.decorated_window {
                    let icon = self.decorator_icon.as_ref().and_then(|(h, img)| {
                        (*h == st.hwnd).then_some(img)
                    });
                    wrap_surface_slide(
                        window_decorator::view(tokens, &self.config.decorator, st, icon),
                        anim,
                    )
                } else {
                    iced::widget::Space::new().into()
                }
            }
            None => iced::widget::Space::new().into(),
        }
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let win_events_sub = window::events().map(|(id, ev)| Message::WindowEvent(id, ev));
        let backend_sub = Subscription::run(shell_backend_worker);

        let mut subs = vec![win_events_sub, backend_sub];
        if self.screenshot_dir.is_none() {
            subs.push(
                iced::time::every(Duration::from_secs(1)).map(|_| Message::Tick(Local::now())),
            );
        }

        if self.launcher_id.is_some()
            || self.dock_popover_id.is_some()
            || self.quick_controls_id.is_some()
            || self.spaces_id.is_some()
            || self.notifications_id.is_some()
            || self.tray_popover_id.is_some()
            || self.calendar_id.is_some()
            || self.global_menu_id.is_some()
        {
            subs.push(keyboard::listen().map(Message::KeyboardEvent));
        }

        if self.has_active_animations() {
            subs.push(window::frames().map(|_| Message::AnimationFrame(Instant::now())));
        }

        if self
            .dock_hover_preview
            .is_some_and(|(_, _, _, triggered)| !triggered)
            || self.is_hover_window_picker_open()
        {
            subs.push(
                iced::time::every(Duration::from_millis(80)).map(|_| Message::DockSensorTick),
            );
        }

        Subscription::batch(subs)
    }

    fn arm_animation(&mut self) {
        if !self.config.accessibility.reduce_motion && self.screenshot_dir.is_none() {
            self.last_anim_tick.get_or_insert_with(Instant::now);
        }
    }

    fn has_active_animations(&self) -> bool {
        !self.theme_transition.is_settled()
            || self.monitor_shells.iter().any(|s| {
                s.dock_magnifier.is_active()
                    || !s.dock_reveal.is_settled()
                    || !s.dock_tooltip_opacity.is_settled()
                    || s.backdrop_transition.is_active()
                    || !s.pantheon_solid.is_settled()
            })
            || self
                .window_anims
                .values()
                .any(|a| !a.progress.is_settled() || a.closing)
    }

    fn sync_all_surface_anim_tokens(&mut self) {
        for shell in &mut self.monitor_shells {
            shell.dock_tokens = self.tokens.with_opacity(shell.dock_reveal.value);
        }
        for anim in self.window_anims.values_mut() {
            anim.tokens = self.tokens.with_opacity(anim.progress.value);
        }
    }

    fn start_surface_open_anim(&mut self, id: window::Id) {
        let Some(&kind) = self.windows.get(&id) else {
            return;
        };
        let (slide_px, from_top) = match kind {
            SurfaceKind::QuickControls
            | SurfaceKind::Notifications
            | SurfaceKind::TrayPopover
            | SurfaceKind::Calendar
            | SurfaceKind::GlobalMenu
            | SurfaceKind::Toast => (6.0, true),
            SurfaceKind::Launcher | SurfaceKind::Spaces | SurfaceKind::WindowDecorator => {
                (8.0, false)
            }
            SurfaceKind::DockPopover => (0.0, false),
            SurfaceKind::TopBar
            | SurfaceKind::Dock
            | SurfaceKind::CalmZone
            | SurfaceKind::Settings => return,
        };
        let instant = self.config.accessibility.reduce_motion || self.screenshot_dir.is_some();
        let (progress, tokens) = if instant {
            (Spring::snappy(1.0), self.tokens)
        } else {
            let mut p = Spring::snappy(0.0);
            p.set_target(1.0);
            (p, self.tokens.with_opacity(0.0))
        };
        if kind == SurfaceKind::DockPopover {
            self.dwm_thumbnails.set_opacity(if instant { 255 } else { 0 });
        }
        self.window_anims.insert(
            id,
            SurfaceAnim {
                progress,
                closing: false,
                slide_px,
                from_top,
                tokens,
            },
        );
        if !instant {
            self.arm_animation();
        }
    }

    fn monitor_infos(&self) -> Vec<MonitorInfo> {
        self.monitor_shells.iter().map(|s| s.info.clone()).collect()
    }

    fn primary_shell(&self) -> &MonitorShell {
        self.monitor_shells
            .get(self.primary_monitor_idx)
            .unwrap_or(&self.monitor_shells[0])
    }

    fn active_monitor(&self) -> &MonitorInfo {
        self.monitor_shells
            .get(self.active_monitor_idx)
            .map(|s| &s.info)
            .unwrap_or(&self.monitor_shells[0].info)
    }

    fn selected_global_menu_category(&self) -> Option<&GlobalMenuCategory> {
        let (_, menu_idx) = self.open_global_menu?;
        self.active_app_menu
            .as_ref()
            .and_then(|m| m.categories.get(menu_idx))
    }

    fn has_active_shell_popup(&self) -> bool {
        self.launcher_id.is_some()
            || self.dock_popover_id.is_some()
            || self.quick_controls_id.is_some()
            || self.spaces_id.is_some()
            || self.notifications_id.is_some()
            || self.tray_popover_id.is_some()
            || self.calendar_id.is_some()
            || self.global_menu_id.is_some()
            || self.settings_id.is_some()
    }

    fn close_all_popups_except_settings(&mut self) -> Task<Message> {
        Task::batch([
            self.close_launcher(),
            self.close_dock_popover(),
            self.close_quick_controls(),
            self.close_spaces(),
            self.close_notifications(),
            self.close_tray_popover(),
            self.close_calendar(),
            self.close_global_menu(),
        ])
    }

    fn toggle_launcher_on(&mut self, mon_idx: usize) -> Task<Message> {
        if self.launcher_id.is_some() && self.active_monitor_idx == mon_idx {
            self.close_launcher()
        } else {
            let close_old = self.close_launcher();
            let open_new = self.open_launcher_on(mon_idx);
            Task::batch([close_old, open_new])
        }
    }

    fn open_launcher_on(&mut self, mon_idx: usize) -> Task<Message> {
        let mon_idx = mon_idx.min(self.monitor_shells.len().saturating_sub(1));
        self.active_monitor_idx = mon_idx;
        let close_pop = self.close_dock_popover();
        let close_qc = self.close_quick_controls();
        let close_sp = self.close_spaces();
        let close_nf = self.close_notifications();
        let close_tr = self.close_tray_popover();
        let close_cal = self.close_calendar();
        let close_gm = self.close_global_menu();

        self.launcher_query.clear();
        self.launcher_page = 0;
        self.launcher_selected_idx = 0;
        self.launcher_view_mode = self.config.launcher.default_view;
        self.launcher_category = AppCategory::All;

        let logical = self.active_monitor().logical_bounds();
        let x = logical.x + 8.0;
        let y = logical.y + self.config.top_bar.height as f32 + 4.0;
        let (id, open_task) = window::open(window::Settings {
            size: Size::new(
                launcher::LAUNCHER_WINDOW_WIDTH,
                launcher::LAUNCHER_WINDOW_HEIGHT,
            ),
            position: window::Position::Specific(Point::new(x, y)),
            decorations: false,
            transparent: true,
            resizable: false,
            level: window::Level::AlwaysOnTop,
            exit_on_close_request: false,
            ..window::Settings::default()
        });
        self.launcher_id = Some(id);
        self.launcher_opened_at = Some(Instant::now());
        self.windows.insert(id, SurfaceKind::Launcher);
        self.sync_decorator_window_visibility_and_bounds();

        Task::batch([
            close_pop,
            close_qc,
            close_sp,
            close_nf,
            close_tr,
            close_cal,
            close_gm,
            open_task.map(Message::WindowOpened),
            window::gain_focus(id),
            operation::focus(launcher::search_input_id()),
        ])
    }

    fn toggle_quick_controls_on(&mut self, mon_idx: usize) -> Task<Message> {
        if self.quick_controls_id.is_some() && self.active_monitor_idx == mon_idx {
            self.close_quick_controls()
        } else {
            let close_old = self.close_quick_controls();
            let open_new = self.open_quick_controls_on(mon_idx);
            Task::batch([close_old, open_new])
        }
    }

    fn open_quick_controls_on(&mut self, mon_idx: usize) -> Task<Message> {
        let mon_idx = mon_idx.min(self.monitor_shells.len().saturating_sub(1));
        self.active_monitor_idx = mon_idx;
        let close_launcher = self.close_launcher();
        let close_pop = self.close_dock_popover();
        let close_sp = self.close_spaces();
        let close_nf = self.close_notifications();
        let close_tr = self.close_tray_popover();
        let close_cal = self.close_calendar();
        let close_gm = self.close_global_menu();
        self.power_confirm_open = false;

        let logical = self.active_monitor().logical_bounds();
        let w = quick_controls::QUICK_CONTROLS_WIDTH;
        let h = quick_controls::estimate_height(&self.summary, false);
        let x = logical.x + logical.width - w - 8.0;
        let y = logical.y + self.config.top_bar.height as f32 + 4.0;

        let (id, open_task) = window::open(window::Settings {
            size: Size::new(w, h),
            position: window::Position::Specific(Point::new(x, y)),
            decorations: false,
            transparent: true,
            resizable: false,
            level: window::Level::AlwaysOnTop,
            exit_on_close_request: false,
            ..window::Settings::default()
        });
        self.quick_controls_id = Some(id);
        self.quick_controls_opened_at = Some(Instant::now());
        self.windows.insert(id, SurfaceKind::QuickControls);
        self.sync_decorator_window_visibility_and_bounds();

        Task::batch([
            close_launcher,
            close_pop,
            close_sp,
            close_nf,
            close_tr,
            close_cal,
            close_gm,
            open_task.map(Message::WindowOpened),
            window::gain_focus(id),
        ])
    }

    fn close_quick_controls(&mut self) -> Task<Message> {
        if let Some(id) = self.quick_controls_id.take() {
            self.quick_controls_opened_at = None;
            self.power_confirm_open = false;
            self.windows.remove(&id);
            self.hwnds.remove(&id);
            self.sync_decorator_window_visibility_and_bounds();
            window::close(id)
        } else {
            Task::none()
        }
    }

    fn toggle_spaces_on(&mut self, mon_idx: usize) -> Task<Message> {
        if self.spaces_id.is_some() && self.active_monitor_idx == mon_idx {
            self.close_spaces()
        } else {
            let close_old = self.close_spaces();
            let open_new = self.open_spaces_on(mon_idx);
            Task::batch([close_old, open_new])
        }
    }

    fn open_spaces_on(&mut self, mon_idx: usize) -> Task<Message> {
        let mon_idx = mon_idx.min(self.monitor_shells.len().saturating_sub(1));
        self.active_monitor_idx = mon_idx;
        self.spaces = enumerate_spaces(&self.running_windows);
        let close_launcher = self.close_launcher();
        let close_pop = self.close_dock_popover();
        let close_qc = self.close_quick_controls();
        let close_nf = self.close_notifications();
        let close_tr = self.close_tray_popover();
        let close_cal = self.close_calendar();
        let close_gm = self.close_global_menu();

        let logical = self.active_monitor().logical_bounds();
        let w = spaces::SPACES_WINDOW_WIDTH;
        let h = spaces::SPACES_WINDOW_HEIGHT;
        let x = logical.x + (logical.width - w) * 0.5;
        let y = logical.y + (logical.height - h) * 0.45;

        let (id, open_task) = window::open(window::Settings {
            size: Size::new(w, h),
            position: window::Position::Specific(Point::new(x, y)),
            decorations: false,
            transparent: true,
            resizable: false,
            level: window::Level::AlwaysOnTop,
            exit_on_close_request: false,
            ..window::Settings::default()
        });
        self.spaces_id = Some(id);
        self.spaces_opened_at = Some(Instant::now());
        self.windows.insert(id, SurfaceKind::Spaces);
        self.sync_decorator_window_visibility_and_bounds();

        Task::batch([
            close_launcher,
            close_pop,
            close_qc,
            close_nf,
            close_tr,
            close_cal,
            close_gm,
            open_task.map(Message::WindowOpened),
            window::gain_focus(id),
        ])
    }

    fn open_global_menu_dropdown(&mut self, mon_idx: usize, menu_index: usize) -> Task<Message> {
        let mon_idx = mon_idx.min(self.monitor_shells.len().saturating_sub(1));
        self.active_monitor_idx = mon_idx;
        self.open_global_menu = Some((mon_idx, menu_index));

        if let Some(menu) = &self.active_app_menu
            && let Some(cat) = menu.categories.get(menu_index)
            && !cat.loaded
        {
            self.global_menu_worker.load_submenu(menu.hwnd, menu_index);
        }

        if let Some(id) = self.global_menu_id {
            self.global_menu_opened_at = Some(Instant::now());
            self.apply_surface_chrome_and_bounds(id);
            let h = global_menu::estimate_height(self.selected_global_menu_category());
            return Task::batch([
                window::resize(id, Size::new(global_menu::GLOBAL_MENU_WIDTH, h)),
                window::gain_focus(id),
            ]);
        }

        let close_launcher = self.close_launcher();
        let close_pop = self.close_dock_popover();
        let close_qc = self.close_quick_controls();
        let close_sp = self.close_spaces();
        let close_nf = self.close_notifications();
        let close_tr = self.close_tray_popover();
        let close_cal = self.close_calendar();

        let logical = self.active_monitor().logical_bounds();
        let w = global_menu::GLOBAL_MENU_WIDTH;
        let h = global_menu::estimate_height(self.selected_global_menu_category());
        let anchor_offset = global_menu_anchor_offset(self.config.top_bar.style, menu_index);
        let x = (logical.x + anchor_offset).clamp(logical.x + 8.0, logical.x + logical.width - w - 8.0);
        let y = logical.y + self.config.top_bar.height as f32 + 4.0;

        let (id, open_task) = window::open(window::Settings {
            size: Size::new(w, h),
            position: window::Position::Specific(Point::new(x, y)),
            decorations: false,
            transparent: true,
            resizable: false,
            level: window::Level::AlwaysOnTop,
            exit_on_close_request: false,
            ..window::Settings::default()
        });
        self.global_menu_id = Some(id);
        self.global_menu_opened_at = Some(Instant::now());
        self.windows.insert(id, SurfaceKind::GlobalMenu);
        self.sync_decorator_window_visibility_and_bounds();

        Task::batch([
            close_launcher,
            close_pop,
            close_qc,
            close_sp,
            close_nf,
            close_tr,
            close_cal,
            open_task.map(Message::WindowOpened),
            window::gain_focus(id),
        ])
    }

    fn close_global_menu(&mut self) -> Task<Message> {
        self.open_global_menu = None;
        if let Some(id) = self.global_menu_id.take() {
            self.global_menu_opened_at = None;
            self.windows.remove(&id);
            self.hwnds.remove(&id);
            self.sync_decorator_window_visibility_and_bounds();
            window::close(id)
        } else {
            Task::none()
        }
    }

    fn open_settings(&mut self) -> Task<Message> {
        if let Some(id) = self.settings_id {
            return Task::batch([window::minimize(id, false), window::gain_focus(id)]);
        }

        let close_all = self.close_all_popups_except_settings();
        let logical = self.active_monitor().logical_bounds();
        let w = settings::SETTINGS_WINDOW_WIDTH;
        let h = settings::SETTINGS_WINDOW_HEIGHT;
        let x = logical.x + (logical.width - w) * 0.5;
        let y = logical.y + (logical.height - h) * 0.46;

        let (id, open_task) = window::open(window::Settings {
            size: Size::new(w, h),
            position: window::Position::Specific(Point::new(x, y)),
            decorations: false,
            transparent: true,
            resizable: false,
            level: window::Level::Normal,
            exit_on_close_request: false,
            ..window::Settings::default()
        });
        self.settings_id = Some(id);
        self.settings_positioned = false;
        self.windows.insert(id, SurfaceKind::Settings);
        self.sync_decorator_window_visibility_and_bounds();

        Task::batch([
            close_all,
            open_task.map(Message::WindowOpened),
            window::gain_focus(id),
        ])
    }

    fn close_settings(&mut self) -> Task<Message> {
        if let Some(id) = self.settings_id.take() {
            self.settings_positioned = false;
            self.windows.remove(&id);
            self.hwnds.remove(&id);
            self.sync_decorator_window_visibility_and_bounds();
            window::close(id)
        } else {
            Task::none()
        }
    }

    fn close_spaces(&mut self) -> Task<Message> {
        if let Some(id) = self.spaces_id.take() {
            self.spaces_opened_at = None;
            self.windows.remove(&id);
            self.hwnds.remove(&id);
            self.sync_decorator_window_visibility_and_bounds();
            window::close(id)
        } else {
            Task::none()
        }
    }

    fn toggle_notifications_on(&mut self, mon_idx: usize) -> Task<Message> {
        if self.notifications_id.is_some() && self.active_monitor_idx == mon_idx {
            self.close_notifications()
        } else {
            let close_old = self.close_notifications();
            let open_new = self.open_notifications_on(mon_idx);
            Task::batch([close_old, open_new])
        }
    }

    fn open_notifications_on(&mut self, mon_idx: usize) -> Task<Message> {
        let mon_idx = mon_idx.min(self.monitor_shells.len().saturating_sub(1));
        self.active_monitor_idx = mon_idx;
        let close_launcher = self.close_launcher();
        let close_pop = self.close_dock_popover();
        let close_qc = self.close_quick_controls();
        let close_sp = self.close_spaces();
        let close_tr = self.close_tray_popover();
        let close_cal = self.close_calendar();
        let close_gm = self.close_global_menu();
        let close_toast = self.close_toast();

        let logical = self.active_monitor().logical_bounds();
        let w = notifications::NOTIF_CENTER_WIDTH;
        let h = notifications::NOTIF_CENTER_HEIGHT;
        let x = logical.x + logical.width - w - 8.0;
        let y = logical.y + self.config.top_bar.height as f32 + 4.0;

        let (id, open_task) = window::open(window::Settings {
            size: Size::new(w, h),
            position: window::Position::Specific(Point::new(x, y)),
            decorations: false,
            transparent: true,
            resizable: false,
            level: window::Level::AlwaysOnTop,
            exit_on_close_request: false,
            ..window::Settings::default()
        });
        self.notifications_id = Some(id);
        self.notifications_opened_at = Some(Instant::now());
        self.windows.insert(id, SurfaceKind::Notifications);
        self.sync_decorator_window_visibility_and_bounds();

        Task::batch([
            close_launcher,
            close_pop,
            close_qc,
            close_sp,
            close_tr,
            close_cal,
            close_gm,
            close_toast,
            open_task.map(Message::WindowOpened),
            window::gain_focus(id),
        ])
    }

    fn close_notifications(&mut self) -> Task<Message> {
        if let Some(id) = self.notifications_id.take() {
            self.notifications_opened_at = None;
            self.windows.remove(&id);
            self.hwnds.remove(&id);
            self.sync_decorator_window_visibility_and_bounds();
            window::close(id)
        } else {
            Task::none()
        }
    }

    fn toggle_tray_on(&mut self, mon_idx: usize) -> Task<Message> {
        if self.tray_popover_id.is_some() && self.active_monitor_idx == mon_idx {
            self.close_tray_popover()
        } else {
            let close_old = self.close_tray_popover();
            let open_new = self.open_tray_popover_on(mon_idx);
            Task::batch([close_old, open_new])
        }
    }

    fn open_tray_popover_on(&mut self, mon_idx: usize) -> Task<Message> {
        let mon_idx = mon_idx.min(self.monitor_shells.len().saturating_sub(1));
        self.active_monitor_idx = mon_idx;
        let close_launcher = self.close_launcher();
        let close_pop = self.close_dock_popover();
        let close_qc = self.close_quick_controls();
        let close_sp = self.close_spaces();
        let close_nf = self.close_notifications();
        let close_cal = self.close_calendar();
        let close_gm = self.close_global_menu();

        if self.screenshot_dir.is_some() && self.tray_icons.is_empty() {
            self.populate_demo_tray_icons();
        }

        let logical = self.active_monitor().logical_bounds();
        let w = tray::TRAY_POPOVER_WIDTH;
        let h = tray::estimate_height(self.tray_icons.len());
        let x = self.tray_popover_logical_x(&logical, w);
        let y = logical.y + self.config.top_bar.height as f32 + 4.0;

        let (id, open_task) = window::open(window::Settings {
            size: Size::new(w, h),
            position: window::Position::Specific(Point::new(x, y)),
            decorations: false,
            transparent: true,
            resizable: false,
            level: window::Level::AlwaysOnTop,
            exit_on_close_request: false,
            ..window::Settings::default()
        });
        self.tray_popover_id = Some(id);
        self.tray_popover_opened_at = Some(Instant::now());
        self.windows.insert(id, SurfaceKind::TrayPopover);
        self.sync_decorator_window_visibility_and_bounds();

        let mut batch = vec![
            close_launcher,
            close_pop,
            close_qc,
            close_sp,
            close_nf,
            close_cal,
            close_gm,
            open_task.map(Message::WindowOpened),
            window::gain_focus(id),
        ];
        if self.screenshot_dir.is_none() {
            batch.push(Task::perform(
                async move {
                    tokio::task::spawn_blocking(enumerate_tray_icons)
                        .await
                        .unwrap_or_default()
                },
                Message::TrayIconsUpdated,
            ));
        }
        Task::batch(batch)
    }

    fn close_tray_popover(&mut self) -> Task<Message> {
        if let Some(id) = self.tray_popover_id.take() {
            self.tray_popover_opened_at = None;
            self.windows.remove(&id);
            if let Some(hwnd_raw) = self.hwnds.remove(&id) {
                set_window_visible(hwnd_raw, false);
            }
            self.sync_decorator_window_visibility_and_bounds();
            window::close(id)
        } else {
            Task::none()
        }
    }

    fn apply_tray_icons_snapshot(&mut self, icons: Vec<TrayIconInfo>) -> Task<Message> {
        sync_rgba_icon_handles(
            &mut self.tray_icon_sources,
            &mut self.tray_icon_handles,
            icons.iter().map(|item| (item.id.as_str(), item.icon.as_ref())),
        );
        self.tray_icons = icons;
        if let Some(id) = self.tray_popover_id {
            self.apply_surface_chrome_and_bounds(id);
            let h = tray::estimate_height(self.tray_icons.len());
            return window::resize(id, Size::new(tray::TRAY_POPOVER_WIDTH, h));
        }
        Task::none()
    }

    fn activate_tray_item(&mut self, idx: usize, kind: TrayClickKind) -> Task<Message> {
        let Some(info) = self.tray_icons.get(idx).cloned() else {
            return Task::none();
        };
        let cursor = cursor_position().unwrap_or((0, 0));
        let close = self.close_tray_popover();
        let activate = Task::perform(
            async move {
                let _ = tokio::task::spawn_blocking(move || {
                    activate_tray_icon(&info, kind, cursor);
                })
                .await;
            },
            |_| Message::RefreshTrayIcons,
        );
        Task::batch([close, activate])
    }

    fn toggle_calendar_on(&mut self, mon_idx: usize) -> Task<Message> {
        if self.calendar_id.is_some() && self.active_monitor_idx == mon_idx {
            self.close_calendar()
        } else {
            let close_old = self.close_calendar();
            let open_new = self.open_calendar_on(mon_idx);
            Task::batch([close_old, open_new])
        }
    }

    fn open_calendar_on(&mut self, mon_idx: usize) -> Task<Message> {
        let mon_idx = mon_idx.min(self.monitor_shells.len().saturating_sub(1));
        self.active_monitor_idx = mon_idx;
        let close_launcher = self.close_launcher();
        let close_pop = self.close_dock_popover();
        let close_qc = self.close_quick_controls();
        let close_sp = self.close_spaces();
        let close_nf = self.close_notifications();
        let close_tr = self.close_tray_popover();
        let close_gm = self.close_global_menu();

        let today = self.now.date_naive();
        self.calendar_year = today.year();
        self.calendar_month = today.month();
        self.calendar_selected_date = today;
        self.calendar_draft.clear();

        if self.screenshot_dir.is_some() && self.calendar_store.events.is_empty() {
            self.calendar_store.add_event(today, "Morning standup · 09:30");
            self.calendar_store
                .add_event(today, "Review VitaDesktop sprint");
            if let Some(first_id) = self.calendar_store.events.first().map(|e| e.id) {
                self.calendar_store.toggle_event(first_id);
            }
            if let Some(tomorrow) = today.succ_opt() {
                self.calendar_store
                    .add_event(tomorrow, "Ship release checklist");
            }
        }

        let logical = self.active_monitor().logical_bounds();
        let w = calendar::CALENDAR_WIDTH;
        let h = calendar::CALENDAR_HEIGHT;
        let x = self.calendar_logical_x(&logical, w);
        let y = logical.y + self.config.top_bar.height as f32 + 4.0;

        let (id, open_task) = window::open(window::Settings {
            size: Size::new(w, h),
            position: window::Position::Specific(Point::new(x, y)),
            decorations: false,
            transparent: true,
            resizable: false,
            level: window::Level::AlwaysOnTop,
            exit_on_close_request: false,
            ..window::Settings::default()
        });
        self.calendar_id = Some(id);
        self.calendar_opened_at = Some(Instant::now());
        self.windows.insert(id, SurfaceKind::Calendar);
        self.sync_decorator_window_visibility_and_bounds();

        Task::batch([
            close_launcher,
            close_pop,
            close_qc,
            close_sp,
            close_nf,
            close_tr,
            close_gm,
            open_task.map(Message::WindowOpened),
            window::gain_focus(id),
        ])
    }

    fn close_calendar(&mut self) -> Task<Message> {
        if let Some(id) = self.calendar_id.take() {
            self.calendar_opened_at = None;
            self.windows.remove(&id);
            self.hwnds.remove(&id);
            self.sync_decorator_window_visibility_and_bounds();
            window::close(id)
        } else {
            Task::none()
        }
    }

    fn tray_popover_logical_x(&self, logical: &vita_core::LogicalRect, width: f32) -> f32 {
        let offset = match self.config.top_bar.style {
            TopBarStyle::FloatingPills => 180.0,
            TopBarStyle::FullWidth => 140.0,
        };
        let min_x = logical.x + 8.0;
        let max_x = (logical.x + logical.width - width - 8.0).max(min_x);
        (logical.x + logical.width - width - offset).clamp(min_x, max_x)
    }

    fn calendar_logical_x(&self, logical: &vita_core::LogicalRect, width: f32) -> f32 {
        let min_x = logical.x + 8.0;
        let max_x = (logical.x + logical.width - width - 8.0).max(min_x);
        match self.config.top_bar.style {
            TopBarStyle::FullWidth => (logical.x + (logical.width - width) * 0.5).clamp(min_x, max_x),
            TopBarStyle::FloatingPills => (logical.x + logical.width - width - 48.0).clamp(min_x, max_x),
        }
    }

    fn merge_system_notifications(&mut self, sys_list: Vec<SystemNotification>) -> Task<Message> {
        let newest_unseen = reconcile_wpn_notifications(
            &sys_list,
            &mut self.wpn_seeded,
            &mut self.seen_wpn_ids,
            &self.dismissed_wpn_ids,
            &mut self.wpn_to_notif_id,
            &mut self.notif_id_to_wpn,
            &mut self.next_notif_id,
            &mut self.notifications,
        );

        if let Some(toast) = newest_unseen
            && !self.focus_mode
            && self.notifications_id.is_none()
            && self.screenshot_dir.is_none()
        {
            return self.show_toast_window(toast);
        }
        Task::none()
    }

    fn push_notification(
        &mut self,
        app_name: &str,
        title: impl Into<String>,
        body: impl Into<String>,
        level: NotificationLevel,
    ) -> Task<Message> {
        let id = self.next_notif_id;
        self.next_notif_id += 1;
        let notif = ToastNotification {
            id,
            app_name: app_name.to_string(),
            title: title.into(),
            body: body.into(),
            timestamp: Local::now().format("%H:%M").to_string(),
            level,
        };
        self.notifications.push(notif.clone());
        if self.notifications.len() > 40 {
            let excess = self.notifications.len() - 40;
            self.notifications.drain(0..excess);
        }

        if self.focus_mode || self.notifications_id.is_some() || self.screenshot_dir.is_some() {
            return Task::none();
        }

        self.show_toast_window(notif)
    }

    fn show_toast_window(&mut self, notif: ToastNotification) -> Task<Message> {
        let id = notif.id;
        let close_old = self.close_toast();
        self.active_toast = Some(notif);

        let logical = self.primary_shell().info.logical_bounds();
        let w = notifications::TOAST_WINDOW_WIDTH;
        let h = notifications::TOAST_WINDOW_HEIGHT;
        let x = logical.x + logical.width - w - 8.0;
        let y = logical.y + self.config.top_bar.height as f32 + 6.0;

        let (tid, open_task) = window::open(window::Settings {
            size: Size::new(w, h),
            position: window::Position::Specific(Point::new(x, y)),
            decorations: false,
            transparent: true,
            resizable: false,
            level: window::Level::AlwaysOnTop,
            exit_on_close_request: false,
            ..window::Settings::default()
        });
        self.toast_id = Some(tid);
        self.windows.insert(tid, SurfaceKind::Toast);

        let expire_task = Task::perform(tokio::time::sleep(Duration::from_secs(4)), move |_| {
            Message::ToastExpired(id)
        });

        Task::batch([close_old, open_task.map(Message::WindowOpened), expire_task])
    }

    fn close_toast(&mut self) -> Task<Message> {
        self.active_toast = None;
        if let Some(id) = self.toast_id.take() {
            self.windows.remove(&id);
            self.hwnds.remove(&id);
            self.window_anims.remove(&id);
            window::close(id)
        } else {
            Task::none()
        }
    }

    fn sync_calm_zone(&mut self) -> Task<Message> {
        if self.config.calm_zone.enabled {
            if let Some(id) = self.calm_zone_id {
                self.apply_surface_chrome_and_bounds(id);
                Task::none()
            } else {
                let logical = self.primary_shell().info.logical_bounds();
                let cz_x = logical.x + 28.0;
                let cz_y = logical.y + self.config.top_bar.height as f32 + 22.0;
                let (id, task) = window::open(window::Settings {
                    size: Size::new(calm_zone::CALM_ZONE_WIDTH, calm_zone::CALM_ZONE_HEIGHT),
                    position: window::Position::Specific(Point::new(cz_x, cz_y)),
                    decorations: false,
                    transparent: true,
                    resizable: false,
                    level: window::Level::Normal,
                    exit_on_close_request: false,
                    ..window::Settings::default()
                });
                self.calm_zone_id = Some(id);
                self.windows.insert(id, SurfaceKind::CalmZone);
                task.map(Message::WindowOpened)
            }
        } else if let Some(id) = self.calm_zone_id.take() {
            self.windows.remove(&id);
            self.hwnds.remove(&id);
            window::close(id)
        } else {
            Task::none()
        }
    }

    fn close_launcher(&mut self) -> Task<Message> {
        if let Some(id) = self.launcher_id.take() {
            self.launcher_opened_at = None;
            self.windows.remove(&id);
            self.hwnds.remove(&id);
            self.sync_decorator_window_visibility_and_bounds();
            window::close(id)
        } else {
            Task::none()
        }
    }

    fn open_dock_popover_on(
        &mut self,
        mon_idx: usize,
        idx: usize,
        mode: dock_menu::DockPopoverMode,
    ) -> Task<Message> {
        self.open_dock_popover_on_ex(mon_idx, idx, mode, false)
    }

    fn open_dock_popover_on_ex(
        &mut self,
        mon_idx: usize,
        idx: usize,
        mode: dock_menu::DockPopoverMode,
        opened_by_hover: bool,
    ) -> Task<Message> {
        let mon_idx = mon_idx.min(self.monitor_shells.len().saturating_sub(1));
        self.active_monitor_idx = mon_idx;
        self.dock_popover_monitor_idx = mon_idx;
        if let Some(shell) = self.monitor_shells.get_mut(mon_idx) {
            shell.dock_tooltip_opacity.set_target(0.0);
            if self.config.accessibility.reduce_motion {
                shell.dock_tooltip_opacity.step(0.0, true);
                shell.dock_last_tooltip_idx = None;
            }
        }
        self.arm_animation();
        let Some(item) = self.dock_items.get(idx).cloned() else {
            return Task::none();
        };

        let reuse_existing = opened_by_hover
            && mode == dock_menu::DockPopoverMode::WindowPicker
            && self.dock_popover_id.is_some()
            && self.dock_popover_opened_by_hover
            && self.dock_popover_monitor_idx == mon_idx;

        let close_existing = if reuse_existing {
            Task::none()
        } else {
            self.close_dock_popover()
        };
        let close_launcher = self.close_launcher();
        let close_qc = self.close_quick_controls();
        let close_sp = self.close_spaces();
        let close_nf = self.close_notifications();
        let close_tr = self.close_tray_popover();
        let close_cal = self.close_calendar();
        let close_gm = self.close_global_menu();

        let mon = self.monitor_shells[mon_idx].info.clone();
        let logical = mon.logical_bounds();
        let center_x = dock::slot_screen_center_x(
            &self.config,
            logical.width,
            self.dock_items.len(),
            idx,
        );
        let target = dock_menu::DockPopoverTarget {
            dock_item_id: item.id.clone(),
            anchor_center_x_logical: center_x,
            mode,
        };

        let preview_layout = (mode == dock_menu::DockPopoverMode::WindowPicker)
            .then(|| dock_thumbnails::compute_preview_layout(&item.windows, mon.dpi_scale));
        self.dock_preview_layout = preview_layout;

        let (log_rect, _) = self.compute_dock_popover_geometry(&mon, &item, &target);

        if reuse_existing && let Some(existing_id) = self.dock_popover_id {
            self.dock_fallback_thumbs.clear();
            self.dock_popover_opened_at = Some(Instant::now());
            self.dock_popover_target = Some(target);
            self.dock_popover_opened_by_hover = true;
            self.dock_hover_outside_since = None;
            self.apply_surface_chrome_and_bounds(existing_id);
            let resize_task = window::resize(existing_id, Size::new(log_rect.width, log_rect.height));
            let thumb_task = self.sync_dock_thumbnail_slots();
            return Task::batch([
                close_launcher,
                close_qc,
                close_sp,
                close_nf,
                close_tr,
                close_cal,
                close_gm,
                resize_task,
                thumb_task,
            ]);
        }

        let (id, open_task) = window::open(window::Settings {
            size: Size::new(log_rect.width, log_rect.height),
            position: window::Position::Specific(Point::new(log_rect.x, log_rect.y)),
            decorations: false,
            transparent: true,
            resizable: false,
            level: window::Level::AlwaysOnTop,
            exit_on_close_request: false,
            ..window::Settings::default()
        });
        self.dock_popover_id = Some(id);
        self.dock_popover_opened_at = Some(Instant::now());
        self.dock_popover_target = Some(target);
        self.dock_popover_opened_by_hover = opened_by_hover;
        self.dock_hover_outside_since = None;
        self.windows.insert(id, SurfaceKind::DockPopover);
        self.sync_decorator_window_visibility_and_bounds();
        let thumb_task = self.sync_dock_thumbnail_slots();

        let mut tasks = vec![
            close_existing,
            close_launcher,
            close_qc,
            close_sp,
            close_nf,
            close_tr,
            close_cal,
            close_gm,
            open_task.map(Message::WindowOpened),
            thumb_task,
        ];
        if !opened_by_hover {
            tasks.push(window::gain_focus(id));
        }
        Task::batch(tasks)
    }

    fn close_dock_popover(&mut self) -> Task<Message> {
        self.dwm_thumbnails.clear();
        self.dock_preview_layout = None;
        self.dock_popover_opened_by_hover = false;
        self.dock_hover_outside_since = None;
        let pop_mon = self.dock_popover_monitor_idx;
        if let Some(id) = self.dock_popover_id.take() {
            self.dock_fallback_thumbs.clear();
            self.dock_popover_opened_at = None;
            self.dock_popover_target = None;
            self.windows.remove(&id);
            self.hwnds.remove(&id);
            self.window_anims.remove(&id);
            if let Some(shell) = self.monitor_shells.get_mut(pop_mon)
                && let Some(idx) = shell.hovered_dock_idx
            {
                shell.dock_last_tooltip_idx = Some(idx);
                shell.dock_tooltip_opacity.set_target(1.0);
                if self.config.accessibility.reduce_motion {
                    shell.dock_tooltip_opacity.step(0.0, true);
                }
            }
            self.arm_animation();
            self.sync_decorator_window_visibility_and_bounds();
            window::close(id)
        } else {
            Task::none()
        }
    }

    fn is_hover_window_picker_open(&self) -> bool {
        self.dock_popover_id.is_some()
            && self.dock_popover_opened_by_hover
            && self
                .dock_popover_target
                .as_ref()
                .is_some_and(|t| t.mode == dock_menu::DockPopoverMode::WindowPicker)
    }

    fn has_blocking_popup_for_hover_preview(&self) -> bool {
        self.screenshot_dir.is_some()
            || self.launcher_id.is_some()
            || self.quick_controls_id.is_some()
            || self.spaces_id.is_some()
            || self.notifications_id.is_some()
            || self.tray_popover_id.is_some()
            || self.calendar_id.is_some()
            || self.global_menu_id.is_some()
            || self.settings_id.is_some()
            || self
                .dock_popover_target
                .as_ref()
                .is_some_and(|t| t.mode == dock_menu::DockPopoverMode::ContextMenu)
    }

    fn is_cursor_in_dock_band(&self, mon_idx: usize, cursor: Option<(i32, i32)>) -> bool {
        let Some((cx, cy)) = cursor else {
            return false;
        };
        let Some(shell) = self.monitor_shells.get(mon_idx) else {
            return false;
        };
        let mon = &shell.info;
        let b = mon.physical_bounds;
        let dpi = mon.dpi_scale;
        let dock_w_px = (self.dock_window_width * dpi).round() as i32;
        let dock_h_px = ((dock::pill_height(&self.config)
            + dock::bottom_margin(&self.config)
            + 40.0)
            * dpi)
            .round() as i32;
        PhysicalRect {
            x: b.x + (b.width - dock_w_px) / 2 - 12,
            y: b.bottom() - dock_h_px,
            width: dock_w_px + 24,
            height: dock_h_px,
        }
        .contains_point(cx, cy)
    }

    fn is_cursor_in_dock_popover(&self, cursor: Option<(i32, i32)>) -> bool {
        let Some((cx, cy)) = cursor else {
            return false;
        };
        let Some(target) = &self.dock_popover_target else {
            return false;
        };
        let Some(item) = self
            .dock_items
            .iter()
            .find(|i| i.id == target.dock_item_id)
        else {
            return false;
        };
        let mon = self
            .monitor_shells
            .get(self.dock_popover_monitor_idx)
            .map(|s| &s.info)
            .unwrap_or(&self.monitor_shells[0].info);
        let (_, phys) = self.compute_dock_popover_geometry(mon, item, target);
        PhysicalRect {
            x: phys.x - 6,
            y: phys.y - 6,
            width: phys.width + 12,
            height: phys.height + 12,
        }
        .contains_point(cx, cy)
    }

    fn evaluate_dock_hover_preview(&mut self) -> Task<Message> {
        let cursor = cursor_position();

        if self.is_hover_window_picker_open() {
            let in_dock = self.is_cursor_in_dock_band(self.dock_popover_monitor_idx, cursor);
            let in_popover = self.is_cursor_in_dock_popover(cursor);
            if in_dock || in_popover {
                self.dock_hover_outside_since = None;
            } else if self
                .dock_popover_opened_at
                .is_some_and(|t| t.elapsed() >= Duration::from_millis(300))
            {
                match self.dock_hover_outside_since {
                    None => self.dock_hover_outside_since = Some(Instant::now()),
                    Some(since) if since.elapsed() >= Duration::from_millis(240) => {
                        self.dock_hover_preview = None;
                        return self.close_dock_popover();
                    }
                    _ => {}
                }
            }
        }

        if self.has_blocking_popup_for_hover_preview() {
            return Task::none();
        }

        let Some((mon_idx, item_idx, entered_at, triggered)) = self.dock_hover_preview else {
            return Task::none();
        };
        if triggered {
            return Task::none();
        }
        if !self.is_cursor_in_dock_band(mon_idx, cursor) {
            self.dock_hover_preview = None;
            return Task::none();
        }
        let dwell = if self.is_hover_window_picker_open() {
            Duration::from_millis(140)
        } else {
            Duration::from_millis(520)
        };
        if entered_at.elapsed() < dwell {
            return Task::none();
        }
        let Some(item) = self.dock_items.get(item_idx) else {
            self.dock_hover_preview = None;
            return Task::none();
        };
        if !item.is_running() {
            self.dock_hover_preview = None;
            return Task::none();
        }

        let already_open_for_item = self.dock_popover_id.is_some()
            && self.dock_popover_monitor_idx == mon_idx
            && self.dock_popover_target.as_ref().is_some_and(|t| {
                t.mode == dock_menu::DockPopoverMode::WindowPicker && t.dock_item_id == item.id
            });
        if let Some((_, _, _, is_triggered)) = &mut self.dock_hover_preview {
            *is_triggered = true;
        }
        if already_open_for_item {
            return Task::none();
        }

        self.open_dock_popover_on_ex(
            mon_idx,
            item_idx,
            dock_menu::DockPopoverMode::WindowPicker,
            true,
        )
    }

    fn refresh_dock_preview_layout(&mut self) {
        let Some(target) = &self.dock_popover_target else {
            self.dock_preview_layout = None;
            return;
        };
        if target.mode != dock_menu::DockPopoverMode::WindowPicker {
            self.dock_preview_layout = None;
            return;
        }
        let Some(item) = self
            .dock_items
            .iter()
            .find(|i| i.id == target.dock_item_id)
        else {
            self.dock_preview_layout = None;
            return;
        };
        let mon = self
            .monitor_shells
            .get(self.dock_popover_monitor_idx)
            .map(|s| &s.info)
            .unwrap_or(&self.monitor_shells[0].info);
        self.dock_preview_layout = Some(dock_thumbnails::compute_preview_layout(
            &item.windows,
            mon.dpi_scale,
        ));
    }

    fn dock_popover_logical_size(&self) -> Option<Size> {
        let target = self.dock_popover_target.as_ref()?;
        let item = self
            .dock_items
            .iter()
            .find(|i| i.id == target.dock_item_id)?;
        let mon = self
            .monitor_shells
            .get(self.dock_popover_monitor_idx)
            .map(|s| &s.info)
            .unwrap_or(&self.monitor_shells[0].info);
        let (log_rect, _) = self.compute_dock_popover_geometry(mon, item, target);
        Some(Size::new(log_rect.width, log_rect.height))
    }

    fn compute_dock_popover_geometry(
        &self,
        mon: &MonitorInfo,
        item: &DockItem,
        target: &dock_menu::DockPopoverTarget,
    ) -> (vita_core::LogicalRect, PhysicalRect) {
        let (pop_w, pop_h) = match target.mode {
            dock_menu::DockPopoverMode::ContextMenu => (
                dock_menu::DOCK_POPOVER_WIDTH,
                dock_menu::estimate_popover_height(item, target.mode),
            ),
            dock_menu::DockPopoverMode::WindowPicker => {
                if let Some(layout) = &self.dock_preview_layout {
                    (layout.window_width, layout.window_height)
                } else {
                    let layout =
                        dock_thumbnails::compute_preview_layout(&item.windows, mon.dpi_scale);
                    (layout.window_width, layout.window_height)
                }
            }
        };

        let (log_rect, phys_rect) = dock_popover_bounds(
            &self.config,
            mon,
            target.anchor_center_x_logical,
            pop_w,
            pop_h,
        );
        (log_rect, phys_rect)
    }

    fn sync_dock_thumbnail_slots(&mut self) -> Task<Message> {
        let Some(pop_id) = self.dock_popover_id else {
            self.dwm_thumbnails.clear();
            return Task::none();
        };
        let Some(target) = &self.dock_popover_target else {
            self.dwm_thumbnails.clear();
            return Task::none();
        };
        if target.mode != dock_menu::DockPopoverMode::WindowPicker {
            self.dwm_thumbnails.clear();
            return Task::none();
        }
        let Some(item) = self
            .dock_items
            .iter()
            .find(|i| i.id == target.dock_item_id)
            .cloned()
        else {
            self.dwm_thumbnails.clear();
            return Task::none();
        };

        self.refresh_dock_preview_layout();
        let Some(layout) = self.dock_preview_layout.clone() else {
            self.dwm_thumbnails.clear();
            return Task::none();
        };

        let mut capture_tasks = Vec::new();
        let mut live_slots = Vec::new();
        let use_live_dwm = self.screenshot_dir.is_none();

        for slot in &layout.slots {
            let Some(win) = item.windows.iter().find(|w| w.hwnd == slot.hwnd) else {
                continue;
            };
            if use_live_dwm && !win.is_minimized && win.hwnd != 0 {
                live_slots.push((slot.hwnd, slot.physical_rect));
            }
            if (win.is_minimized || !use_live_dwm)
                && win.hwnd != 0
                && !self.dock_fallback_thumbs.contains_key(&win.hwnd)
            {
                let hwnd = win.hwnd;
                capture_tasks.push(Task::perform(
                    async move {
                        let thumb = tokio::task::spawn_blocking(move || {
                            capture_window_thumbnail(hwnd, 376, 224)
                        })
                        .await
                        .ok()
                        .flatten();
                        (hwnd, thumb)
                    },
                    |(hwnd, thumb)| Message::DockFallbackThumbCaptured(hwnd, thumb),
                ));
            }
        }

        if use_live_dwm && let Some(&pop_hwnd) = self.hwnds.get(&pop_id) {
            self.dwm_thumbnails.sync_slots(pop_hwnd, &live_slots);
        } else {
            self.dwm_thumbnails.clear();
        }

        Task::batch(capture_tasks)
    }

    fn reconcile_open_window_picker(&mut self) -> Task<Message> {
        let Some(pop_id) = self.dock_popover_id else {
            return Task::none();
        };
        let Some(target) = &self.dock_popover_target else {
            return Task::none();
        };
        if target.mode != dock_menu::DockPopoverMode::WindowPicker {
            return Task::none();
        }
        let still_running = self
            .dock_items
            .iter()
            .find(|i| i.id == target.dock_item_id)
            .is_some_and(DockItem::is_running);
        if !still_running {
            return self.close_dock_popover();
        }
        self.refresh_dock_preview_layout();
        let mut tasks = Vec::new();
        if let Some(size) = self.dock_popover_logical_size() {
            self.apply_surface_chrome_and_bounds(pop_id);
            tasks.push(window::resize(pop_id, size));
        }
        tasks.push(self.sync_dock_thumbnail_slots());
        Task::batch(tasks)
    }

    fn handle_keyboard(&mut self, ev: keyboard::Event) -> Task<Message> {
        let keyboard::Event::KeyPressed { key, .. } = ev else {
            return Task::none();
        };
        let Key::Named(named) = key else {
            return Task::none();
        };

        if named == Named::Escape {
            return Task::batch([
                self.close_launcher(),
                self.close_dock_popover(),
                self.close_quick_controls(),
                self.close_spaces(),
                self.close_notifications(),
                self.close_tray_popover(),
                self.close_calendar(),
                self.close_global_menu(),
            ]);
        }

        if self.launcher_id.is_none() {
            return Task::none();
        }

        let filtered = filter_and_sort_apps(
            &self.installed_apps,
            &self.launcher_query,
            self.launcher_category,
        );
        if filtered.is_empty() {
            return Task::none();
        }

        let cols = if !self.launcher_query.trim().is_empty()
            || self.launcher_view_mode == LauncherViewMode::Grid
        {
            self.config.launcher.cols.max(1)
        } else {
            3
        };
        let per_page = (self.config.launcher.cols * self.config.launcher.rows).max(1);
        let max_idx = filtered.len().saturating_sub(1);

        match named {
            Named::ArrowRight => {
                self.launcher_selected_idx = (self.launcher_selected_idx + 1).min(max_idx);
            }
            Named::ArrowLeft => {
                self.launcher_selected_idx = self.launcher_selected_idx.saturating_sub(1);
            }
            Named::ArrowDown => {
                self.launcher_selected_idx = (self.launcher_selected_idx + cols).min(max_idx);
            }
            Named::ArrowUp => {
                self.launcher_selected_idx = self.launcher_selected_idx.saturating_sub(cols);
            }
            _ => return Task::none(),
        }

        if self.launcher_query.trim().is_empty()
            && self.launcher_view_mode == LauncherViewMode::Grid
        {
            self.launcher_page = self.launcher_selected_idx / per_page;
        }
        Task::none()
    }

    fn apply_new_config(&mut self, cfg: ShellConfig, persist: bool) -> Task<Message> {
        let old_topbar_h = self.config.top_bar.height;
        let old_dock_vis = self.config.dock.visibility;
        let old_hide_tb = self.config.hide_native_taskbar;
        self.config = cfg;
        self.system_dark = system_prefers_dark();
        let new_tokens = resolve_tokens(&self.config, self.system_dark);
        if new_tokens != self.tokens_target {
            if self.config.accessibility.reduce_motion || self.screenshot_dir.is_some() {
                self.tokens_from = new_tokens;
                self.tokens_target = new_tokens;
                self.tokens = new_tokens;
                self.theme_transition.snap(1.0);
            } else {
                self.tokens_from = self.tokens;
                self.tokens_target = new_tokens;
                self.theme_transition.snap(0.0);
                self.theme_transition.set_target(1.0);
                self.arm_animation();
            }
        }
        self.sync_all_surface_anim_tokens();
        for shell in &mut self.monitor_shells {
            shell.dock_magnifier.update_params(
                self.config.dock.magnification_scale,
                self.config.dock.max_magnify_radius,
            );
        }

        if persist {
            let _ = self.config.save_atomic();
        }

        if old_topbar_h != self.config.top_bar.height
            || old_dock_vis != self.config.dock.visibility
            || old_hide_tb != self.config.hide_native_taskbar
        {
            let monitors = self.monitor_infos();
            let bottom_reserve = bottom_reserve_for_config(&self.config);
            if let Ok(mut wg) = self.work_area_guard.lock() {
                wg.apply(
                    &monitors,
                    self.config.top_bar.height,
                    bottom_reserve,
                    self.config.hide_native_taskbar,
                );
            }
        }
        let tuck_task = self.sync_workarea_tuck();

        let dock_task = self.reconcile_dock_windows();

        let top_ids: Vec<window::Id> = self.monitor_shells.iter().map(|s| s.top_bar_id).collect();
        for tid in top_ids {
            self.apply_surface_chrome_and_bounds(tid);
        }
        let dock_ids: Vec<window::Id> = self
            .monitor_shells
            .iter()
            .filter_map(|s| s.dock_id)
            .collect();
        for did in dock_ids {
            self.apply_surface_chrome_and_bounds(did);
        }
        if let Some(lid) = self.launcher_id {
            self.apply_surface_chrome_and_bounds(lid);
        }
        if let Some(pid) = self.dock_popover_id {
            self.apply_surface_chrome_and_bounds(pid);
        }
        if let Some(qcid) = self.quick_controls_id {
            self.apply_surface_chrome_and_bounds(qcid);
        }
        if let Some(sid) = self.spaces_id {
            self.apply_surface_chrome_and_bounds(sid);
        }
        if let Some(nid) = self.notifications_id {
            self.apply_surface_chrome_and_bounds(nid);
        }
        if let Some(tid) = self.tray_popover_id {
            self.apply_surface_chrome_and_bounds(tid);
        }
        if let Some(cid) = self.calendar_id {
            self.apply_surface_chrome_and_bounds(cid);
        }
        if let Some(tid) = self.toast_id {
            self.apply_surface_chrome_and_bounds(tid);
        }
        if let Some(stid) = self.settings_id {
            self.apply_surface_chrome_and_bounds(stid);
        }
        if let Some(gmid) = self.global_menu_id {
            self.apply_surface_chrome_and_bounds(gmid);
        }

        self.apply_dwm_theming();
        let deco_task = self.sync_decorator_window();
        let dock_rebuild_task = self.rebuild_dock_items();
        self.update_dock_visibility();
        let calm_task = self.sync_calm_zone();
        Task::batch([
            tuck_task,
            dock_task,
            dock_rebuild_task,
            deco_task,
            calm_task,
        ])
    }

    fn reconcile_dock_windows(&mut self) -> Task<Message> {
        let mut tasks = Vec::new();
        let mut to_close = Vec::new();
        for idx in 0..self.monitor_shells.len() {
            let want_dock = self.config.dock.all_monitors || idx == self.primary_monitor_idx;
            let has_dock = self.monitor_shells[idx].dock_id.is_some();
            if want_dock && !has_dock {
                let logical = self.monitor_shells[idx].info.logical_bounds();
                let dock_x = logical.x + (logical.width - self.dock_window_width) * 0.5;
                let dock_y = logical.y + logical.height - self.dock_window_height;
                let (d_id, open_d) = window::open(window::Settings {
                    size: Size::new(self.dock_window_width, self.dock_window_height),
                    position: window::Position::Specific(Point::new(dock_x, dock_y)),
                    decorations: false,
                    transparent: true,
                    resizable: false,
                    level: window::Level::AlwaysOnTop,
                    exit_on_close_request: false,
                    ..window::Settings::default()
                });
                self.monitor_shells[idx].dock_id = Some(d_id);
                self.monitor_shells[idx].dock_hidden = false;
                self.windows.insert(d_id, SurfaceKind::Dock);
                self.dock_to_mon.insert(d_id, idx);
                tasks.push(open_d.map(Message::WindowOpened));
            } else if !want_dock && let Some(d_id) = self.monitor_shells[idx].dock_id.take() {
                self.dock_to_mon.remove(&d_id);
                self.windows.remove(&d_id);
                self.hwnds.remove(&d_id);
                to_close.push(window::close(d_id));
            }
        }
        tasks.extend(to_close);
        Task::batch(tasks)
    }

    fn reconcile_monitors(&mut self, new_monitors: Vec<MonitorInfo>) -> Task<Message> {
        if new_monitors.is_empty() {
            return Task::none();
        }
        let close_popups = self.close_all_popups_except_settings();
        let mut close_tasks = vec![close_popups];

        for shell in self.monitor_shells.drain(..) {
            self.top_bar_to_mon.remove(&shell.top_bar_id);
            self.windows.remove(&shell.top_bar_id);
            self.hwnds.remove(&shell.top_bar_id);
            close_tasks.push(window::close(shell.top_bar_id));
            if let Some(d_id) = shell.dock_id {
                self.dock_to_mon.remove(&d_id);
                self.windows.remove(&d_id);
                self.hwnds.remove(&d_id);
                close_tasks.push(window::close(d_id));
            }
        }

        let primary_idx = primary_monitor_index(&new_monitors);
        self.primary_monitor_idx = primary_idx;
        self.active_monitor_idx = primary_idx;

        let bottom_reserve = bottom_reserve_for_config(&self.config);
        if let Ok(mut wg) = self.work_area_guard.lock() {
            wg.apply(
                &new_monitors,
                self.config.top_bar.height,
                bottom_reserve,
                self.config.hide_native_taskbar,
            );
        }

        let (_, slot_centers) = dock::compute_dock_layout(&self.config, self.dock_items.len());
        let mut open_tasks = Vec::new();
        for (idx, mon) in new_monitors.into_iter().enumerate() {
            let logical = mon.logical_bounds();
            let (tb_id, open_tb) = window::open(window::Settings {
                size: Size::new(logical.width, self.config.top_bar.height as f32),
                position: window::Position::Specific(Point::new(logical.x, logical.y)),
                decorations: false,
                transparent: true,
                resizable: false,
                level: window::Level::AlwaysOnTop,
                exit_on_close_request: false,
                ..window::Settings::default()
            });
            self.windows.insert(tb_id, SurfaceKind::TopBar);
            self.top_bar_to_mon.insert(tb_id, idx);
            open_tasks.push(open_tb.map(Message::WindowOpened));

            let mut magnifier = DockMagnifier::new(
                self.config.dock.magnification_scale,
                self.config.dock.max_magnify_radius,
            );
            magnifier.sync_slots(slot_centers.clone());

            let should_open_dock = self.config.dock.all_monitors || idx == primary_idx;
            let dock_id = if should_open_dock {
                let dock_x = logical.x + (logical.width - self.dock_window_width) * 0.5;
                let dock_y = logical.y + logical.height - self.dock_window_height;
                let (d_id, open_d) = window::open(window::Settings {
                    size: Size::new(self.dock_window_width, self.dock_window_height),
                    position: window::Position::Specific(Point::new(dock_x, dock_y)),
                    decorations: false,
                    transparent: true,
                    resizable: false,
                    level: window::Level::AlwaysOnTop,
                    exit_on_close_request: false,
                    ..window::Settings::default()
                });
                self.windows.insert(d_id, SurfaceKind::Dock);
                self.dock_to_mon.insert(d_id, idx);
                open_tasks.push(open_d.map(Message::WindowOpened));
                Some(d_id)
            } else {
                None
            };

            self.monitor_shells.push(MonitorShell {
                info: mon,
                top_bar_id: tb_id,
                dock_id,
                dock_magnifier: magnifier,
                hovered_dock_idx: None,
                dock_hidden: false,
                dock_hwnd_shown: true,
                dock_last_trigger: None,
                maximized_bg_hwnd: None,
                backdrop_transition: BackdropTransition::new(self.tokens.surface_base),
                pantheon_solid: Spring::smooth(0.0),
                dock_reveal: Spring::new(1.0, 260.0, 28.0),
                dock_tokens: self.tokens,
                dock_tooltip_opacity: Spring::snappy(0.0),
                dock_last_tooltip_idx: None,
            });
        }

        self.tucked_monitors.clear();
        let tuck_task = self.sync_workarea_tuck();

        let calm_task = self.sync_calm_zone();
        close_tasks.extend(open_tasks);
        close_tasks.push(tuck_task);
        close_tasks.push(calm_task);
        Task::batch(close_tasks)
    }

    fn update_dock_visibility(&mut self) {
        if self.screenshot_dir.is_some() {
            return;
        }
        let cursor = cursor_position();
        let vis_mode = self.config.dock.visibility;

        for mon_idx in 0..self.monitor_shells.len() {
            let Some(dock_id) = self.monitor_shells[mon_idx].dock_id else {
                continue;
            };
            let Some(&dock_hwnd) = self.hwnds.get(&dock_id) else {
                continue;
            };

            let mon = &self.monitor_shells[mon_idx].info;
            let b = mon.physical_bounds;
            let dpi = mon.dpi_scale;
            let dock_w_px = (self.dock_window_width * dpi).round() as i32;
            let dock_band_h_px = ((dock::pill_height(&self.config)
                + dock::bottom_margin(&self.config)
                + 8.0)
                * dpi)
                .round() as i32;
            let dock_band_x = b.x + (b.width - dock_w_px) / 2;
            let dock_band = PhysicalRect {
                x: dock_band_x,
                y: b.bottom() - dock_band_h_px,
                width: dock_w_px,
                height: dock_band_h_px,
            };

            let trigger_strip = PhysicalRect {
                x: b.x,
                y: b.bottom() - 5,
                width: b.width,
                height: 5,
            };

            let cursor_in_trigger = cursor.is_some_and(|(cx, cy)| trigger_strip.contains_point(cx, cy));
            if cursor_in_trigger {
                self.monitor_shells[mon_idx].dock_last_trigger = Some(Instant::now());
            }

            let cursor_near_shown_dock = !self.monitor_shells[mon_idx].dock_hidden
                && cursor.is_some_and(|(cx, cy)| {
                    PhysicalRect {
                        x: dock_band.x - 24,
                        y: dock_band.y - 24,
                        width: dock_band.width + 48,
                        height: dock_band.height + 24,
                    }
                    .contains_point(cx, cy)
                });
            let recent_trigger = self.monitor_shells[mon_idx]
                .dock_last_trigger
                .is_some_and(|t| t.elapsed() < Duration::from_millis(500));

            let pointer_active = cursor_in_trigger
                || cursor_near_shown_dock
                || recent_trigger
                || self.monitor_shells[mon_idx].hovered_dock_idx.is_some()
                || (self.dock_popover_id.is_some() && self.dock_popover_monitor_idx == mon_idx);

            let want_hide = match vis_mode {
                DockVisibility::AlwaysVisible => false,
                DockVisibility::AutoHide => !pointer_active,
                DockVisibility::IntelliHide => {
                    let window_overlaps = self.running_windows.iter().any(|w| {
                        !w.is_minimized
                            && w.monitor_idx == mon_idx
                            && (w.is_maximized || w.frame_rect.intersects(dock_band))
                    });
                    window_overlaps && !pointer_active
                }
            };

            let reduce = self.config.accessibility.reduce_motion;
            let tokens = self.tokens;
            let shell = &mut self.monitor_shells[mon_idx];
            if want_hide != shell.dock_hidden {
                shell.dock_hidden = want_hide;
                if reduce {
                    if want_hide {
                        shell.dock_reveal.snap(0.0);
                        if shell.dock_hwnd_shown {
                            set_window_visible(dock_hwnd, false);
                            shell.dock_hwnd_shown = false;
                        }
                    } else {
                        if !shell.dock_hwnd_shown {
                            set_window_visible(dock_hwnd, true);
                            shell.dock_hwnd_shown = true;
                        }
                        shell.dock_reveal.snap(1.0);
                    }
                    shell.dock_tokens = tokens.with_opacity(shell.dock_reveal.value);
                } else {
                    if !want_hide && !shell.dock_hwnd_shown {
                        set_window_visible(dock_hwnd, true);
                        shell.dock_hwnd_shown = true;
                    }
                    shell.dock_reveal.set_target(if want_hide { 0.0 } else { 1.0 });
                    self.arm_animation();
                }
                if want_hide && let Some(s) = self.monitor_shells.get_mut(mon_idx) {
                    s.hovered_dock_idx = None;
                    s.dock_tooltip_opacity.set_target(0.0);
                    if reduce {
                        s.dock_tooltip_opacity.step(0.0, true);
                        s.dock_last_tooltip_idx = None;
                    }
                }
            }
        }
    }

    fn compute_maximized_states(&self) -> (Vec<bool>, Vec<Option<isize>>) {
        let count = self.monitor_shells.len();
        if (self.screenshot_dir.is_some() && !self.open_global_menu_on_boot)
            || !self.config.decorator.enabled
            || !self.config.decorator.hide_maximized_titlebar
        {
            return (vec![false; count], vec![None; count]);
        }
        let states = resolve_monitor_maximized_states(
            count,
            self.decorated_window.as_ref(),
            &self.running_windows,
        );
        let tuck_flags = if DEBUG_FORCE_TUCK.load(std::sync::atomic::Ordering::Relaxed) {
            vec![true; count]
        } else {
            states.iter().map(|s| s.should_tuck).collect()
        };
        let frontmost_hwnds = states.into_iter().map(|s| s.frontmost_hwnd).collect();
        (tuck_flags, frontmost_hwnds)
    }

    fn spawn_maximized_bg_sample_task(
        &self,
        targets: Vec<(usize, isize, MonitorInfo)>,
        delay_ms: u64,
        allow_fallback: bool,
    ) -> Task<Message> {
        if targets.is_empty() {
            return Task::none();
        }
        let top_bar_h = self.config.top_bar.height;
        Task::perform(
            async move {
                if delay_ms > 0 {
                    tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                }
                let samples = tokio::task::spawn_blocking(move || {
                    targets
                        .into_iter()
                        .map(|(mon_idx, hwnd, mon)| {
                            (mon_idx, hwnd, sample_maximized_window_bg(hwnd, &mon, top_bar_h))
                        })
                        .collect()
                })
                .await
                .unwrap_or_default();
                Message::MaximizedWindowColorsSampled {
                    samples,
                    allow_fallback,
                }
            },
            std::convert::identity,
        )
    }

    /// Recomputes per-monitor title-bar tuck state, applies work-area changes, and schedules
    /// smooth backdrop fade/color-matching transitions for Top Bar windows.
    fn sync_workarea_tuck(&mut self) -> Task<Message> {
        let prev_tucked = self.tucked_monitors.clone();
        let (flags, max_hwnds) = self.compute_maximized_states();
        let height = self.config.decorator.maximized_titlebar_height.clamp(24, 44);
        if flags != self.tucked_monitors || height != self.tuck_height_applied {
            self.tucked_monitors = flags.clone();
            self.tuck_height_applied = height;
            let monitors = self.monitor_infos();
            let bottom_reserve = bottom_reserve_for_config(&self.config);
            if let Ok(mut wg) = self.work_area_guard.lock() {
                wg.set_tucked_monitors(flags.clone(), height);
                wg.apply(
                    &monitors,
                    self.config.top_bar.height,
                    bottom_reserve,
                    self.config.hide_native_taskbar,
                );
            }
        }

        let instant = self.config.accessibility.reduce_motion || self.screenshot_dir.is_some();
        let debug_force = DEBUG_FORCE_TUCK.load(std::sync::atomic::Ordering::Relaxed);
        let islands_active = self.config.top_bar.style == TopBarStyle::FloatingPills
            && self.config.decorator.enabled
            && self.config.decorator.hide_maximized_titlebar;
        let fallback_bg = self.tokens_target.surface_base;
        let mut fast_targets = Vec::new();
        let mut settle_targets = Vec::new();
        let mut confirm_targets = Vec::new();
        let mut should_arm = false;

        for (mon_idx, shell) in self.monitor_shells.iter_mut().enumerate() {
            let tucked_now = flags.get(mon_idx).copied().unwrap_or(false);
            let pantheon_target = if tucked_now { 1.0 } else { 0.0 };
            if instant {
                shell.pantheon_solid.snap(pantheon_target);
            } else {
                shell.pantheon_solid.set_target(pantheon_target);
                if !shell.pantheon_solid.is_settled() {
                    should_arm = true;
                }
            }

            let maybe_hwnd = if islands_active {
                max_hwnds.get(mon_idx).copied().flatten()
            } else {
                None
            };
            if let Some(hwnd) = maybe_hwnd {
                let prev_hwnd = shell.maximized_bg_hwnd;
                let hwnd_changed = prev_hwnd != Some(hwnd);
                let just_tucked =
                    !prev_tucked.get(mon_idx).copied().unwrap_or(false) && tucked_now;
                shell.maximized_bg_hwnd = Some(hwnd);

                if self.screenshot_dir.is_some() {
                    let c = self
                        .decorated_window
                        .as_ref()
                        .filter(|d| d.hwnd == hwnd)
                        .and_then(|d| d.titlebar_bg)
                        .unwrap_or(fallback_bg);
                    shell.backdrop_transition.set_active_color(c, true);
                } else if let Some(&cached) = self.window_bg_cache.get(&hwnd) {
                    if hwnd_changed || shell.backdrop_transition.opacity() <= 0.03 {
                        shell.backdrop_transition.set_active_color(cached, instant);
                    }
                    if hwnd_changed || just_tucked {
                        if prev_hwnd.is_some() && !just_tucked {
                            fast_targets.push((mon_idx, hwnd, shell.info.clone()));
                        } else {
                            settle_targets.push((mon_idx, hwnd, shell.info.clone()));
                        }
                    }
                } else if hwnd_changed {
                    if prev_hwnd.is_some() {
                        fast_targets.push((mon_idx, hwnd, shell.info.clone()));
                    } else {
                        settle_targets.push((mon_idx, hwnd, shell.info.clone()));
                        confirm_targets.push((mon_idx, hwnd, shell.info.clone()));
                    }
                } else if just_tucked {
                    settle_targets.push((mon_idx, hwnd, shell.info.clone()));
                }
            } else {
                shell.maximized_bg_hwnd = None;
                if islands_active && debug_force && tucked_now {
                    shell.backdrop_transition.set_active_color(fallback_bg, instant);
                } else {
                    shell.backdrop_transition.set_inactive(instant);
                }
            }

            if !instant && shell.backdrop_transition.is_active() {
                should_arm = true;
            }
        }

        if should_arm {
            self.arm_animation();
        }

        Task::batch([
            self.spawn_maximized_bg_sample_task(fast_targets, 60, true),
            self.spawn_maximized_bg_sample_task(settle_targets, 180, false),
            self.spawn_maximized_bg_sample_task(confirm_targets, 360, true),
        ])
    }

    fn apply_dwm_theming(&mut self) {
        if !self.config.decorator.enabled || !self.config.decorator.dwm_theme_sync {
            self.dwm_themer.restore_all();
            return;
        }
        let Some(st) = &self.decorated_window else {
            return;
        };
        if st.hwnd == 0 || st.is_fullscreen {
            return;
        }
        let border_c = rgb_to_colorref(
            self.tokens.accent_primary.r,
            self.tokens.accent_primary.g,
            self.tokens.accent_primary.b,
        );
        let caption_c = rgb_to_colorref(
            self.tokens.surface_elevated.r,
            self.tokens.surface_elevated.g,
            self.tokens.surface_elevated.b,
        );
        let text_c = if self.config.decorator.custom_overlay {
            caption_c
        } else {
            rgb_to_colorref(
                self.tokens.text_primary.r,
                self.tokens.text_primary.g,
                self.tokens.text_primary.b,
            )
        };
        self.dwm_themer
            .apply(st.hwnd, self.tokens.is_dark, border_c, caption_c, text_c);
    }

    fn sync_decorator_window(&mut self) -> Task<Message> {
        let need_window = self.config.decorator.enabled
            && self.config.decorator.custom_overlay
            && !self.has_active_shell_popup()
            && self.decorated_window.as_ref().is_some_and(|st| {
                !st.is_fullscreen
                    && !(st.is_maximized && self.config.decorator.maximized_topbar_controls)
                    && st.frame_rect.width >= 180
            });

        if !need_window {
            if let Some(id) = self.decorator_id
                && let Some(&hwnd_raw) = self.hwnds.get(&id)
            {
                set_window_visible(hwnd_raw, false);
            }
            return Task::none();
        }

        if self.decorator_id.is_some() {
            self.sync_decorator_window_visibility_and_bounds();
            return Task::none();
        }

        let Some(st) = &self.decorated_window else {
            return Task::none();
        };
        let rect = decorator_overlay_rect(st);
        let logical = rect.to_logical(st.dpi_scale);
        let (id, open_task) = window::open(window::Settings {
            size: Size::new(logical.width.max(100.0), logical.height.max(26.0)),
            position: window::Position::Specific(Point::new(logical.x, logical.y)),
            decorations: false,
            transparent: true,
            resizable: false,
            level: window::Level::AlwaysOnTop,
            exit_on_close_request: false,
            ..window::Settings::default()
        });
        self.decorator_id = Some(id);
        self.windows.insert(id, SurfaceKind::WindowDecorator);
        open_task.map(Message::WindowOpened)
    }

    fn sync_decorator_window_visibility_and_bounds(&self) {
        let Some(id) = self.decorator_id else {
            return;
        };
        let Some(&hwnd_raw) = self.hwnds.get(&id) else {
            return;
        };
        let should_show = self.config.decorator.enabled
            && self.config.decorator.custom_overlay
            && !self.has_active_shell_popup()
            && self.decorated_window.as_ref().is_some_and(|st| {
                !st.is_fullscreen
                    && !(st.is_maximized && self.config.decorator.maximized_topbar_controls)
                    && st.frame_rect.width >= 180
            });

        if !should_show {
            set_window_visible(hwnd_raw, false);
            return;
        }
        if let Some(st) = &self.decorated_window {
            let frosted = matches!(
                self.config.surface_material,
                vita_core::SurfaceMaterial::Frosted
            );
            configure_shell_window(
                hwnd_raw,
                ShellWindowRole::Decorator,
                self.tokens.is_dark,
                frosted,
            );
            let rect = decorator_overlay_rect(st);
            reposition_window_physical(hwnd_raw, rect, true);
            set_window_visible(hwnd_raw, true);
        }
    }

    fn setup_screenshot_preview_surfaces(&mut self) -> Task<Message> {
        let mon_idx = self.primary_monitor_idx;
        let mon = self.monitor_shells[mon_idx].info.clone();
        let mut tasks = Vec::new();

        if self.open_global_menu_on_boot {
            self.active_app_menu = Some(ActiveAppMenu {
                hwnd: 9999,
                app_name: "VS Code".to_string(),
                exe_path: String::new(),
                monitor_idx: mon_idx,
                categories: vec![
                    GlobalMenuCategory {
                        label: "VS Code".into(),
                        items: Vec::new(),
                        lazy_uia_index: None,
                        loaded: true,
                    },
                    GlobalMenuCategory {
                        label: "File".into(),
                        items: vec![
                            sample_menu_item("New Text File", Some("Ctrl+N"), false, true),
                            sample_menu_item("Open File...", Some("Ctrl+O"), false, true),
                            sample_menu_item("Open Folder...", Some("Ctrl+K"), false, true),
                            sample_separator(),
                            sample_menu_item("Save", Some("Ctrl+S"), false, true),
                            sample_menu_item("Save As...", Some("Ctrl+Shift+S"), false, true),
                            sample_separator(),
                            sample_menu_item("Auto Save", None, true, true),
                            sample_separator(),
                            sample_menu_item("Close Editor", Some("Ctrl+W"), false, true),
                        ],
                        lazy_uia_index: None,
                        loaded: true,
                    },
                    empty_cat("Edit"),
                    empty_cat("Selection"),
                    empty_cat("View"),
                    empty_cat("Go"),
                ],
            });
            self.decorated_window = Some(DecoratedWindowState {
                hwnd: 9999,
                title: "main.rs — VitaDesktop — VS Code".to_string(),
                exe_name: "Code.exe".to_string(),
                frame_rect: mon.physical_bounds,
                dpi_scale: mon.dpi_scale,
                is_maximized: true,
                is_fullscreen: false,
                mode: DecoratorMode::ControlsOnly,
                monitor_idx: mon_idx,
                titlebar_bg: Some(RgbaColor::from_hex("#1F2330")),
                icon: None,
            });
            tasks.push(self.sync_workarea_tuck());
            tasks.push(self.open_global_menu_dropdown(mon_idx, 1));
        }

        if self.open_decorator_preview_on_boot {
            let b = mon.physical_bounds;
            self.decorated_window = Some(DecoratedWindowState {
                hwnd: 0,
                title: "Notes — Horizon Workspace".to_string(),
                exe_name: "notepad.exe".to_string(),
                frame_rect: PhysicalRect {
                    x: b.x + (b.width - 720) / 2,
                    y: b.y + 180,
                    width: 720,
                    height: 440,
                },
                dpi_scale: mon.dpi_scale,
                is_maximized: false,
                is_fullscreen: false,
                mode: DecoratorMode::FullHeader,
                monitor_idx: mon_idx,
                titlebar_bg: None,
                icon: None,
            });
            tasks.push(self.sync_decorator_window());
        }

        if self.open_calendar_on_boot && self.calendar_id.is_none() {
            tasks.push(self.open_calendar_on(mon_idx));
        }

        if self.open_tray_on_boot && self.tray_popover_id.is_none() {
            if self.tray_icons.is_empty() {
                self.populate_demo_tray_icons();
            }
            tasks.push(self.open_tray_popover_on(mon_idx));
        }

        if self.open_dock_thumbnails_on_boot && self.dock_popover_id.is_none() {
            let target_idx = self
                .dock_items
                .iter()
                .position(|i| i.windows.len() >= 2)
                .unwrap_or_else(|| self.populate_demo_dock_thumbnails(&mon));
            tasks.push(self.open_dock_popover_on(
                mon_idx,
                target_idx,
                dock_menu::DockPopoverMode::WindowPicker,
            ));
        }

        Task::batch(tasks)
    }

    fn populate_demo_tray_icons(&mut self) {
        let demos = [
            (
                "demo:onedrive",
                "OneDrive — Personal · Up to date",
                "OneDrive.exe",
                false,
                [46, 134, 222],
            ),
            (
                "demo:security",
                "Windows Security · No actions needed",
                "SecurityHealthSystray.exe",
                false,
                [46, 174, 118],
            ),
            (
                "demo:docker",
                "Docker Desktop · Engine running",
                "Docker Desktop.exe",
                true,
                [36, 150, 237],
            ),
            (
                "demo:tailscale",
                "Tailscale · Connected",
                "tailscale-ipn.exe",
                true,
                [88, 96, 114],
            ),
            (
                "demo:bluetooth",
                "Bluetooth Devices",
                "explorer.exe",
                true,
                [66, 108, 224],
            ),
        ];

        let mut list = Vec::with_capacity(demos.len());
        self.tray_icon_sources.clear();
        self.tray_icon_handles.clear();
        for (id, tooltip, exe_name, is_overflow, rgb) in demos {
            let icon = make_demo_badge_icon(24, rgb);
            self.tray_icon_sources.insert(id.to_string(), icon.clone());
            self.tray_icon_handles.insert(
                id.to_string(),
                image::Handle::from_rgba(icon.width, icon.height, icon.pixels.to_vec()),
            );
            list.push(TrayIconInfo {
                id: id.to_string(),
                tooltip: tooltip.to_string(),
                exe_name: exe_name.to_string(),
                owner_hwnd: 0,
                uid: 0,
                callback_msg: 0,
                uia_automation_id: None,
                is_overflow,
                icon: Some(icon),
            });
        }
        self.tray_icons = list;
    }

    fn populate_demo_dock_thumbnails(&mut self, mon: &MonitorInfo) -> usize {
        let (exe_name, exe_path) = self
            .config
            .dock
            .pinned_apps
            .iter()
            .find(|p| p.exe_name.eq_ignore_ascii_case("WindowsTerminal.exe"))
            .or_else(|| self.config.dock.pinned_apps.first())
            .map_or_else(
                || ("WindowsTerminal.exe".to_string(), String::new()),
                |p| (p.exe_name.clone(), p.target.clone()),
            );

        let demo_windows = vec![
            RunningWindow {
                hwnd: -101,
                title: "PowerShell — VitaDesktop".to_string(),
                exe_name: exe_name.clone(),
                exe_path: exe_path.clone(),
                pid: 4101,
                is_focused: true,
                is_minimized: false,
                is_maximized: false,
                frame_rect: mon.physical_bounds,
                monitor_idx: self.primary_monitor_idx,
                icon: None,
            },
            RunningWindow {
                hwnd: -102,
                title: "cargo test --workspace".to_string(),
                exe_name: exe_name.clone(),
                exe_path: exe_path.clone(),
                pid: 4101,
                is_focused: false,
                is_minimized: false,
                is_maximized: false,
                frame_rect: mon.physical_bounds,
                monitor_idx: self.primary_monitor_idx,
                icon: None,
            },
            RunningWindow {
                hwnd: -103,
                title: "Release Notes — Preview".to_string(),
                exe_name: exe_name.clone(),
                exe_path,
                pid: 4101,
                is_focused: false,
                is_minimized: true,
                is_maximized: false,
                frame_rect: mon.physical_bounds,
                monitor_idx: self.primary_monitor_idx,
                icon: None,
            },
        ];

        self.dock_fallback_thumbs.insert(
            -101,
            make_demo_window_preview([30, 34, 52], [198, 120, 86]),
        );
        self.dock_fallback_thumbs.insert(
            -102,
            make_demo_window_preview([24, 38, 48], [86, 178, 148]),
        );

        self.running_windows = demo_windows;
        let _ = self.rebuild_dock_items();
        self.dock_items
            .iter()
            .position(|i| i.exe_name.eq_ignore_ascii_case(&exe_name))
            .unwrap_or(0)
    }

    fn shutdown_shell_cleanup(&mut self) {
        self.dwm_thumbnails.clear();
        self.dwm_themer.restore_all();
        if let Ok(mut wg) = self.work_area_guard.lock() {
            wg.restore_now();
        }
        if let Ok(mut guard) = self.taskbar_guard.lock() {
            guard.set_hidden(false);
        }
        restore_all_taskbars();
    }

    fn rebuild_dock_items(&mut self) -> Task<Message> {
        self.dock_items = merge_dock_items(
            &self.config.dock.pinned_apps,
            &self.running_windows,
            &self.pinned_icons,
        );
        sync_rgba_icon_handles(
            &mut self.icon_sources,
            &mut self.icon_handles,
            self.dock_items
                .iter()
                .map(|item| (item.id.as_str(), item.icon.as_ref())),
        );

        let (new_w, slot_centers) = dock::compute_dock_layout(&self.config, self.dock_items.len());
        let new_h = dock::window_height(&self.config);
        let resized = (new_w - self.dock_window_width).abs() > 1.0
            || (new_h - self.dock_window_height).abs() > 1.0;
        self.dock_window_width = new_w;
        self.dock_window_height = new_h;
        for shell in &mut self.monitor_shells {
            shell.dock_magnifier.sync_slots(slot_centers.clone());
        }
        if resized {
            let dock_ids: Vec<window::Id> =
                self.monitor_shells.iter().filter_map(|s| s.dock_id).collect();
            let mut tasks = Vec::with_capacity(dock_ids.len());
            for did in dock_ids {
                self.apply_surface_chrome_and_bounds(did);
                tasks.push(window::resize(did, Size::new(new_w, new_h)));
            }
            return Task::batch(tasks);
        }
        Task::none()
    }

    fn apply_surface_chrome_and_bounds(&self, id: window::Id) {
        let Some(&hwnd_raw) = self.hwnds.get(&id) else {
            return;
        };
        let Some(&kind) = self.windows.get(&id) else {
            return;
        };

        let frosted = matches!(
            self.config.surface_material,
            vita_core::SurfaceMaterial::Frosted
        );
        let full_width = self.config.top_bar.style == TopBarStyle::FullWidth;

        match kind {
            SurfaceKind::TopBar => {
                let mon_idx = self
                    .top_bar_to_mon
                    .get(&id)
                    .copied()
                    .unwrap_or(self.primary_monitor_idx);
                let mon = &self.monitor_shells[mon_idx].info;
                configure_shell_window(
                    hwnd_raw,
                    ShellWindowRole::TopBar { full_width },
                    self.tokens.is_dark,
                    frosted,
                );
                let h_px = (self.config.top_bar.height as f32 * mon.dpi_scale).round() as i32;
                reposition_window_physical(
                    hwnd_raw,
                    PhysicalRect {
                        x: mon.physical_bounds.x,
                        y: mon.physical_bounds.y,
                        width: mon.physical_bounds.width,
                        height: h_px,
                    },
                    true,
                );
            }
            SurfaceKind::Dock => {
                let mon_idx = self
                    .dock_to_mon
                    .get(&id)
                    .copied()
                    .unwrap_or(self.primary_monitor_idx);
                let mon = &self.monitor_shells[mon_idx].info;
                let dpi = mon.dpi_scale;
                let bounds = mon.physical_bounds;
                configure_shell_window(
                    hwnd_raw,
                    ShellWindowRole::Dock,
                    self.tokens.is_dark,
                    frosted,
                );
                let w_px = (self.dock_window_width * dpi).round() as i32;
                let h_px = (dock::window_height(&self.config) * dpi).round() as i32;
                let x_px = bounds.x + (bounds.width - w_px) / 2;
                let y_px = bounds.y + bounds.height - h_px;
                reposition_window_physical(
                    hwnd_raw,
                    PhysicalRect {
                        x: x_px,
                        y: y_px,
                        width: w_px,
                        height: h_px,
                    },
                    true,
                );
            }
            SurfaceKind::Launcher => {
                let mon = self.active_monitor();
                let dpi = mon.dpi_scale;
                let bounds = mon.physical_bounds;
                configure_shell_window(
                    hwnd_raw,
                    ShellWindowRole::Popover,
                    self.tokens.is_dark,
                    frosted,
                );
                let w_px = (launcher::LAUNCHER_WINDOW_WIDTH * dpi).round() as i32;
                let h_px = (launcher::LAUNCHER_WINDOW_HEIGHT * dpi).round() as i32;
                let x_px = bounds.x + (8.0 * dpi).round() as i32;
                let y_px =
                    bounds.y + ((self.config.top_bar.height as f32 + 4.0) * dpi).round() as i32;
                reposition_window_physical(
                    hwnd_raw,
                    PhysicalRect {
                        x: x_px,
                        y: y_px,
                        width: w_px,
                        height: h_px,
                    },
                    true,
                );
            }
            SurfaceKind::DockPopover => {
                let mon = self
                    .monitor_shells
                    .get(self.dock_popover_monitor_idx)
                    .map(|s| &s.info)
                    .unwrap_or(&self.monitor_shells[0].info);
                let dpi = mon.dpi_scale;
                let role = if self.dock_popover_opened_by_hover {
                    ShellWindowRole::HoverPopover
                } else {
                    ShellWindowRole::Popover
                };
                configure_shell_window(
                    hwnd_raw,
                    role,
                    self.tokens.is_dark,
                    frosted,
                );
                if let Some(target) = &self.dock_popover_target
                    && let Some(item) = self
                        .dock_items
                        .iter()
                        .find(|i| i.id == target.dock_item_id)
                {
                    let (_, phys_rect) = self.compute_dock_popover_geometry(mon, item, target);
                    let _ = dpi;
                    reposition_window_physical(hwnd_raw, phys_rect, true);
                }
            }
            SurfaceKind::QuickControls => {
                let mon = self.active_monitor();
                let dpi = mon.dpi_scale;
                let bounds = mon.physical_bounds;
                configure_shell_window(
                    hwnd_raw,
                    ShellWindowRole::Popover,
                    self.tokens.is_dark,
                    frosted,
                );
                let w = quick_controls::QUICK_CONTROLS_WIDTH;
                let h = quick_controls::estimate_height(&self.summary, self.power_confirm_open);
                let w_px = (w * dpi).round() as i32;
                let h_px = (h * dpi).round() as i32;
                let x_px = bounds.x + bounds.width - w_px - (8.0 * dpi).round() as i32;
                let y_px =
                    bounds.y + ((self.config.top_bar.height as f32 + 4.0) * dpi).round() as i32;
                reposition_window_physical(
                    hwnd_raw,
                    PhysicalRect {
                        x: x_px,
                        y: y_px,
                        width: w_px,
                        height: h_px,
                    },
                    true,
                );
            }
            SurfaceKind::Spaces => {
                let mon = self.active_monitor();
                let dpi = mon.dpi_scale;
                let bounds = mon.physical_bounds;
                configure_shell_window(
                    hwnd_raw,
                    ShellWindowRole::Popover,
                    self.tokens.is_dark,
                    frosted,
                );
                let w_px = (spaces::SPACES_WINDOW_WIDTH * dpi).round() as i32;
                let h_px = (spaces::SPACES_WINDOW_HEIGHT * dpi).round() as i32;
                let x_px = bounds.x + (bounds.width - w_px) / 2;
                let y_px = bounds.y + ((bounds.height - h_px) as f32 * 0.45).round() as i32;
                reposition_window_physical(
                    hwnd_raw,
                    PhysicalRect {
                        x: x_px,
                        y: y_px,
                        width: w_px,
                        height: h_px,
                    },
                    true,
                );
            }
            SurfaceKind::CalmZone => {
                let mon = &self.primary_shell().info;
                let dpi = mon.dpi_scale;
                let bounds = mon.physical_bounds;
                configure_shell_window(
                    hwnd_raw,
                    ShellWindowRole::DesktopWidget,
                    self.tokens.is_dark,
                    frosted,
                );
                let w_px = (calm_zone::CALM_ZONE_WIDTH * dpi).round() as i32;
                let h_px = (calm_zone::CALM_ZONE_HEIGHT * dpi).round() as i32;
                let x_px = bounds.x + (28.0 * dpi).round() as i32;
                let y_px =
                    bounds.y + ((self.config.top_bar.height as f32 + 22.0) * dpi).round() as i32;
                position_window_bottom(
                    hwnd_raw,
                    PhysicalRect {
                        x: x_px,
                        y: y_px,
                        width: w_px,
                        height: h_px,
                    },
                );
            }
            SurfaceKind::Notifications => {
                let mon = self.active_monitor();
                let dpi = mon.dpi_scale;
                let bounds = mon.physical_bounds;
                configure_shell_window(
                    hwnd_raw,
                    ShellWindowRole::Popover,
                    self.tokens.is_dark,
                    frosted,
                );
                let w_px = (notifications::NOTIF_CENTER_WIDTH * dpi).round() as i32;
                let h_px = (notifications::NOTIF_CENTER_HEIGHT * dpi).round() as i32;
                let x_px = bounds.x + bounds.width - w_px - (8.0 * dpi).round() as i32;
                let y_px =
                    bounds.y + ((self.config.top_bar.height as f32 + 4.0) * dpi).round() as i32;
                reposition_window_physical(
                    hwnd_raw,
                    PhysicalRect {
                        x: x_px,
                        y: y_px,
                        width: w_px,
                        height: h_px,
                    },
                    true,
                );
            }
            SurfaceKind::TrayPopover => {
                let mon = self.active_monitor();
                let dpi = mon.dpi_scale;
                let logical = mon.logical_bounds();
                configure_shell_window(
                    hwnd_raw,
                    ShellWindowRole::Popover,
                    self.tokens.is_dark,
                    frosted,
                );
                let w = tray::TRAY_POPOVER_WIDTH;
                let h = tray::estimate_height(self.tray_icons.len());
                let x = self.tray_popover_logical_x(&logical, w);
                let y = logical.y + self.config.top_bar.height as f32 + 4.0;
                let phys = vita_core::LogicalRect {
                    x,
                    y,
                    width: w,
                    height: h,
                }
                .to_physical(dpi);
                reposition_window_physical(hwnd_raw, phys, true);
            }
            SurfaceKind::Calendar => {
                let mon = self.active_monitor();
                let dpi = mon.dpi_scale;
                let logical = mon.logical_bounds();
                configure_shell_window(
                    hwnd_raw,
                    ShellWindowRole::Popover,
                    self.tokens.is_dark,
                    frosted,
                );
                let w = calendar::CALENDAR_WIDTH;
                let h = calendar::CALENDAR_HEIGHT;
                let x = self.calendar_logical_x(&logical, w);
                let y = logical.y + self.config.top_bar.height as f32 + 4.0;
                let phys = vita_core::LogicalRect {
                    x,
                    y,
                    width: w,
                    height: h,
                }
                .to_physical(dpi);
                reposition_window_physical(hwnd_raw, phys, true);
            }
            SurfaceKind::Toast => {
                let mon = &self.primary_shell().info;
                let dpi = mon.dpi_scale;
                let bounds = mon.physical_bounds;
                configure_shell_window(
                    hwnd_raw,
                    ShellWindowRole::Toast,
                    self.tokens.is_dark,
                    frosted,
                );
                let w_px = (notifications::TOAST_WINDOW_WIDTH * dpi).round() as i32;
                let h_px = (notifications::TOAST_WINDOW_HEIGHT * dpi).round() as i32;
                let x_px = bounds.x + bounds.width - w_px - (8.0 * dpi).round() as i32;
                let y_px =
                    bounds.y + ((self.config.top_bar.height as f32 + 6.0) * dpi).round() as i32;
                reposition_window_physical(
                    hwnd_raw,
                    PhysicalRect {
                        x: x_px,
                        y: y_px,
                        width: w_px,
                        height: h_px,
                    },
                    true,
                );
            }
            SurfaceKind::Settings => {
                let mon = self.active_monitor();
                let dpi = mon.dpi_scale;
                let bounds = mon.physical_bounds;
                configure_shell_window(
                    hwnd_raw,
                    ShellWindowRole::Settings,
                    self.tokens.is_dark,
                    frosted,
                );
                if !self.settings_positioned {
                    let w_px = (settings::SETTINGS_WINDOW_WIDTH * dpi).round() as i32;
                    let h_px = (settings::SETTINGS_WINDOW_HEIGHT * dpi).round() as i32;
                    let x_px = bounds.x + (bounds.width - w_px) / 2;
                    let y_px = bounds.y + ((bounds.height - h_px) as f32 * 0.46).round() as i32;
                    reposition_window_physical(
                        hwnd_raw,
                        PhysicalRect {
                            x: x_px,
                            y: y_px,
                            width: w_px,
                            height: h_px,
                        },
                        false,
                    );
                }
            }
            SurfaceKind::GlobalMenu => {
                let mon_idx = self
                    .open_global_menu
                    .map_or(self.active_monitor_idx, |(m, _)| m);
                let mon = self
                    .monitor_shells
                    .get(mon_idx)
                    .map(|s| &s.info)
                    .unwrap_or(&self.monitor_shells[0].info);
                let dpi = mon.dpi_scale;
                let bounds = mon.physical_bounds;
                configure_shell_window(
                    hwnd_raw,
                    ShellWindowRole::Popover,
                    self.tokens.is_dark,
                    frosted,
                );
                let menu_idx = self.open_global_menu.map_or(0, |(_, idx)| idx);
                let anchor_logical = global_menu_anchor_offset(self.config.top_bar.style, menu_idx);
                let w_px = (global_menu::GLOBAL_MENU_WIDTH * dpi).round() as i32;
                let h_px = (global_menu::estimate_height(self.selected_global_menu_category()) * dpi)
                    .round() as i32;
                let x_px = (bounds.x + (anchor_logical * dpi).round() as i32)
                    .clamp(bounds.x + (8.0 * dpi).round() as i32, bounds.right() - w_px - (8.0 * dpi).round() as i32);
                let y_px =
                    bounds.y + ((self.config.top_bar.height as f32 + 4.0) * dpi).round() as i32;
                reposition_window_physical(
                    hwnd_raw,
                    PhysicalRect {
                        x: x_px,
                        y: y_px,
                        width: w_px,
                        height: h_px,
                    },
                    true,
                );
            }
            SurfaceKind::WindowDecorator => {
                self.sync_decorator_window_visibility_and_bounds();
            }
        }
    }
}

fn bottom_reserve_for_config(config: &ShellConfig) -> u32 {
    if config.dock.reserve_work_area && config.dock.visibility == DockVisibility::AlwaysVisible {
        (dock::pill_height(config) + dock::bottom_margin(config)).round() as u32
    } else {
        0
    }
}

fn global_menu_anchor_offset(style: TopBarStyle, menu_idx: usize) -> f32 {
    let base = match style {
        TopBarStyle::FloatingPills => 92.0,
        TopBarStyle::FullWidth => 136.0,
    };
    base + (menu_idx as f32 * 52.0)
}

fn decorator_overlay_rect(st: &DecoratedWindowState) -> PhysicalRect {
    let dpi = st.dpi_scale.max(1.0);
    match st.mode {
        DecoratorMode::FullHeader => PhysicalRect {
            x: st.frame_rect.x,
            y: st.frame_rect.y,
            width: st.frame_rect.width.max(180),
            height: (window_decorator::FULL_HEADER_HEIGHT * dpi).round() as i32,
        },
        DecoratorMode::ControlsOnly => {
            let w_px = (window_decorator::CONTROLS_ONLY_WIDTH * dpi).round() as i32;
            let h_px = (window_decorator::CONTROLS_ONLY_HEIGHT * dpi).round() as i32;
            let border_inset = (1.0 * dpi).round() as i32;
            PhysicalRect {
                x: st.frame_rect.right() - w_px - border_inset,
                y: st.frame_rect.y + border_inset,
                width: w_px,
                height: h_px,
            }
        }
    }
}

fn map_pointer_to_unscaled_x(
    icon_size: u32,
    count: usize,
    magnifier: &DockMagnifier,
    local_x: f32,
) -> f32 {
    let base = icon_size as f32;
    let slot_w = base + 8.0;
    if count == 0 {
        return local_x;
    }
    let unscaled_total = count as f32 * slot_w + (count - 1) as f32 * 6.0;
    let scaled_icons: f32 = (0..count)
        .map(|i| base * magnifier.scale_at(i) + 8.0)
        .sum();
    let current_total = scaled_icons + (count - 1) as f32 * 6.0;
    if current_total <= 1.0 {
        local_x
    } else {
        (local_x / current_total) * unscaled_total
    }
}

fn closest_slot(icon_size: u32, count: usize, unscaled_x: f32) -> Option<usize> {
    let slot_w = icon_size as f32 + 8.0;
    let step = slot_w + 6.0;
    let mut best: Option<(usize, f32)> = None;
    for i in 0..count {
        let center = (i as f32) * step + slot_w * 0.5;
        let dist = (unscaled_x - center).abs();
        if dist <= slot_w * 0.65 && best.is_none_or(|(_, d)| dist < d) {
            best = Some((i, dist));
        }
    }
    best.map(|(i, _)| i)
}

fn apply_hardware_cmd(cmd: HardwareCmd) {
    match cmd {
        HardwareCmd::Volume(pct) => {
            let _ = set_master_volume(pct);
        }
        HardwareCmd::Mute(muted) => {
            let _ = set_master_mute(muted);
        }
        HardwareCmd::Brightness(pct) => {
            let _ = set_display_brightness(pct);
        }
    }
}

fn launch_app_or_target(target: &str) {
    if target.contains('!') || target.starts_with('{') || target.starts_with("shell:") {
        launch_app_aumid(target);
    } else {
        launch_target(target);
    }
}

fn infer_exe_name_from_aumid(aumid: &str) -> String {
    if let Some(last) = aumid.rsplit(['\\', '/']).next()
        && last.to_ascii_lowercase().ends_with(".exe")
    {
        return last.to_string();
    }
    String::new()
}

fn resolve_tokens(config: &ShellConfig, system_dark: bool) -> ThemeTokens {
    ThemeTokens::resolve(
        config.theme_mode,
        system_dark,
        config.accent,
        config.surface_material,
        config.accessibility.increase_contrast,
    )
}

fn sample_menu_item(
    label: &str,
    shortcut: Option<&str>,
    checked: bool,
    enabled: bool,
) -> GlobalMenuItem {
    GlobalMenuItem {
        label: label.to_string(),
        shortcut: shortcut.map(ToString::to_string),
        enabled,
        checked,
        is_separator: false,
        action: None,
    }
}

fn sample_separator() -> GlobalMenuItem {
    GlobalMenuItem {
        label: String::new(),
        shortcut: None,
        enabled: false,
        checked: false,
        is_separator: true,
        action: None,
    }
}

fn cursor_dock_masks(monitors: &[MonitorInfo]) -> (u64, u64) {
    let Some((cx, cy)) = cursor_position() else {
        return (0, 0);
    };
    let mut near = 0u64;
    let mut trig = 0u64;
    for (idx, mon) in monitors.iter().enumerate().take(64) {
        let b = mon.physical_bounds;
        if cx >= b.x && cx < b.right() {
            let near_top = b.bottom() - (140.0 * mon.dpi_scale).round() as i32;
            if cy >= near_top && cy < b.bottom() {
                near |= 1u64 << idx;
            }
            if cy >= b.bottom() - 6 && cy < b.bottom() {
                trig |= 1u64 << idx;
            }
        }
    }
    (near, trig)
}

fn empty_cat(label: &str) -> GlobalMenuCategory {
    GlobalMenuCategory {
        label: label.to_string(),
        items: Vec::new(),
        lazy_uia_index: None,
        loaded: true,
    }
}

/// Retains existing GPU `image::Handle`s whenever an item's `RgbaIcon` pixels are unchanged,
/// preventing texture re-uploads and dock/tray flicker across frequent window updates.
fn sync_rgba_icon_handles<'a>(
    sources: &mut HashMap<String, RgbaIcon>,
    handles: &mut HashMap<String, image::Handle>,
    items: impl Iterator<Item = (&'a str, Option<&'a RgbaIcon>)>,
) {
    let mut keep = std::collections::HashSet::new();
    for (id, icon) in items {
        if let Some(ic) = icon {
            keep.insert(id.to_string());
            let unchanged =
                sources.get(id).is_some_and(|prev| prev == ic) && handles.contains_key(id);
            if !unchanged {
                sources.insert(id.to_string(), ic.clone());
                handles.insert(
                    id.to_string(),
                    image::Handle::from_rgba(ic.width, ic.height, ic.pixels.to_vec()),
                );
            }
        }
    }
    sources.retain(|k, _| keep.contains(k));
    handles.retain(|k, _| keep.contains(k));
}

fn dock_popover_bounds(
    config: &ShellConfig,
    mon: &MonitorInfo,
    anchor_center_x_logical: f32,
    pop_w: f32,
    pop_h: f32,
) -> (vita_core::LogicalRect, PhysicalRect) {
    let logical = mon.logical_bounds();
    let min_x = logical.x + 8.0;
    let max_x = (logical.x + logical.width - pop_w - 8.0).max(min_x);
    let x = (logical.x + anchor_center_x_logical - pop_w * 0.5).clamp(min_x, max_x);
    let min_y = logical.y + config.top_bar.height as f32 + 4.0;
    let y = (logical.y
        + logical.height
        - dock::pill_height(config)
        - dock::bottom_margin(config)
        - pop_h
        - 22.0)
        .max(min_y);
    let log_rect = vita_core::LogicalRect {
        x,
        y,
        width: pop_w,
        height: pop_h,
    };
    let phys_rect = log_rect.to_physical(mon.dpi_scale);
    (log_rect, phys_rect)
}

#[allow(clippy::too_many_arguments)]
fn reconcile_wpn_notifications(
    sys_list: &[SystemNotification],
    wpn_seeded: &mut bool,
    seen_wpn_ids: &mut HashSet<String>,
    dismissed_wpn_ids: &HashSet<String>,
    wpn_to_notif_id: &mut HashMap<String, u64>,
    notif_id_to_wpn: &mut HashMap<u64, String>,
    next_notif_id: &mut u64,
    notifications: &mut Vec<ToastNotification>,
) -> Option<ToastNotification> {
    let was_seeded = *wpn_seeded;
    *wpn_seeded = true;

    let mut newest_unseen: Option<ToastNotification> = None;
    let mut merged_sys = Vec::new();

    for sys in sys_list.iter().rev() {
        let is_new = seen_wpn_ids.insert(sys.wpn_id.clone());
        let id = *wpn_to_notif_id
            .entry(sys.wpn_id.clone())
            .or_insert_with(|| {
                let allocated = *next_notif_id;
                *next_notif_id += 1;
                allocated
            });
        notif_id_to_wpn.insert(id, sys.wpn_id.clone());

        if dismissed_wpn_ids.contains(&sys.wpn_id) {
            continue;
        }

        let toast = ToastNotification {
            id,
            app_name: sys.app_name.clone(),
            title: sys.title.clone(),
            body: sys.body.clone(),
            timestamp: sys.timestamp.clone(),
            level: sys.level,
        };
        if was_seeded && is_new {
            newest_unseen = Some(toast.clone());
        }
        merged_sys.push(toast);
    }

    let mut combined: Vec<ToastNotification> = notifications
        .iter()
        .filter(|n| !notif_id_to_wpn.contains_key(&n.id))
        .cloned()
        .collect();
    combined.extend(merged_sys);
    combined.sort_by_key(|n| n.id);
    if combined.len() > 40 {
        let excess = combined.len() - 40;
        combined.drain(0..excess);
    }
    *notifications = combined;
    newest_unseen
}

fn make_demo_badge_icon(size: u32, rgb: [u8; 3]) -> RgbaIcon {
    let mut pixels = vec![0u8; (size as usize) * (size as usize) * 4];
    let center = (size as f32 - 1.0) * 0.5;
    let radius = size as f32 * 0.44;
    for y in 0..size {
        for x in 0..size {
            let dx = x as f32 - center;
            let dy = y as f32 - center;
            let dist = (dx * dx + dy * dy).sqrt();
            let idx = ((y as usize) * (size as usize) + (x as usize)) * 4;
            if dist <= radius {
                let inner = dist <= radius * 0.42;
                pixels[idx] = if inner { 250 } else { rgb[0] };
                pixels[idx + 1] = if inner { 250 } else { rgb[1] };
                pixels[idx + 2] = if inner { 250 } else { rgb[2] };
                pixels[idx + 3] = 255;
            }
        }
    }
    RgbaIcon {
        width: size,
        height: size,
        pixels: std::sync::Arc::from(pixels),
    }
}

fn make_demo_window_preview(bg: [u8; 3], accent: [u8; 3]) -> image::Handle {
    let w = 188u32;
    let h = 112u32;
    let mut pixels = vec![0u8; (w as usize) * (h as usize) * 4];
    for y in 0..h {
        for x in 0..w {
            let idx = ((y as usize) * (w as usize) + (x as usize)) * 4;
            let is_header = y < 18;
            let is_line = (30..36).contains(&y) && (16..126).contains(&x)
                || (46..52).contains(&y) && (16..158).contains(&x)
                || (62..68).contains(&y) && (16..96).contains(&x)
                || (78..84).contains(&y) && (16..138).contains(&x);
            let rgb = if is_header {
                [
                    bg[0].saturating_add(14),
                    bg[1].saturating_add(14),
                    bg[2].saturating_add(18),
                ]
            } else if (30..36).contains(&y) && (16..62).contains(&x) {
                accent
            } else if is_line {
                [
                    bg[0].saturating_add(44),
                    bg[1].saturating_add(48),
                    bg[2].saturating_add(58),
                ]
            } else {
                bg
            };
            pixels[idx] = rgb[0];
            pixels[idx + 1] = rgb[1];
            pixels[idx + 2] = rgb[2];
            pixels[idx + 3] = 255;
        }
    }
    image::Handle::from_rgba(w, h, pixels)
}

fn shell_backend_worker() -> impl Stream<Item = Message> {
    stream::channel(64, |mut output: futures_mpsc::Sender<Message>| async move {
        let (win_tx, mut win_rx) = mpsc::unbounded_channel();
        let (hk_tx, mut hk_rx) = mpsc::unbounded_channel();
        let (cfg_tx, mut cfg_rx) = mpsc::unbounded_channel();
        let (deco_tx, mut deco_rx) = mpsc::unbounded_channel();
        let (tray_tx, mut tray_rx) = mpsc::unbounded_channel();
        let (notif_tx, mut notif_rx) = mpsc::unbounded_channel();

        let _tracker = WindowTracker::start(win_tx);
        let _deco_tracker = DecoratorTracker::start(deco_tx);
        let _hotkeys = HotkeyGuard::start(hk_tx);
        let _tray_tracker = TrayTracker::start(tray_tx);
        let _notif_watcher = WindowsNotificationWatcher::start(notif_tx);
        let _cfg_watcher = spawn_config_watcher(ShellConfig::config_path(), move |cfg| {
            let _ = cfg_tx.send(cfg);
        })
        .ok();

        let mut menu_rx = GLOBAL_MENU_RX
            .lock()
            .ok()
            .and_then(|mut opt| opt.take())
            .unwrap_or_else(|| mpsc::unbounded_channel().1);

        let mut last_monitors = enumerate_monitors();
        let mut last_near_mask: u64 = 0;
        let mut last_trigger_mask: u64 = 0;
        let mut trigger_grace_until: Option<Instant> = None;

        let mut cursor_interval = tokio::time::interval(Duration::from_millis(100));
        cursor_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        let mut telemetry_interval = tokio::time::interval(Duration::from_secs(2));
        telemetry_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                Some(wins) = win_rx.recv() => {
                    if output.send(Message::WindowsUpdated(wins)).await.is_err() {
                        break;
                    }
                }
                Some(ev) = hk_rx.recv() => {
                    if output.send(Message::Hotkey(ev)).await.is_err() {
                        break;
                    }
                }
                Some(cfg) = cfg_rx.recv() => {
                    if output.send(Message::ConfigReloaded(cfg)).await.is_err() {
                        break;
                    }
                }
                Some(deco) = deco_rx.recv() => {
                    if output.send(Message::DecoratedWindowUpdated(deco)).await.is_err() {
                        break;
                    }
                }
                Some(menu) = menu_rx.recv() => {
                    if output.send(Message::ActiveAppMenuUpdated(menu)).await.is_err() {
                        break;
                    }
                }
                Some(tray_icons) = tray_rx.recv() => {
                    if output.send(Message::TrayIconsUpdated(tray_icons)).await.is_err() {
                        break;
                    }
                }
                Some(sys_notifs) = notif_rx.recv() => {
                    if output.send(Message::SystemNotificationsUpdated(sys_notifs)).await.is_err() {
                        break;
                    }
                }
                _ = cursor_interval.tick() => {
                    let (near_mask, trig_mask) = cursor_dock_masks(&last_monitors);
                    let mut notify = near_mask != last_near_mask || trig_mask != last_trigger_mask;
                    if near_mask != 0 {
                        trigger_grace_until = Some(Instant::now() + Duration::from_millis(550));
                    } else if let Some(deadline) = trigger_grace_until
                        && Instant::now() >= deadline
                    {
                        trigger_grace_until = None;
                        notify = true;
                    }
                    last_near_mask = near_mask;
                    last_trigger_mask = trig_mask;
                    if notify && output.send(Message::DockSensorTick).await.is_err() {
                        break;
                    }
                }
                _ = telemetry_interval.tick() => {
                    let prev_monitors = last_monitors.clone();
                    if let Ok((summary, monitors, layout_changed)) = tokio::task::spawn_blocking(move || {
                        let summary = read_system_summary();
                        let current_mons = enumerate_monitors();
                        let same_topology = current_mons.len() == prev_monitors.len()
                            && current_mons.iter().zip(&prev_monitors).all(|(a, b)| {
                                a.physical_bounds == b.physical_bounds
                                    && (a.dpi_scale - b.dpi_scale).abs() < 0.01
                                    && a.is_primary == b.is_primary
                            });
                        if same_topology
                            && let Ok(slot) = WORK_AREA_SHARED.lock()
                            && let Some(guard) = &*slot
                            && let Ok(mut g) = guard.lock()
                        {
                            g.ensure_applied();
                        }
                        (summary, current_mons, !same_topology)
                    }).await {
                        if layout_changed {
                            last_monitors = monitors.clone();
                            if output.send(Message::MonitorsChanged(monitors)).await.is_err() {
                                break;
                            }
                        }
                        if output.send(Message::TelemetryUpdated(summary)).await.is_err() {
                            break;
                        }
                    }
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_decorated(
        hwnd: isize,
        exe_name: &str,
        monitor_idx: usize,
        is_maximized: bool,
        is_fullscreen: bool,
    ) -> DecoratedWindowState {
        DecoratedWindowState {
            hwnd,
            title: exe_name.to_string(),
            exe_name: exe_name.to_string(),
            frame_rect: PhysicalRect::default(),
            dpi_scale: 1.0,
            is_maximized,
            is_fullscreen,
            mode: DecoratorMode::ControlsOnly,
            monitor_idx,
            titlebar_bg: None,
            icon: None,
        }
    }

    fn fake_running(
        hwnd: isize,
        exe_name: &str,
        monitor_idx: usize,
        is_maximized: bool,
        is_minimized: bool,
        is_focused: bool,
    ) -> RunningWindow {
        RunningWindow {
            hwnd,
            title: exe_name.to_string(),
            exe_name: exe_name.to_string(),
            exe_path: String::new(),
            pid: 1,
            is_focused,
            is_minimized,
            is_maximized,
            frame_rect: PhysicalRect::default(),
            monitor_idx,
            icon: None,
        }
    }

    #[test]
    fn resolve_tuck_flags_handles_frontmost_zorder_and_stale_windows() {
        // 1. Maximized Antigravity in front of maximized Edge on monitor 0 -> [true]
        let antigravity = fake_decorated(101, "Antigravity.exe", 0, true, false);
        let running_antigravity_front = vec![
            fake_running(101, "Antigravity.exe", 0, true, false, true),
            fake_running(202, "msedge.exe", 0, true, false, false),
        ];
        assert_eq!(
            resolve_tuck_flags(1, Some(&antigravity), &running_antigravity_front),
            vec![true]
        );

        // 2. Maximized Edge in front of maximized Antigravity on monitor 0 -> [false]
        let edge = fake_decorated(202, "msedge.exe", 0, true, false);
        let running_edge_front = vec![
            fake_running(202, "msedge.exe", 0, true, false, true),
            fake_running(101, "Antigravity.exe", 0, true, false, false),
        ];
        assert_eq!(
            resolve_tuck_flags(1, Some(&edge), &running_edge_front),
            vec![false]
        );

        // 3. Focused Antigravity just maximized while running_windows is still stale -> [true]
        let running_stale_unmax = vec![
            fake_running(101, "Antigravity.exe", 0, false, false, true),
            fake_running(202, "msedge.exe", 0, true, false, false),
        ];
        assert_eq!(
            resolve_tuck_flags(1, Some(&antigravity), &running_stale_unmax),
            vec![true]
        );

        // 4. Focused Antigravity just restored while running_windows still says maximized -> [false]
        let antigravity_restored = fake_decorated(101, "Antigravity.exe", 0, false, false);
        let running_stale_max_only_antigravity =
            vec![fake_running(101, "Antigravity.exe", 0, true, false, true)];
        assert_eq!(
            resolve_tuck_flags(
                1,
                Some(&antigravity_restored),
                &running_stale_max_only_antigravity
            ),
            vec![false]
        );

        // 5. VitaDesktop popup focused (decorated = None) keeps frontmost Z-order choice -> [true]
        assert_eq!(
            resolve_tuck_flags(1, None, &running_antigravity_front),
            vec![true]
        );

        // 6. Multi-monitor: maximized Antigravity on monitor 0, maximized Edge on monitor 1 -> [true, false]
        let multi_running = vec![
            fake_running(101, "Antigravity.exe", 0, true, false, true),
            fake_running(202, "msedge.exe", 1, true, false, false),
        ];
        assert_eq!(
            resolve_tuck_flags(2, Some(&antigravity), &multi_running),
            vec![true, false]
        );

        // 7. Fullscreen Antigravity -> [false]
        let antigravity_fs = fake_decorated(101, "Antigravity.exe", 0, false, true);
        let running_fs = vec![fake_running(101, "Antigravity.exe", 0, true, false, true)];
        assert_eq!(
            resolve_tuck_flags(1, Some(&antigravity_fs), &running_fs),
            vec![false]
        );

        // 8. Maximized Antigravity moved from monitor 0 to monitor 1 with stale running_windows -> [false, true]
        let antigravity_mon1 = fake_decorated(101, "Antigravity.exe", 1, true, false);
        let running_stale_mon0 = vec![fake_running(101, "Antigravity.exe", 0, true, false, true)];
        assert_eq!(
            resolve_tuck_flags(2, Some(&antigravity_mon1), &running_stale_mon0),
            vec![false, true]
        );
    }

    #[test]
    fn resolve_monitor_maximized_states_matches_browsers_without_tucking() {
        let code = fake_decorated(101, "Code.exe", 0, true, false);
        let windows = vec![
            fake_running(101, "Code.exe", 0, true, false, true),
            fake_running(202, "explorer.exe", 1, true, false, false),
            fake_running(303, "msedge.exe", 2, true, false, false),
        ];
        assert_eq!(
            resolve_monitor_maximized_states(3, Some(&code), &windows),
            vec![
                MonitorMaximizedState {
                    frontmost_hwnd: Some(101),
                    should_tuck: true,
                },
                MonitorMaximizedState {
                    frontmost_hwnd: Some(202),
                    should_tuck: true,
                },
                MonitorMaximizedState {
                    frontmost_hwnd: Some(303),
                    should_tuck: false,
                },
            ]
        );

        let edge_front = fake_decorated(303, "msedge.exe", 0, true, false);
        let stacked = vec![
            fake_running(303, "msedge.exe", 0, true, false, true),
            fake_running(101, "Code.exe", 0, true, false, false),
        ];
        assert_eq!(
            resolve_monitor_maximized_states(1, Some(&edge_front), &stacked),
            vec![MonitorMaximizedState {
                frontmost_hwnd: Some(303),
                should_tuck: false,
            }]
        );
    }

    #[test]
    fn reconcile_wpn_seeds_without_toast_then_surfaces_new_and_respects_dismissed() {
        let mut seeded = false;
        let mut seen = HashSet::new();
        let mut dismissed = HashSet::new();
        let mut wpn_to_id = HashMap::new();
        let mut id_to_wpn = HashMap::new();
        let mut next_id = 10u64;
        let mut notifs = vec![ToastNotification {
            id: 1,
            app_name: "VitaDesktop".into(),
            title: "Shell ready".into(),
            body: "Warm welcome".into(),
            timestamp: "09:00".into(),
            level: NotificationLevel::Info,
        }];

        let make_sys = |wpn_id: &str, title: &str| SystemNotification {
            wpn_id: wpn_id.to_string(),
            app_name: "Slack".to_string(),
            title: title.to_string(),
            body: "Ping".to_string(),
            timestamp: "09:05".to_string(),
            level: NotificationLevel::Info,
        };

        // Initial snapshot is newest-first: [w2, w1]. Must NOT toast on startup.
        let initial = vec![make_sys("w2", "Second"), make_sys("w1", "First")];
        let toast0 = reconcile_wpn_notifications(
            &initial,
            &mut seeded,
            &mut seen,
            &dismissed,
            &mut wpn_to_id,
            &mut id_to_wpn,
            &mut next_id,
            &mut notifs,
        );
        assert!(toast0.is_none());
        assert!(seeded);
        assert_eq!(notifs.len(), 3);
        assert_eq!(notifs[0].title, "Shell ready");
        assert_eq!(notifs[1].title, "First");
        assert_eq!(notifs[2].title, "Second");

        // Dismiss "w1" and verify it stays gone when snapshots repeat.
        let w1_id = wpn_to_id["w1"];
        dismissed.insert("w1".to_string());
        notifs.retain(|n| n.id != w1_id);

        // New notification "w3" arrives alongside history [w3, w2, w1].
        let next = vec![
            make_sys("w3", "Third"),
            make_sys("w2", "Second"),
            make_sys("w1", "First"),
        ];
        let toast1 = reconcile_wpn_notifications(
            &next,
            &mut seeded,
            &mut seen,
            &dismissed,
            &mut wpn_to_id,
            &mut id_to_wpn,
            &mut next_id,
            &mut notifs,
        );
        assert_eq!(toast1.as_ref().map(|t| t.title.as_str()), Some("Third"));
        assert!(notifs.iter().all(|n| n.title != "First"));
        assert_eq!(notifs.last().map(|n| n.title.as_str()), Some("Third"));

        // Re-emitting the same snapshot does not trigger a duplicate toast.
        let toast2 = reconcile_wpn_notifications(
            &next,
            &mut seeded,
            &mut seen,
            &dismissed,
            &mut wpn_to_id,
            &mut id_to_wpn,
            &mut next_id,
            &mut notifs,
        );
        assert!(toast2.is_none());
    }

    #[test]
    fn dock_popover_bounds_clamp_inside_monitor_work_area() {
        let cfg = ShellConfig::default();
        let mon = MonitorInfo {
            name: "TEST".into(),
            is_primary: true,
            physical_bounds: PhysicalRect {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
            },
            physical_workarea: PhysicalRect {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
            },
            dpi_scale: 1.5,
        };
        let logical = mon.logical_bounds();

        // Near left edge
        let (left_log, _) = dock_popover_bounds(&cfg, &mon, 40.0, 444.0, 212.0);
        assert!(left_log.x >= logical.x + 8.0);
        assert!(left_log.x + left_log.width <= logical.x + logical.width - 8.0);
        assert!(left_log.y >= logical.y + cfg.top_bar.height as f32 + 4.0);
        // Popover bottom clears the top of the dock pill so it does not overlap magnified icons.
        let pill_top =
            logical.y + logical.height - dock::pill_height(&cfg) - dock::bottom_margin(&cfg);
        assert!(left_log.y + left_log.height <= pill_top - 22.0 + 0.01);

        // Near right edge
        let (right_log, _) = dock_popover_bounds(&cfg, &mon, logical.width - 20.0, 444.0, 212.0);
        assert!(right_log.x >= logical.x + 8.0);
        assert!(right_log.x + right_log.width <= logical.x + logical.width - 8.0);
    }

    #[test]
    fn sync_rgba_icon_handles_preserves_handles_when_pixels_unchanged() {
        let icon_a = RgbaIcon {
            width: 2,
            height: 2,
            pixels: std::sync::Arc::from(vec![255u8; 16]),
        };
        let icon_b = RgbaIcon {
            width: 2,
            height: 2,
            pixels: std::sync::Arc::from(vec![128u8; 16]),
        };
        let mut sources = HashMap::new();
        let mut handles = HashMap::new();

        sync_rgba_icon_handles(
            &mut sources,
            &mut handles,
            [("a", Some(&icon_a)), ("b", Some(&icon_b))].into_iter(),
        );
        let orig_handle_a = handles.get("a").expect("handle a").id();
        let orig_handle_b = handles.get("b").expect("handle b").id();

        // Identical icon content should retain the exact same Iced handle ID (no GPU upload).
        let icon_a_clone = RgbaIcon {
            width: 2,
            height: 2,
            pixels: std::sync::Arc::from(vec![255u8; 16]),
        };
        let icon_b_changed = RgbaIcon {
            width: 2,
            height: 2,
            pixels: std::sync::Arc::from(vec![64u8; 16]),
        };
        sync_rgba_icon_handles(
            &mut sources,
            &mut handles,
            [("a", Some(&icon_a_clone)), ("b", Some(&icon_b_changed))].into_iter(),
        );
        assert_eq!(handles.get("a").expect("handle a").id(), orig_handle_a);
        assert_ne!(handles.get("b").expect("handle b").id(), orig_handle_b);

        // Removing an item prunes its cached handle.
        sync_rgba_icon_handles(&mut sources, &mut handles, [("a", Some(&icon_a_clone))].into_iter());
        assert!(handles.contains_key("a"));
        assert!(!handles.contains_key("b"));
    }
}
