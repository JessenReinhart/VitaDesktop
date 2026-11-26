use std::ffi::c_void;
use std::mem::size_of;
use vita_core::PhysicalRect;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Dwm::{
    DWM_SYSTEMBACKDROP_TYPE, DWM_WINDOW_CORNER_PREFERENCE, DWMSBT_NONE, DWMSBT_TRANSIENTWINDOW,
    DWMWA_EXCLUDED_FROM_PEEK, DWMWA_SYSTEMBACKDROP_TYPE, DWMWA_TRANSITIONS_FORCEDISABLED,
    DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND,
    DWMWCP_ROUND, DwmSetWindowAttribute,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GWL_EXSTYLE, GetWindowLongPtrW, HWND_BOTTOM, HWND_TOP, HWND_TOPMOST, IsWindow, SW_HIDE,
    SW_SHOWNA, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
    SWP_SHOWWINDOW, SetWindowLongPtrW, SetWindowPos, ShowWindow, WS_EX_APPWINDOW,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellWindowRole {
    TopBar { full_width: bool },
    Dock,
    Popover,
    HoverPopover,
    DesktopWidget,
    Toast,
    Decorator,
    Settings,
}

/// Applies Win32 extended styles and Windows 11 DWM attributes to a shell window HWND.
pub fn configure_shell_window(
    hwnd_raw: u64,
    role: ShellWindowRole,
    dark_mode: bool,
    frosted: bool,
) {
    if hwnd_raw == 0 {
        return;
    }
    let hwnd = HWND(hwnd_raw as *mut c_void);
    if !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
        return;
    }

    let current_ex = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) };
    let mut new_ex = match role {
        ShellWindowRole::TopBar { .. }
        | ShellWindowRole::Dock
        | ShellWindowRole::HoverPopover
        | ShellWindowRole::DesktopWidget
        | ShellWindowRole::Toast
        | ShellWindowRole::Decorator => {
            (current_ex & !(WS_EX_APPWINDOW.0 as isize))
                | (WS_EX_TOOLWINDOW.0 as isize)
                | (WS_EX_NOACTIVATE.0 as isize)
        }
        ShellWindowRole::Popover => {
            (current_ex & !(WS_EX_APPWINDOW.0 as isize)) | (WS_EX_TOOLWINDOW.0 as isize)
        }
        ShellWindowRole::Settings => current_ex,
    };

    if role == ShellWindowRole::Popover {
        new_ex &= !(WS_EX_NOACTIVATE.0 as isize);
    }

    if new_ex != current_ex {
        unsafe {
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, new_ex);
        }
    }

    let exclude_peek: i32 = 1;
    let dark_val: i32 = i32::from(dark_mode);
    let corner_pref: DWM_WINDOW_CORNER_PREFERENCE = match role {
        ShellWindowRole::Settings => DWMWCP_ROUND,
        ShellWindowRole::TopBar { .. }
        | ShellWindowRole::Dock
        | ShellWindowRole::Popover
        | ShellWindowRole::HoverPopover
        | ShellWindowRole::DesktopWidget
        | ShellWindowRole::Toast
        | ShellWindowRole::Decorator => DWMWCP_DONOTROUND,
    };
    let backdrop: DWM_SYSTEMBACKDROP_TYPE = match role {
        // Apply native Windows 11 Acrylic only to tight-bound surfaces without transparent shadow gutters.
        ShellWindowRole::TopBar { full_width: true } | ShellWindowRole::Settings if frosted => {
            DWMSBT_TRANSIENTWINDOW
        }
        _ => DWMSBT_NONE,
    };

    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_EXCLUDED_FROM_PEEK,
            (&raw const exclude_peek).cast(),
            size_of::<i32>() as u32,
        );
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            (&raw const dark_val).cast(),
            size_of::<i32>() as u32,
        );
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            (&raw const corner_pref).cast(),
            size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32,
        );
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_SYSTEMBACKDROP_TYPE,
            (&raw const backdrop).cast(),
            size_of::<DWM_SYSTEMBACKDROP_TYPE>() as u32,
        );

        // Disable DWM's own show/hide transition fades so shell surfaces appear instantly.
        let transitions_disabled: i32 = 1;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_TRANSITIONS_FORCEDISABLED,
            (&raw const transitions_disabled).cast(),
            size_of::<i32>() as u32,
        );

        let _ = SetWindowPos(
            hwnd,
            None,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );
    }
}

/// Repositions and resizes an HWND in exact physical pixels without stealing foreground focus.
pub fn reposition_window_physical(hwnd_raw: u64, rect: PhysicalRect, topmost: bool) {
    if hwnd_raw == 0 {
        return;
    }
    let hwnd = HWND(hwnd_raw as *mut c_void);
    if !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
        return;
    }

    let z_order = if topmost { HWND_TOPMOST } else { HWND_TOP };
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            Some(z_order),
            rect.x,
            rect.y,
            rect.width.max(1),
            rect.height.max(1),
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
    }
}

/// Positions a desktop-level widget window at `HWND_BOTTOM` so user apps always sit above it.
pub fn position_window_bottom(hwnd_raw: u64, rect: PhysicalRect) {
    if hwnd_raw == 0 {
        return;
    }
    let hwnd = HWND(hwnd_raw as *mut c_void);
    if !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
        return;
    }
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_BOTTOM),
            rect.x,
            rect.y,
            rect.width.max(1),
            rect.height.max(1),
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
    }
}

/// Instantaneously shows (`SW_SHOWNA`) or hides (`SW_HIDE`) an existing shell HWND without destroying its swapchain.
pub fn set_window_visible(hwnd_raw: u64, visible: bool) {
    if hwnd_raw == 0 {
        return;
    }
    let hwnd = HWND(hwnd_raw as *mut c_void);
    if !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
        return;
    }
    unsafe {
        let _ = ShowWindow(hwnd, if visible { SW_SHOWNA } else { SW_HIDE });
    }
}
