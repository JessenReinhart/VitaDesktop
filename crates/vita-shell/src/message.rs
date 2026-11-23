use crate::surfaces::settings::SettingsTab;
use chrono::{DateTime, Local};
use iced::{Point, keyboard, window};
use std::collections::HashMap;
use std::time::Instant;
use vita_core::{
    AccentPreset, ActiveAppMenu, AppCategory, DecoratedWindowState, DecoratorButtonStyle,
    DockBackgroundStyle, DockVisibility, GlobalMenuAction, InstalledApp, LauncherViewMode,
    MonitorInfo, RgbaIcon, RunningWindow, ShellConfig, ShellPreset, SpaceInfo, SurfaceMaterial,
    SystemSummary, ThemeMode, TopBarStyle,
};
use vita_win32::HotkeyEvent;

#[derive(Debug, Clone)]
pub enum Message {
    WindowOpened(window::Id),
    WindowRawId(window::Id, u64),
    WindowEvent(window::Id, window::Event),
    KeyboardEvent(keyboard::Event),

    // Background data streams
    Tick(DateTime<Local>),
    DockSensorTick,
    TelemetryUpdated(SystemSummary),
    WindowsUpdated(Vec<RunningWindow>),
    MonitorsChanged(Vec<MonitorInfo>),
    PinnedIconsLoaded(HashMap<String, RgbaIcon>),
    InstalledAppsLoaded(Vec<InstalledApp>),
    ConfigReloaded(ShellConfig),
    Hotkey(HotkeyEvent),

    // Global Menu & Custom Window Decorator
    ActiveAppMenuUpdated(Option<ActiveAppMenu>),
    OpenGlobalMenu { monitor_idx: usize, menu_index: usize },
    GlobalMenuInvoke(GlobalMenuAction),
    DecoratedWindowUpdated(Option<DecoratedWindowState>),
    DecoratorHeaderPressed(isize),
    DecoratorMinimize(isize),
    DecoratorToggleMaximize(isize),
    DecoratorClose(isize),

    // Dock pointer & animation (scoped by monitor index)
    DockPointerEntered(usize),
    DockPointerMoved(usize, Point),
    DockPointerExited(usize),
    AnimationFrame(Instant),

    // Dock actions & popover
    DockItemClicked(usize, usize),
    DockRightClicked(usize),
    DockMenuLaunchNew(String),
    DockMenuSwitchToPicker,
    DockMenuTogglePin(String),
    DockMenuCloseAll(String),
    FocusSpecificWindow(isize),
    CloseSpecificWindow(isize),

    // Launcher actions
    ToggleLauncherOn(usize),
    LauncherQueryChanged(String),
    LauncherViewModeChanged(LauncherViewMode),
    LauncherCategorySelected(AppCategory),
    LauncherPageChanged(usize),
    LauncherLaunchApp(String),
    LauncherLaunchSelected,
    LauncherTogglePin(String),

    // Quick Controls & Hardware actions
    ToggleQuickControls,
    ToggleQuickControlsOn(usize),
    VolumeChanged(u8),
    ToggleMute,
    BrightnessChanged(u8),
    ToggleFocusMode,
    TogglePowerConfirm,
    PowerLock,
    PowerSleep,
    PowerRestart,
    PowerShutdown,
    OpenTargetAndDismiss(String),

    // Spaces & Notifications
    SpacesUpdated(Vec<SpaceInfo>),
    ToggleSpaces,
    ToggleSpacesOn(usize),
    SwitchSpace(usize),
    CreateNewSpace,
    ToggleNotifications,
    ToggleNotificationsOn(usize),
    DismissNotification(u64),
    ClearAllNotifications,
    ToastExpired(u64),

    // Settings Studio
    OpenSettings,
    CloseSettings,
    MinimizeSettings,
    DragSettingsWindow,
    SettingsTabSelected(SettingsTab),
    SettingsSelectPreset(ShellPreset),
    SettingsSelectThemeMode(ThemeMode),
    SettingsSelectMaterial(SurfaceMaterial),
    SettingsSelectAccent(AccentPreset),
    SettingsToggleReduceMotion,
    SettingsToggleIncreaseContrast,
    SettingsSelectTopBarStyle(TopBarStyle),
    SettingsSelectTopBarHeight(u32),
    SettingsToggleShowSeconds,
    SettingsToggleGlobalMenu,
    SettingsDockIconSizeChanged(u32),
    SettingsDockHeightChanged(u32),
    SettingsDockMagnifyChanged(f32),
    SettingsSelectDockVisibility(DockVisibility),
    SettingsToggleDockReserveWorkArea,
    SettingsToggleDockAllMonitors,
    SettingsSelectDockBackgroundStyle(DockBackgroundStyle),
    SettingsDockBackgroundOpacityChanged(u8),
    SettingsDockBackgroundColorChanged(String),
    SettingsDockCornerRadiusChanged(u32),
    SettingsToggleDockBorder,
    SettingsToggleDockSpaceChip,
    SettingsToggleDockResumeChip,
    SettingsToggleDecoratorEnabled,
    SettingsToggleDecoratorDwmSync,
    SettingsToggleDecoratorOverlay,
    SettingsToggleDecoratorMaxTopBar,
    SettingsToggleHideMaximizedTitlebar,
    SettingsMaximizedTitlebarHeightChanged(u32),
    SettingsSelectDecoratorButtonStyle(DecoratorButtonStyle),
    SettingsToggleCalmZone,
    SettingsCalmUserNameChanged(String),
    SettingsToggleCalmThread,
    SettingsCalmThreadTitleChanged(String),
    SettingsCalmThreadSubtitleChanged(String),
    SettingsToggleHideTaskbar,
    SettingsOpenConfigFile,
    SettingsResetDefaults,

    // Shell controls
    CyclePreset,
    ToggleTheme,
    CaptureScreenshots,
    ScreenshotCaptured(&'static str, window::Screenshot),
    ExitRequested,
}
