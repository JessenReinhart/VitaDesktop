# VitaDesktop

Pure-Rust GPU-accelerated desktop environment and shell overlay for Windows 11, built with `iced` (`0.14`, `wgpu` DX12) and `windows` (`0.62`).

## Architecture

- **`crates/vita-core`** — Platform-free domain models, theme token resolution (`ThemeTokens`, 7 `AccentPreset` palettes, WCAG 2.2 contrast validation), `ShellConfig` TOML persistence + hot reload, spring dock magnification (`DockMagnifier`), and fuzzy app search.
- **`crates/vita-win32`** — Native Windows 11, Win32, DWM, COM, and UI Automation integration:
  - Per-monitor layout and DPI tracking (`enumerate_monitors`, `GetDpiForMonitor`).
  - Non-destructive native taskbar hiding across primary and secondary displays (`TaskbarGuard` + active `SetWinEventHook` enforcement) with an out-of-process crash-recovery `--watchdog <pid>`.
  - Per-monitor GNOME-style work-area reservation (`WorkAreaGuard` via `SPI_SETWORKAREA`) and `workareas.bak` crash restoration.
  - Active app **Global Menu** extraction via Win32 `GetMenu` (`HMENU`) and Windows UI Automation (`IUIAutomation`).
  - **Custom Window Decorator** & DWM theme sync (`DecoratorTracker`, `DwmWindowThemer` for border/caption/corner customization, `FullHeader` and flush `ControlsOnly` titlebar overlays).
  - Live window tracking, `AppsFolder` icon caching, Virtual Desktop enumeration/switching, global hotkeys, audio/brightness controls, and system power actions.
- **`crates/vita-shell`** — Multi-window, multi-monitor `iced::daemon` UI surfaces:
  - **Top Bar**: Horizon floating pills (`HorizonLight` / `HorizonDark`) or Pantheon full-width acrylic bar (`PantheonLight` / `PantheonDark`), with Global Menu strip, quick status cluster, notifications bell, and GNOME-Unite maximized window controls.
  - **Dock**: Multi-monitor Intelli-Hide / Always-visible / Auto-hide dock with spring magnification, reducible dock pill height (`36–84 px`), optional Space/Resume utility chips, multi-window picker, customizable background style/color/opacity/radius, and a Settings toggle to show/hide the dock entirely.
  - **Launcher, Quick Controls, Spaces, Calm Zone, Notifications & Settings Studio**: Full customization persisted to `%APPDATA%\VitaDesktop\config.toml`.

## Build & Run

Requires the Rust toolchain (`edition = "2024"`) on Windows 10/11.

```powershell
# Lint & unit tests
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# Optimized release build (target\release\vita-shell.exe)
cargo build --release
```

### Usage

```powershell
# Launch VitaDesktop shell overlay (hides native taskbar + spawns recovery watchdog)
.\target\release\vita-shell.exe

# Run in non-intrusive companion mode (keeps native Windows Taskbar visible)
.\target\release\vita-shell.exe --no-hide-taskbar

# Launch with a specific preset and open Settings directly
.\target\release\vita-shell.exe --no-hide-taskbar --preset pantheon-dark --open-settings --settings-tab bar

# Restore native Windows taskbar and original monitor work areas unconditionally
.\target\release\vita-shell.exe --restore-taskbar
```

### Hotkeys

- `Alt + Space` — Toggle Application Launcher on the active monitor
- `` Alt + ` `` — Toggle Spaces Overview
- `Ctrl + Alt + Shift + R` — Emergency taskbar + work-area restore and exit

## License

MIT
