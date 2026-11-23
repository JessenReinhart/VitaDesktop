use crate::decorator::{
    is_window_always_on_top, minimize_window, toggle_always_on_top, toggle_maximize_window,
};
use crate::windows::{close_window, launch_target, process_exe_info};
use crate::workspaces::switch_to_space;
use std::ffi::c_void;
use std::sync::mpsc::{self, Sender};
use std::thread::JoinHandle;
use tokio::sync::mpsc::UnboundedSender;
use vita_core::{
    ActiveAppMenu, GlobalMenuAction, GlobalMenuCategory, GlobalMenuItem, WindowMenuAction,
};
use windows::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Accessibility::{
    CUIAutomation, ExpandCollapseState_Collapsed, IUIAutomation, IUIAutomationElement,
    IUIAutomationExpandCollapsePattern, IUIAutomationInvokePattern, TreeScope_Subtree,
    UIA_ControlTypePropertyId, UIA_ExpandCollapsePatternId, UIA_InvokePatternId,
    UIA_MenuBarControlTypeId, UIA_MenuControlTypeId, UIA_MenuItemControlTypeId,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumChildWindows, GetClassNameW, GetMenu, GetMenuItemCount, GetMenuItemInfoW, GetSubMenu,
    GetWindowRect, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, HMENU,
    IsWindow, IsZoomed, MENUITEMINFOW, MFS_CHECKED, MFS_DISABLED, MFT_SEPARATOR, MIIM_FTYPE,
    MIIM_ID, MIIM_STATE, MIIM_STRING, MIIM_SUBMENU, OBJID_CLIENT, PostMessageW, SC_KEYMENU,
    SMTO_ABORTIFHUNG, SendMessageTimeoutW, SetForegroundWindow, WM_COMMAND, WM_GETOBJECT,
    WM_SYSCOMMAND,
};
use windows::core::{BOOL, BSTR, PWSTR};

const MAX_TOP_MENUS: usize = 7;

enum GlobalMenuCmd {
    Inspect { hwnd: isize, monitor_idx: usize },
    RetryInspect { hwnd: isize, monitor_idx: usize },
    LoadSubmenu { hwnd: isize, menu_index: usize },
    Invoke(GlobalMenuAction, usize),
    Shutdown,
}

/// STA COM worker extracting Win32 `HMENU` and Windows UI Automation (`IUIAutomation`) menu bars
/// for the focused external application.
pub struct GlobalMenuWorker {
    cmd_tx: Sender<GlobalMenuCmd>,
    thread: Option<JoinHandle<()>>,
}

impl GlobalMenuWorker {
    pub fn start(out_tx: UnboundedSender<Option<ActiveAppMenu>>) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel::<GlobalMenuCmd>();
        let cmd_tx_retry = cmd_tx.clone();
        let thread = std::thread::Builder::new()
            .name("vita-global-menu".into())
            .spawn(move || {
                unsafe {
                    let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
                }
                let uia: Option<IUIAutomation> =
                    unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) }.ok();

                let mut ctx = MenuWorkerCtx {
                    uia: uia.as_ref(),
                    out_tx: &out_tx,
                    retry_tx: &cmd_tx_retry,
                    retried_hwnd: None,
                    current_menu: None,
                    uia_top_items: Vec::new(),
                    uia_cached_subitems: Vec::new(),
                };

                while let Ok(mut cmd) = cmd_rx.recv() {
                    // Coalesce bursts of rapid foreground switches to the newest request.
                    while let Ok(newer) = cmd_rx.try_recv() {
                        match (&cmd, &newer) {
                            (
                                GlobalMenuCmd::Inspect { .. } | GlobalMenuCmd::RetryInspect { .. },
                                GlobalMenuCmd::Inspect { .. },
                            ) => {
                                cmd = newer;
                            }
                            _ => {
                                handle_menu_cmd(cmd, &mut ctx);
                                cmd = newer;
                            }
                        }
                    }

                    if matches!(cmd, GlobalMenuCmd::Shutdown) {
                        break;
                    }

                    handle_menu_cmd(cmd, &mut ctx);
                }

                drop(ctx);
                drop(uia);
                unsafe {
                    CoUninitialize();
                }
            })
            .ok();

        Self { cmd_tx, thread }
    }

    pub fn inspect_window(&self, hwnd: isize, monitor_idx: usize) {
        let _ = self.cmd_tx.send(GlobalMenuCmd::Inspect { hwnd, monitor_idx });
    }

    pub fn load_submenu(&self, hwnd: isize, menu_index: usize) {
        let _ = self
            .cmd_tx
            .send(GlobalMenuCmd::LoadSubmenu { hwnd, menu_index });
    }

    pub fn invoke(&self, action: GlobalMenuAction, current_space_idx: usize) {
        let _ = self
            .cmd_tx
            .send(GlobalMenuCmd::Invoke(action, current_space_idx));
    }
}

impl Drop for GlobalMenuWorker {
    fn drop(&mut self) {
        let _ = self.cmd_tx.send(GlobalMenuCmd::Shutdown);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

struct MenuWorkerCtx<'a> {
    uia: Option<&'a IUIAutomation>,
    out_tx: &'a UnboundedSender<Option<ActiveAppMenu>>,
    retry_tx: &'a Sender<GlobalMenuCmd>,
    retried_hwnd: Option<isize>,
    current_menu: Option<ActiveAppMenu>,
    uia_top_items: Vec<IUIAutomationElement>,
    uia_cached_subitems: Vec<Vec<IUIAutomationElement>>,
}

fn handle_menu_cmd(cmd: GlobalMenuCmd, ctx: &mut MenuWorkerCtx<'_>) {
    match cmd {
        GlobalMenuCmd::Shutdown => {}
        GlobalMenuCmd::Inspect { hwnd, monitor_idx } => {
            ctx.retried_hwnd = None;
            run_inspect(hwnd, monitor_idx, false, ctx);
        }
        GlobalMenuCmd::RetryInspect { hwnd, monitor_idx } => {
            if ctx.current_menu.as_ref().is_none_or(|m| m.hwnd != hwnd)
                || !ctx.uia_top_items.is_empty()
            {
                return;
            }
            run_inspect(hwnd, monitor_idx, true, ctx);
        }
        GlobalMenuCmd::LoadSubmenu { hwnd, menu_index } => {
            let Some(menu) = ctx.current_menu.as_mut() else {
                return;
            };
            if menu.hwnd != hwnd {
                return;
            }
            let Some(cat) = menu.categories.get_mut(menu_index) else {
                return;
            };
            if cat.loaded {
                return;
            }
            let Some(uia_idx) = cat.lazy_uia_index else {
                return;
            };
            let (Some(auto), Some(top_el)) = (ctx.uia, ctx.uia_top_items.get(uia_idx)) else {
                cat.loaded = true;
                return;
            };

            let (items, elements) = extract_uia_submenu_items(hwnd, menu_index, auto, top_el);
            cat.items = items;
            cat.loaded = true;
            if let Some(slot) = ctx.uia_cached_subitems.get_mut(uia_idx) {
                *slot = elements;
            }
            let _ = ctx.out_tx.send(Some(menu.clone()));
        }
        GlobalMenuCmd::Invoke(action, current_space_idx) => {
            execute_global_menu_action(
                action,
                current_space_idx,
                ctx.uia,
                &ctx.current_menu,
                &ctx.uia_top_items,
                &ctx.uia_cached_subitems,
            );
        }
    }
}

fn run_inspect(hwnd: isize, monitor_idx: usize, is_retry: bool, ctx: &mut MenuWorkerCtx<'_>) {
    ctx.uia_top_items.clear();
    ctx.uia_cached_subitems.clear();

    if hwnd == 0 || !unsafe { IsWindow(Some(HWND(hwnd as *mut c_void))) }.as_bool() {
        ctx.current_menu = None;
        let _ = ctx.out_tx.send(None);
        return;
    }

    let h = HWND(hwnd as *mut c_void);
    let is_chromium = is_chromium_window(h);
    if is_chromium {
        wake_chromium_uia(h);
    }

    let built = build_active_app_menu(h, monitor_idx, is_chromium, ctx.uia, &mut ctx.uia_top_items);
    ctx.uia_cached_subitems
        .resize_with(ctx.uia_top_items.len(), Vec::new);

    if is_chromium
        && ctx.uia_top_items.is_empty()
        && !is_retry
        && ctx.retried_hwnd != Some(hwnd)
    {
        ctx.retried_hwnd = Some(hwnd);
        let tx2 = ctx.retry_tx.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(220));
            let _ = tx2.send(GlobalMenuCmd::RetryInspect { hwnd, monitor_idx });
            std::thread::sleep(std::time::Duration::from_millis(280));
            let _ = tx2.send(GlobalMenuCmd::RetryInspect { hwnd, monitor_idx });
        });
    }

    ctx.current_menu = built.clone();
    let _ = ctx.out_tx.send(built);
}

fn execute_global_menu_action(
    action: GlobalMenuAction,
    current_space_idx: usize,
    uia: Option<&IUIAutomation>,
    current_menu: &Option<ActiveAppMenu>,
    uia_top_items: &[IUIAutomationElement],
    uia_cached_subitems: &[Vec<IUIAutomationElement>],
) {
    match action {
        GlobalMenuAction::Win32Command { hwnd, command_id } => {
            let h = HWND(hwnd as *mut c_void);
            if unsafe { IsWindow(Some(h)) }.as_bool() {
                unsafe {
                    let _ = SetForegroundWindow(h);
                    let _ = PostMessageW(Some(h), WM_COMMAND, WPARAM(command_id as usize), LPARAM(0));
                }
            }
        }
        GlobalMenuAction::UiaInvoke {
            hwnd,
            menu_index,
            item_index,
        } => {
            let h = HWND(hwnd as *mut c_void);
            if !unsafe { IsWindow(Some(h)) }.as_bool() {
                return;
            }
            unsafe {
                let _ = SetForegroundWindow(h);
            }
            if let Some(menu) = current_menu
                && let Some(cat) = menu.categories.get(menu_index)
                && let Some(uia_idx) = cat.lazy_uia_index
                && let Some(el) = uia_cached_subitems.get(uia_idx).and_then(|v| v.get(item_index))
            {
                if let Ok(inv_pat) = unsafe {
                    el.GetCurrentPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId)
                } {
                    let _ = unsafe { inv_pat.Invoke() };
                } else if let Ok(exp_pat) = unsafe {
                    el.GetCurrentPatternAs::<IUIAutomationExpandCollapsePattern>(
                        UIA_ExpandCollapsePatternId,
                    )
                } {
                    let _ = unsafe { exp_pat.Expand() };
                }
            }
        }
        GlobalMenuAction::SyntheticMenuClick { hwnd, index } => {
            let h = HWND(hwnd as *mut c_void);
            if !unsafe { IsWindow(Some(h)) }.as_bool() {
                return;
            }
            unsafe {
                let _ = SetForegroundWindow(h);
            }
            if is_chromium_window(h) {
                wake_chromium_uia(h);
            }
            if let Some(el) = uia_top_items.get(index) {
                if let Ok(exp_pat) = unsafe {
                    el.GetCurrentPatternAs::<IUIAutomationExpandCollapsePattern>(
                        UIA_ExpandCollapsePatternId,
                    )
                } {
                    let _ = unsafe { exp_pat.Expand() };
                    return;
                }
                if let Ok(inv_pat) = unsafe {
                    el.GetCurrentPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId)
                } {
                    let _ = unsafe { inv_pat.Invoke() };
                    return;
                }
            }
            if uia.is_some() {
                unsafe {
                    let _ = PostMessageW(Some(h), WM_SYSCOMMAND, WPARAM(SC_KEYMENU as usize), LPARAM(0));
                }
            }
        }
        GlobalMenuAction::WindowAction(win_act) => match win_act {
            WindowMenuAction::NewWindow { exe_path } => launch_target(&exe_path),
            WindowMenuAction::Minimize(hwnd) => minimize_window(hwnd),
            WindowMenuAction::ToggleMaximize(hwnd) => toggle_maximize_window(hwnd),
            WindowMenuAction::ToggleAlwaysOnTop(hwnd) => toggle_always_on_top(hwnd),
            WindowMenuAction::MoveToNextSpace(_hwnd) => {
                switch_to_space(current_space_idx, current_space_idx + 1);
            }
            WindowMenuAction::QuitApp(hwnd) => close_window(hwnd),
        },
    }
}

fn is_chromium_window(hwnd: HWND) -> bool {
    let mut buf = [0u16; 64];
    let len = unsafe { GetClassNameW(hwnd, &mut buf) };
    if len <= 0 {
        return false;
    }
    let cname = String::from_utf16_lossy(&buf[..len as usize]);
    cname == "Chrome_WidgetWin_1"
}

unsafe extern "system" fn wake_child_proc(hwnd: HWND, _lparam: LPARAM) -> BOOL {
    let mut res: usize = 0;
    unsafe {
        let _ = SendMessageTimeoutW(
            hwnd,
            WM_GETOBJECT,
            WPARAM(0),
            LPARAM(OBJID_CLIENT.0 as isize),
            SMTO_ABORTIFHUNG,
            35,
            Some(&mut res),
        );
    }
    BOOL(1)
}

fn wake_chromium_uia(hwnd: HWND) {
    let mut res: usize = 0;
    unsafe {
        let _ = SendMessageTimeoutW(
            hwnd,
            WM_GETOBJECT,
            WPARAM(0),
            LPARAM(OBJID_CLIENT.0 as isize),
            SMTO_ABORTIFHUNG,
            35,
            Some(&mut res),
        );
        let _ = EnumChildWindows(Some(hwnd), Some(wake_child_proc), LPARAM(0));
    }
}

fn build_active_app_menu(
    hwnd: HWND,
    monitor_idx: usize,
    is_chromium: bool,
    uia: Option<&IUIAutomation>,
    uia_top_items: &mut Vec<IUIAutomationElement>,
) -> Option<ActiveAppMenu> {
    let mut pid = 0u32;
    unsafe {
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
    }
    let (exe_path, exe_name) = process_exe_info(pid);
    let title = read_hwnd_title(hwnd);
    let app_name = friendly_app_name(&exe_name, &title);

    let mut categories = vec![build_universal_app_category(
        hwnd.0 as isize,
        &app_name,
        &exe_path,
    )];

    let hmenu = unsafe { GetMenu(hwnd) };
    let mut extracted = if !hmenu.is_invalid() && !hmenu.0.is_null() {
        extract_win32_hmenu(hwnd.0 as isize, hmenu)
    } else if let Some(auto) = uia {
        extract_uia_top_menus(auto, hwnd, uia_top_items)
    } else {
        Vec::new()
    };

    if extracted.is_empty() && is_chromium {
        extracted = fallback_electron_menus(hwnd.0 as isize, &exe_name);
    }

    if extracted
        .first()
        .is_some_and(|c| c.label.eq_ignore_ascii_case(&app_name))
    {
        extracted.remove(0);
    }
    categories.extend(extracted);

    Some(ActiveAppMenu {
        hwnd: hwnd.0 as isize,
        app_name,
        exe_path,
        monitor_idx,
        categories,
    })
}

fn fallback_electron_menus(hwnd: isize, exe_name: &str) -> Vec<GlobalMenuCategory> {
    let labels: &[&str] = if exe_name.eq_ignore_ascii_case("Antigravity.exe") {
        &["Workspace", "View", "Window"]
    } else if exe_name.eq_ignore_ascii_case("Code.exe") {
        &["File", "Edit", "Selection", "View", "Go", "Run", "Terminal"]
    } else {
        &[]
    };

    labels
        .iter()
        .enumerate()
        .map(|(idx, &label)| GlobalMenuCategory {
            label: label.to_string(),
            items: vec![GlobalMenuItem {
                label: format!("Focus {label} menu in app"),
                shortcut: None,
                enabled: true,
                checked: false,
                is_separator: false,
                action: Some(GlobalMenuAction::SyntheticMenuClick {
                    hwnd,
                    index: idx + 1,
                }),
            }],
            lazy_uia_index: None,
            loaded: true,
        })
        .collect()
}

fn build_universal_app_category(
    hwnd: isize,
    app_name: &str,
    exe_path: &str,
) -> GlobalMenuCategory {
    let is_max = unsafe { IsZoomed(HWND(hwnd as *mut c_void)) }.as_bool();
    let is_top = is_window_always_on_top(hwnd);

    let items = vec![
        GlobalMenuItem {
            label: format!("New {app_name} Window"),
            shortcut: None,
            enabled: !exe_path.trim().is_empty(),
            checked: false,
            is_separator: false,
            action: Some(GlobalMenuAction::WindowAction(WindowMenuAction::NewWindow {
                exe_path: exe_path.to_string(),
            })),
        },
        separator_item(),
        GlobalMenuItem {
            label: "Minimize".to_string(),
            shortcut: None,
            enabled: true,
            checked: false,
            is_separator: false,
            action: Some(GlobalMenuAction::WindowAction(WindowMenuAction::Minimize(hwnd))),
        },
        GlobalMenuItem {
            label: if is_max {
                "Restore Size".to_string()
            } else {
                "Maximize".to_string()
            },
            shortcut: None,
            enabled: true,
            checked: false,
            is_separator: false,
            action: Some(GlobalMenuAction::WindowAction(WindowMenuAction::ToggleMaximize(hwnd))),
        },
        GlobalMenuItem {
            label: "Always on Top".to_string(),
            shortcut: None,
            enabled: true,
            checked: is_top,
            is_separator: false,
            action: Some(GlobalMenuAction::WindowAction(WindowMenuAction::ToggleAlwaysOnTop(hwnd))),
        },
        GlobalMenuItem {
            label: "Move to Next Space".to_string(),
            shortcut: None,
            enabled: true,
            checked: false,
            is_separator: false,
            action: Some(GlobalMenuAction::WindowAction(WindowMenuAction::MoveToNextSpace(hwnd))),
        },
        separator_item(),
        GlobalMenuItem {
            label: format!("Quit {app_name}"),
            shortcut: Some("Alt+F4".to_string()),
            enabled: true,
            checked: false,
            is_separator: false,
            action: Some(GlobalMenuAction::WindowAction(WindowMenuAction::QuitApp(hwnd))),
        },
    ];

    GlobalMenuCategory {
        label: app_name.to_string(),
        items,
        lazy_uia_index: None,
        loaded: true,
    }
}

fn extract_win32_hmenu(hwnd: isize, hmenu: HMENU) -> Vec<GlobalMenuCategory> {
    let count = unsafe { GetMenuItemCount(Some(hmenu)) };
    if count <= 0 {
        return Vec::new();
    }

    let mut categories = Vec::new();
    let cap = (count as usize).min(MAX_TOP_MENUS);
    for idx in 0..cap {
        let Some((raw_label, _, _, _, _, sub)) = read_menu_item_info(hmenu, idx as u32) else {
            continue;
        };
        let (label, _) = parse_menu_label(&raw_label);
        if label.trim().is_empty() {
            continue;
        }
        let items = if !sub.is_invalid() && !sub.0.is_null() {
            extract_win32_submenu_items(hwnd, sub)
        } else {
            Vec::new()
        };
        categories.push(GlobalMenuCategory {
            label,
            items,
            lazy_uia_index: None,
            loaded: true,
        });
    }
    categories
}

fn extract_win32_submenu_items(hwnd: isize, sub: HMENU) -> Vec<GlobalMenuItem> {
    let count = unsafe { GetMenuItemCount(Some(sub)) };
    if count <= 0 {
        return Vec::new();
    }
    let mut out = Vec::new();
    for idx in 0..(count as usize).min(36) {
        let Some((raw, id, is_sep, enabled, checked, nested)) = read_menu_item_info(sub, idx as u32)
        else {
            continue;
        };
        if is_sep {
            if !out.is_empty() && !out.last().is_some_and(|m: &GlobalMenuItem| m.is_separator) {
                out.push(separator_item());
            }
            continue;
        }
        let (label, shortcut) = parse_menu_label(&raw);
        if label.trim().is_empty() {
            continue;
        }
        let has_nested = !nested.is_invalid() && !nested.0.is_null();
        out.push(GlobalMenuItem {
            label: if has_nested {
                format!("{label}  ›")
            } else {
                label
            },
            shortcut,
            enabled: enabled && !has_nested && id != 0,
            checked,
            is_separator: false,
            action: (!has_nested && id != 0)
                .then_some(GlobalMenuAction::Win32Command { hwnd, command_id: id }),
        });
    }
    if out.last().is_some_and(|i| i.is_separator) {
        out.pop();
    }
    out
}

fn read_menu_item_info(
    menu: HMENU,
    pos: u32,
) -> Option<(String, u32, bool, bool, bool, HMENU)> {
    let mut buf = [0u16; 128];
    let mut info = MENUITEMINFOW {
        cbSize: std::mem::size_of::<MENUITEMINFOW>() as u32,
        fMask: MIIM_FTYPE | MIIM_STATE | MIIM_ID | MIIM_SUBMENU | MIIM_STRING,
        dwTypeData: PWSTR(buf.as_mut_ptr()),
        cch: buf.len() as u32,
        ..Default::default()
    };
    if unsafe { GetMenuItemInfoW(menu, pos, true, &mut info) }.is_err() {
        return None;
    }
    let is_sep = (info.fType.0 & MFT_SEPARATOR.0) != 0;
    let enabled = (info.fState.0 & MFS_DISABLED.0) == 0;
    let checked = (info.fState.0 & MFS_CHECKED.0) != 0;
    let sub = unsafe { GetSubMenu(menu, pos as i32) };
    let text = if is_sep {
        String::new()
    } else {
        let len = (info.cch as usize).min(buf.len());
        String::from_utf16_lossy(&buf[..len])
    };
    Some((text, info.wID, is_sep, enabled, checked, sub))
}

fn extract_uia_top_menus(
    auto: &IUIAutomation,
    hwnd: HWND,
    uia_top_items: &mut Vec<IUIAutomationElement>,
) -> Vec<GlobalMenuCategory> {
    let Ok(root) = (unsafe { auto.ElementFromHandle(hwnd) }) else {
        return Vec::new();
    };
    let Ok(menubar_cond) = (unsafe {
        auto.CreatePropertyCondition(
            UIA_ControlTypePropertyId,
            &VARIANT::from(UIA_MenuBarControlTypeId.0),
        )
    }) else {
        return Vec::new();
    };
    let Ok(item_cond) = (unsafe {
        auto.CreatePropertyCondition(
            UIA_ControlTypePropertyId,
            &VARIANT::from(UIA_MenuItemControlTypeId.0),
        )
    }) else {
        return Vec::new();
    };

    if let Ok(bars) = unsafe { root.FindAll(TreeScope_Subtree, &menubar_cond) }
        && let Ok(bar_len) = unsafe { bars.Length() }
    {
        for b in 0..bar_len {
            let Ok(bar) = (unsafe { bars.GetElement(b) }) else {
                continue;
            };
            let auto_id = unsafe { bar.CurrentAutomationId() }
                .map(|s| s.to_string())
                .unwrap_or_default();
            let name = unsafe { bar.CurrentName() }
                .map(|s| s.to_string())
                .unwrap_or_default();
            if auto_id.eq_ignore_ascii_case("SystemMenuBar")
                || name.eq_ignore_ascii_case("System")
            {
                continue;
            }

            // Search TreeScope_Subtree inside the menubar so Electron wrapper divs don't hide items.
            let Ok(items) = (unsafe { bar.FindAll(TreeScope_Subtree, &item_cond) }) else {
                continue;
            };
            let Ok(ilen) = (unsafe { items.Length() }) else {
                continue;
            };
            if ilen <= 0 {
                continue;
            }

            let bar_rc = unsafe { bar.CurrentBoundingRectangle() }.unwrap_or_default();
            let mut collected: Vec<(i32, String, IUIAutomationElement)> = Vec::new();
            for idx in 0..ilen {
                let Ok(item) = (unsafe { items.GetElement(idx) }) else {
                    continue;
                };
                let item_rc = unsafe { item.CurrentBoundingRectangle() }.unwrap_or_default();
                if bar_rc.bottom > bar_rc.top
                    && item_rc.bottom > item_rc.top
                    && (item_rc.top < bar_rc.top - 6 || item_rc.bottom > bar_rc.bottom + 8)
                {
                    continue;
                }
                let raw_name = unsafe { item.CurrentName() }
                    .map(|s: BSTR| s.to_string())
                    .unwrap_or_default();
                let (label, _) = parse_menu_label(&raw_name);
                let trimmed = label.trim();
                if trimmed.is_empty()
                    || trimmed.eq_ignore_ascii_case("System")
                    || trimmed.eq_ignore_ascii_case("Application")
                {
                    continue;
                }
                collected.push((item_rc.left, trimmed.to_string(), item));
            }
            if !collected.is_empty() {
                collected.sort_by_key(|(left, _, _)| *left);
                let mut categories = Vec::new();
                for (_, label, item) in collected.into_iter().take(MAX_TOP_MENUS) {
                    let uia_idx = uia_top_items.len();
                    uia_top_items.push(item);
                    categories.push(GlobalMenuCategory {
                        label,
                        items: Vec::new(),
                        lazy_uia_index: Some(uia_idx),
                        loaded: false,
                    });
                }
                return categories;
            }
        }
    }

    // Fallback: look for top-strip MenuItem elements directly inside the top 60px of the window.
    let mut win_rc = RECT::default();
    if unsafe { GetWindowRect(hwnd, &mut win_rc) }.is_err() {
        return Vec::new();
    }
    let Ok(all_items) = (unsafe { root.FindAll(TreeScope_Subtree, &item_cond) }) else {
        return Vec::new();
    };
    let Ok(alen) = (unsafe { all_items.Length() }) else {
        return Vec::new();
    };

    let mut top_strip: Vec<(i32, String, IUIAutomationElement)> = Vec::new();
    for idx in 0..alen {
        let Ok(item) = (unsafe { all_items.GetElement(idx) }) else {
            continue;
        };
        let Ok(rc) = (unsafe { item.CurrentBoundingRectangle() }) else {
            continue;
        };
        if rc.right <= rc.left
            || rc.bottom <= rc.top
            || rc.top < win_rc.top - 4
            || rc.bottom > win_rc.top + 60
        {
            continue;
        }
        let raw_name = unsafe { item.CurrentName() }
            .map(|s: BSTR| s.to_string())
            .unwrap_or_default();
        let (label, _) = parse_menu_label(&raw_name);
        let trimmed = label.trim();
        if trimmed.is_empty()
            || matches!(
                trimmed.to_ascii_lowercase().as_str(),
                "system" | "application" | "minimize" | "maximize" | "restore" | "close"
            )
        {
            continue;
        }
        top_strip.push((rc.left, trimmed.to_string(), item));
    }

    top_strip.sort_by_key(|(left, _, _)| *left);
    let mut categories = Vec::new();
    for (_, label, item) in top_strip.into_iter().take(MAX_TOP_MENUS) {
        let uia_idx = uia_top_items.len();
        uia_top_items.push(item);
        categories.push(GlobalMenuCategory {
            label,
            items: Vec::new(),
            lazy_uia_index: Some(uia_idx),
            loaded: false,
        });
    }
    categories
}

fn extract_uia_submenu_items(
    hwnd: isize,
    menu_index: usize,
    auto: &IUIAutomation,
    top_item: &IUIAutomationElement,
) -> (Vec<GlobalMenuItem>, Vec<IUIAutomationElement>) {
    let mut expanded_here = false;
    if let Ok(exp_pat) = unsafe {
        top_item.GetCurrentPatternAs::<IUIAutomationExpandCollapsePattern>(UIA_ExpandCollapsePatternId)
    } && unsafe { exp_pat.CurrentExpandCollapseState() }
        .is_ok_and(|s| s == ExpandCollapseState_Collapsed)
    {
        expanded_here = unsafe { exp_pat.Expand() }.is_ok();
    } else if let Ok(inv_pat) =
        unsafe { top_item.GetCurrentPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId) }
    {
        expanded_here = unsafe { inv_pat.Invoke() }.is_ok();
    }

    if expanded_here {
        std::thread::sleep(std::time::Duration::from_millis(70));
    }

    let mut out_items = Vec::new();
    let mut out_els = Vec::new();

    if let Ok(cond) = unsafe {
        auto.CreatePropertyCondition(
            UIA_ControlTypePropertyId,
            &VARIANT::from(UIA_MenuItemControlTypeId.0),
        )
    } {
        if let Ok(found) = unsafe { top_item.FindAll(TreeScope_Subtree, &cond) } {
            collect_uia_submenu_array(hwnd, menu_index, &found, &mut out_items, &mut out_els);
        }

        // Electron/Chromium frequently renders open dropdown popups as UIA_MenuControlTypeId outside top_item.
        if out_items.is_empty()
            && let Ok(root) = unsafe { auto.ElementFromHandle(HWND(hwnd as *mut c_void)) }
            && let Ok(menu_cond) = unsafe {
                auto.CreatePropertyCondition(
                    UIA_ControlTypePropertyId,
                    &VARIANT::from(UIA_MenuControlTypeId.0),
                )
            }
            && let Ok(menus) = unsafe { root.FindAll(TreeScope_Subtree, &menu_cond) }
            && let Ok(mlen) = unsafe { menus.Length() }
        {
            for m in 0..mlen {
                if let Ok(menu_el) = (unsafe { menus.GetElement(m) })
                    && let Ok(found) = unsafe { menu_el.FindAll(TreeScope_Subtree, &cond) }
                {
                    collect_uia_submenu_array(
                        hwnd,
                        menu_index,
                        &found,
                        &mut out_items,
                        &mut out_els,
                    );
                    if !out_items.is_empty() {
                        break;
                    }
                }
            }
        }
    }

    if expanded_here
        && let Ok(exp_pat) = unsafe {
            top_item.GetCurrentPatternAs::<IUIAutomationExpandCollapsePattern>(
                UIA_ExpandCollapsePatternId,
            )
        }
    {
        let _ = unsafe { exp_pat.Collapse() };
    }

    (out_items, out_els)
}

fn collect_uia_submenu_array(
    hwnd: isize,
    menu_index: usize,
    found: &windows::Win32::UI::Accessibility::IUIAutomationElementArray,
    out_items: &mut Vec<GlobalMenuItem>,
    out_els: &mut Vec<IUIAutomationElement>,
) {
    let Ok(len) = (unsafe { found.Length() }) else {
        return;
    };
    for i in 0..len {
        if out_items.len() >= 28 {
            break;
        }
        let Ok(child) = (unsafe { found.GetElement(i) }) else {
            continue;
        };
        let raw_name = unsafe { child.CurrentName() }
            .map(|s| s.to_string())
            .unwrap_or_default();
        let (label, _) = parse_menu_label(&raw_name);
        let trimmed = label.trim();
        if trimmed.is_empty() {
            continue;
        }
        let accel = unsafe { child.CurrentAcceleratorKey() }
            .map(|s| s.to_string())
            .ok()
            .filter(|s| !s.trim().is_empty());
        let enabled = unsafe { child.CurrentIsEnabled() }.is_ok_and(|b| b.as_bool());

        let item_idx = out_els.len();
        out_els.push(child);
        out_items.push(GlobalMenuItem {
            label: trimmed.to_string(),
            shortcut: accel,
            enabled,
            checked: false,
            is_separator: false,
            action: Some(GlobalMenuAction::UiaInvoke {
                hwnd,
                menu_index,
                item_index: item_idx,
            }),
        });
    }
}

fn separator_item() -> GlobalMenuItem {
    GlobalMenuItem {
        label: String::new(),
        shortcut: None,
        enabled: false,
        checked: false,
        is_separator: true,
        action: None,
    }
}

fn read_hwnd_title(hwnd: HWND) -> String {
    let len = unsafe { GetWindowTextLengthW(hwnd) };
    if len <= 0 {
        return String::new();
    }
    let mut buf = vec![0u16; (len + 1) as usize];
    let n = unsafe { GetWindowTextW(hwnd, &mut buf) };
    if n <= 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buf[..n as usize])
}

pub fn friendly_app_name(exe_name: &str, title: &str) -> String {
    let stem = exe_name
        .strip_suffix(".exe")
        .or_else(|| exe_name.strip_suffix(".EXE"))
        .unwrap_or(exe_name);
    match stem.to_ascii_lowercase().as_str() {
        "msedge" => "Edge".to_string(),
        "chrome" => "Chrome".to_string(),
        "firefox" => "Firefox".to_string(),
        "code" => "VS Code".to_string(),
        "windowsterminal" | "wt" => "Terminal".to_string(),
        "explorer" => "Files".to_string(),
        "notepad" => "Notepad".to_string(),
        "mspaint" => "Paint".to_string(),
        "systemsettings" => "Windows Settings".to_string(),
        "" => title
            .split(['—', '-'])
            .next_back()
            .unwrap_or("Application")
            .trim()
            .chars()
            .take(20)
            .collect(),
        _ => {
            let mut chars = stem.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str()),
                None => "Application".to_string(),
            }
        }
    }
}

/// Strips Win32 `&` accelerator markers (`&&` -> `&`) and splits trailing `\tShortcut`.
pub fn parse_menu_label(raw: &str) -> (String, Option<String>) {
    let (label_raw, shortcut_raw) = match raw.split_once('\t') {
        Some((l, r)) => (l, Some(r.trim())),
        None => (raw, None),
    };

    let mut clean = String::with_capacity(label_raw.len());
    let mut chars = label_raw.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '&' {
            if chars.peek() == Some(&'&') {
                clean.push('&');
                chars.next();
            }
        } else {
            clean.push(c);
        }
    }

    let shortcut = shortcut_raw
        .filter(|s| !s.is_empty())
        .map(ToString::to_string);
    (clean.trim().to_string(), shortcut)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_label_mnemonic_and_shortcut_parse_cleanly() {
        assert_eq!(
            parse_menu_label("E&xit\tAlt+F4"),
            ("Exit".to_string(), Some("Alt+F4".to_string()))
        );
        assert_eq!(
            parse_menu_label("Search && &Replace\tCtrl+H"),
            ("Search & Replace".to_string(), Some("Ctrl+H".to_string()))
        );
        assert_eq!(parse_menu_label("&New"), ("New".to_string(), None));
    }

    #[test]
    fn friendly_names_format_cleanly() {
        assert_eq!(friendly_app_name("WindowsTerminal.exe", "PowerShell"), "Terminal");
        assert_eq!(friendly_app_name("Code.exe", "main.rs - VS Code"), "VS Code");
        assert_eq!(friendly_app_name("notepad.exe", "Untitled"), "Notepad");
        assert_eq!(friendly_app_name("obsidian.exe", "Notes"), "Obsidian");
    }
}
