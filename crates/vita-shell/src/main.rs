#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod icons;
mod message;
mod style;
mod surfaces;

use app::VitaShell;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use surfaces::settings::SettingsTab;
use tracing_subscriber::EnvFilter;
use vita_core::{ShellConfig, ShellPreset};
use vita_win32::{
    TaskbarGuard, restore_all_taskbars, restore_saved_workareas, run_watchdog, spawn_watchdog,
};

#[derive(Debug)]
enum CliCommand {
    Watchdog(u32),
    RestoreTaskbar,
    Run {
        companion_mode: bool,
        preset: Option<ShellPreset>,
        screenshot_dir: Option<PathBuf>,
        open_launcher: bool,
        open_quick_controls: bool,
        open_spaces: bool,
        open_notifications: bool,
        open_settings: bool,
        open_global_menu: bool,
        open_decorator_preview: bool,
        settings_tab: SettingsTab,
    },
}

fn parse_cli() -> CliCommand {
    let args: Vec<String> = std::env::args().collect();
    let mut i = 1;
    let mut companion_mode = false;
    let mut preset = None;
    let mut screenshot_dir = None;
    let mut open_launcher = false;
    let mut open_quick_controls = false;
    let mut open_spaces = false;
    let mut open_notifications = false;
    let mut open_settings = false;
    let mut open_global_menu = false;
    let mut open_decorator_preview = false;
    let mut settings_tab = SettingsTab::default();

    while i < args.len() {
        match args[i].as_str() {
            "--watchdog" => {
                if let Some(pid_str) = args.get(i + 1)
                    && let Ok(pid) = pid_str.parse::<u32>()
                {
                    return CliCommand::Watchdog(pid);
                }
            }
            "--restore-taskbar" => return CliCommand::RestoreTaskbar,
            "--no-hide-taskbar" | "--companion" => {
                companion_mode = true;
            }
            "--open-launcher" => {
                open_launcher = true;
            }
            "--open-quick-controls" => {
                open_quick_controls = true;
            }
            "--open-spaces" => {
                open_spaces = true;
            }
            "--open-notifications" => {
                open_notifications = true;
            }
            "--open-settings" => {
                open_settings = true;
            }
            "--open-global-menu" => {
                open_global_menu = true;
            }
            "--open-decorator-preview" => {
                open_decorator_preview = true;
            }
            "--settings-tab" => {
                if let Some(t) = args.get(i + 1) {
                    settings_tab = match t.to_ascii_lowercase().as_str() {
                        "overview" => SettingsTab::Overview,
                        "appearance" => SettingsTab::Appearance,
                        "bar" | "bar-and-dock" | "dock" => SettingsTab::BarAndDock,
                        "calm" | "calm-zone" => SettingsTab::CalmZone,
                        "system" => SettingsTab::System,
                        _ => SettingsTab::default(),
                    };
                    i += 1;
                }
            }
            "--preset" => {
                if let Some(p) = args.get(i + 1) {
                    preset = match p.to_ascii_lowercase().as_str() {
                        "horizon-light" => Some(ShellPreset::HorizonLight),
                        "horizon-dark" => Some(ShellPreset::HorizonDark),
                        "pantheon-light" => Some(ShellPreset::PantheonLight),
                        "pantheon-dark" => Some(ShellPreset::PantheonDark),
                        _ => None,
                    };
                    i += 1;
                }
            }
            "--screenshot-dir" => {
                if let Some(dir) = args.get(i + 1) {
                    screenshot_dir = Some(PathBuf::from(dir));
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }

    CliCommand::Run {
        companion_mode,
        preset,
        screenshot_dir,
        open_launcher,
        open_quick_controls,
        open_spaces,
        open_notifications,
        open_settings,
        open_global_menu,
        open_decorator_preview,
        settings_tab,
    }
}

fn main() -> iced::Result {
    match parse_cli() {
        CliCommand::Watchdog(parent_pid) => {
            run_watchdog(parent_pid);
            Ok(())
        }
        CliCommand::RestoreTaskbar => {
            restore_saved_workareas();
            restore_all_taskbars();
            Ok(())
        }
        CliCommand::Run {
            companion_mode,
            preset,
            screenshot_dir,
            open_launcher,
            open_quick_controls,
            open_spaces,
            open_notifications,
            open_settings,
            open_global_menu,
            open_decorator_preview,
            settings_tab,
        } => {
            let filter = EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("vita_shell=info,vita_win32=info,warn"));
            tracing_subscriber::fmt().with_env_filter(filter).init();

            let default_hook = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |info| {
                restore_saved_workareas();
                restore_all_taskbars();
                default_hook(info);
            }));

            let cfg = ShellConfig::load_or_default();
            let mut hide_taskbar = cfg.hide_native_taskbar && !companion_mode;
            let mut watchdog_spawned = false;
            if hide_taskbar {
                if let Err(err) = spawn_watchdog() {
                    tracing::error!(
                        ?err,
                        "Failed to spawn taskbar recovery watchdog; staying in companion mode"
                    );
                    hide_taskbar = false;
                } else {
                    watchdog_spawned = true;
                }
            }

            let taskbar_guard = Arc::new(Mutex::new(TaskbarGuard::new(hide_taskbar)));
            let guard_for_boot = Arc::clone(&taskbar_guard);

            iced::daemon(
                move || {
                    VitaShell::boot(
                        preset,
                        companion_mode,
                        screenshot_dir.clone(),
                        open_launcher,
                        open_quick_controls,
                        open_spaces,
                        open_notifications,
                        open_settings,
                        open_global_menu,
                        open_decorator_preview,
                        settings_tab,
                        Arc::clone(&guard_for_boot),
                        watchdog_spawned,
                    )
                },
                VitaShell::update,
                VitaShell::view,
            )
            .title(VitaShell::title)
            .subscription(VitaShell::subscription)
            .style(VitaShell::daemon_style)
            .font(style::FONT_POPPINS_REGULAR_BYTES)
            .font(style::FONT_POPPINS_MEDIUM_BYTES)
            .font(style::FONT_POPPINS_SEMIBOLD_BYTES)
            .font(style::FONT_JETBRAINS_MONO_BYTES)
            .default_font(style::FONT_REGULAR)
            .run()
        }
    }
}
