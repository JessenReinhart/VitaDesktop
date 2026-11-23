use std::mem::size_of;
use vita_core::{MonitorInfo, PhysicalRect};
use windows::Win32::Foundation::{LPARAM, POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFOEXW,
};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetSystemMetrics, MONITORINFOF_PRIMARY, SM_CXSCREEN, SM_CYSCREEN,
};
use windows::core::{BOOL, w};

unsafe extern "system" fn monitor_enum_proc(
    hmonitor: HMONITOR,
    _hdc: HDC,
    _rc: *mut RECT,
    lparam: LPARAM,
) -> BOOL {
    let list = unsafe { &mut *(lparam.0 as *mut Vec<MonitorInfo>) };
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;

    if unsafe { GetMonitorInfoW(hmonitor, &mut info.monitorInfo) }.as_bool() {
        let mut dpi_x: u32 = 96;
        let mut dpi_y: u32 = 96;
        let _ = unsafe { GetDpiForMonitor(hmonitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) };
        let dpi_scale = (dpi_x as f32 / 96.0).max(1.0);

        let rc = info.monitorInfo.rcMonitor;
        let wa = info.monitorInfo.rcWork;
        let name_end = info
            .szDevice
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(info.szDevice.len());
        let name = String::from_utf16_lossy(&info.szDevice[..name_end]);

        list.push(MonitorInfo {
            name,
            is_primary: (info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY) != 0,
            physical_bounds: PhysicalRect {
                x: rc.left,
                y: rc.top,
                width: rc.right - rc.left,
                height: rc.bottom - rc.top,
            },
            physical_workarea: PhysicalRect {
                x: wa.left,
                y: wa.top,
                width: wa.right - wa.left,
                height: wa.bottom - wa.top,
            },
            dpi_scale,
        });
    }

    BOOL(1)
}

/// Enumerates all active physical monitors along with their Per-Monitor DPI scales.
pub fn enumerate_monitors() -> Vec<MonitorInfo> {
    let mut monitors = Vec::new();
    let ptr = &mut monitors as *mut Vec<MonitorInfo> as isize;
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(monitor_enum_proc), LPARAM(ptr));
    }
    monitors
}

/// Returns the primary display monitor (or a fallback desktop rect if enumeration fails).
pub fn primary_monitor() -> MonitorInfo {
    let monitors = enumerate_monitors();
    if let Some(primary) = monitors.into_iter().find(|m| m.is_primary) {
        return primary;
    }
    let w = unsafe { GetSystemMetrics(SM_CXSCREEN) }.max(1280);
    let h = unsafe { GetSystemMetrics(SM_CYSCREEN) }.max(720);
    let rect = PhysicalRect {
        x: 0,
        y: 0,
        width: w,
        height: h,
    };
    MonitorInfo {
        name: "PRIMARY".into(),
        is_primary: true,
        physical_bounds: rect,
        physical_workarea: rect,
        dpi_scale: 1.0,
    }
}

/// Queries Windows Personalize registry setting (`AppsUseLightTheme == 0`).
pub fn system_prefers_dark() -> bool {
    let mut value: u32 = 1;
    let mut size = size_of::<u32>() as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some((&mut value as *mut u32).cast()),
            Some(&mut size),
        )
    };
    status.is_ok() && value == 0
}

/// Returns current physical cursor coordinates in virtual-screen pixels.
pub fn cursor_position() -> Option<(i32, i32)> {
    let mut pt = POINT::default();
    if unsafe { GetCursorPos(&mut pt) }.is_ok() {
        Some((pt.x, pt.y))
    } else {
        None
    }
}

/// Returns index of the primary monitor inside `monitors` (or `0` if none marked).
pub fn primary_monitor_index(monitors: &[MonitorInfo]) -> usize {
    monitors.iter().position(|m| m.is_primary).unwrap_or(0)
}

/// Returns index of the monitor containing the cursor, falling back to primary monitor.
pub fn monitor_index_from_cursor(monitors: &[MonitorInfo]) -> usize {
    if monitors.is_empty() {
        return 0;
    }
    if let Some((x, y)) = cursor_position()
        && let Some(idx) = monitors.iter().position(|m| m.physical_bounds.contains_point(x, y))
    {
        return idx;
    }
    primary_monitor_index(monitors)
}

/// Returns index of the monitor holding the center or largest overlap of `rect`.
pub fn monitor_index_for_rect(monitors: &[MonitorInfo], rect: PhysicalRect) -> usize {
    if monitors.is_empty() {
        return 0;
    }
    let cx = rect.x + rect.width / 2;
    let cy = rect.y + rect.height / 2;
    if let Some(idx) = monitors.iter().position(|m| m.physical_bounds.contains_point(cx, cy)) {
        return idx;
    }
    monitors
        .iter()
        .enumerate()
        .max_by_key(|(_, m)| intersection_area(m.physical_bounds, rect))
        .map_or(0, |(idx, _)| idx)
}

fn intersection_area(a: PhysicalRect, b: PhysicalRect) -> i64 {
    let x1 = a.x.max(b.x);
    let y1 = a.y.max(b.y);
    let x2 = a.right().min(b.right());
    let y2 = a.bottom().min(b.bottom());
    let w = (x2 - x1).max(0) as i64;
    let h = (y2 - y1).max(0) as i64;
    w * h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primary_monitor_has_positive_dimensions_and_dpi() {
        let monitors = enumerate_monitors();
        assert!(!monitors.is_empty(), "Expected at least one monitor");
        let primary = primary_monitor();
        assert!(primary.physical_bounds.width > 0);
        assert!(primary.physical_bounds.height > 0);
        assert!(primary.dpi_scale >= 1.0);
    }
}
