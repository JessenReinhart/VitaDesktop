use crate::monitors::enumerate_monitors;
use directories::BaseDirs;
use std::fs;
use std::mem::size_of;
use std::path::PathBuf;
use vita_core::{MonitorInfo, PhysicalRect};
use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::Graphics::Dwm::{DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute};
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetWindowRect, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
    IsZoomed, SPI_SETWORKAREA, SPIF_SENDCHANGE, SWP_ASYNCWINDOWPOS, SWP_NOACTIVATE, SWP_NOZORDER,
    SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SetWindowPos, SystemParametersInfoW,
};
use windows::core::BOOL;

pub type AppBarGuard = WorkAreaGuard;

fn backup_file_path() -> PathBuf {
    BaseDirs::new()
        .map(|b| b.data_local_dir().join("VitaDesktop").join("workareas.bak"))
        .unwrap_or_else(|| PathBuf::from("vita-workareas.bak"))
}

fn save_workarea_backup(rects: &[PhysicalRect]) {
    let path = backup_file_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let body: String = rects
        .iter()
        .map(|r| format!("{},{},{},{}", r.x, r.y, r.width, r.height))
        .collect::<Vec<_>>()
        .join("\n");
    let _ = fs::write(path, body);
}

fn load_workarea_backup() -> Option<Vec<PhysicalRect>> {
    let raw = fs::read_to_string(backup_file_path()).ok()?;
    let mut rects = Vec::new();
    for line in raw.lines() {
        let parts: Vec<i32> = line
            .split(',')
            .filter_map(|s| s.trim().parse::<i32>().ok())
            .collect();
        if parts.len() == 4 && parts[2] > 0 && parts[3] > 0 {
            rects.push(PhysicalRect {
                x: parts[0],
                y: parts[1],
                width: parts[2],
                height: parts[3],
            });
        }
    }
    (!rects.is_empty()).then_some(rects)
}

fn clear_workarea_backup() {
    let _ = fs::remove_file(backup_file_path());
}

fn set_monitor_workarea(rect: PhysicalRect, broadcast: bool) {
    let mut rc = RECT {
        left: rect.x,
        top: rect.y,
        right: rect.x + rect.width,
        bottom: rect.y + rect.height,
    };
    let flags = if broadcast {
        SPIF_SENDCHANGE
    } else {
        SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0)
    };
    unsafe {
        let _ = SystemParametersInfoW(
            SPI_SETWORKAREA,
            0,
            Some((&raw mut rc).cast()),
            flags,
        );
    }
}

/// Restores saved pre-VitaDesktop per-monitor work areas if a crash backup file exists.
pub fn restore_saved_workareas() {
    if let Some(rects) = load_workarea_backup() {
        set_monitor_workareas(&rects, true);
        clear_workarea_backup();
    }
}


/// Returns the work-area top edge (physical px, relative to the monitor top) for a Top Bar of
/// `top_px`, optionally tucked upward by `tuck_px` so a maximized window's own title bar hides
/// behind the Top Bar. Keeps at least 2px so the window never slides above the screen edge.
pub fn tucked_top_px(top_px: i32, tuck_px: i32, tucked: bool) -> i32 {
    if tucked {
        top_px - tuck_px.clamp(0, (top_px - 2).max(0))
    } else {
        top_px
    }
}

fn set_monitor_workareas(rects: &[PhysicalRect], broadcast_final: bool) {
    let last = rects.len().saturating_sub(1);
    for (idx, &rect) in rects.iter().enumerate() {
        set_monitor_workarea(rect, broadcast_final && idx == last);
    }
}

const MAX_DRIFT_RETRIES: u8 = 2;

fn should_retry_drift(current: PhysicalRect, target: PhysicalRect, retries: &mut u8) -> bool {
    if current == target {
        *retries = 0;
        return false;
    }
    if *retries >= MAX_DRIFT_RETRIES {
        return false;
    }
    *retries += 1;
    true
}

/// Manages GNOME-style per-monitor `SPI_SETWORKAREA` reservations so maximized windows
/// clear the Vita Top Bar and reclaim native taskbar pixels across all displays.
///
/// When a monitor is flagged as "tucked", the reserved top edge shrinks by the app title bar
/// height. The system then places maximized windows so their title bar sits underneath the
/// (opaque) Top Bar, without fighting the application over window bounds.
pub struct WorkAreaGuard {
    active: bool,
    original_workareas: Vec<PhysicalRect>,
    target_workareas: Vec<PhysicalRect>,
    drift_retries: Vec<u8>,
    tuck_flags: Vec<bool>,
    tuck_height_logical: u32,
}

impl WorkAreaGuard {
    pub fn new(monitors: &[MonitorInfo], active: bool) -> Self {
        let saved = load_workarea_backup().filter(|bak| {
            bak.len() == monitors.len()
                && monitors
                    .iter()
                    .zip(bak.iter())
                    .all(|(m, b)| m.physical_bounds.intersects(*b))
        });

        let original_workareas = saved
            .unwrap_or_else(|| monitors.iter().map(|m| m.physical_workarea).collect());

        Self {
            active,
            original_workareas,
            target_workareas: Vec::new(),
            drift_retries: Vec::new(),
            tuck_flags: Vec::new(),
            tuck_height_logical: 34,
        }
    }

    /// Sets which monitors should hide the maximized app title bar behind the Top Bar.
    pub fn set_tucked_monitors(&mut self, flags: Vec<bool>, tuck_height_logical: u32) {
        self.tuck_flags = flags;
        self.tuck_height_logical = tuck_height_logical;
    }

    pub fn apply(
        &mut self,
        monitors: &[MonitorInfo],
        top_bar_height_logical: u32,
        bottom_reserve_logical: u32,
        hide_native_taskbar: bool,
    ) {
        if monitors.is_empty() {
            return;
        }
        if self.original_workareas.len() != monitors.len() {
            self.original_workareas = monitors.iter().map(|m| m.physical_workarea).collect();
        }

        self.target_workareas.clear();
        for (idx, mon) in monitors.iter().enumerate() {
            let b = mon.physical_bounds;
            let orig = self
                .original_workareas
                .get(idx)
                .copied()
                .unwrap_or(mon.physical_workarea);
            let top_px = (top_bar_height_logical as f32 * mon.dpi_scale).round() as i32;
            let tuck_px = (self.tuck_height_logical as f32 * mon.dpi_scale).round() as i32;
            let tucked = self.tuck_flags.get(idx).copied().unwrap_or(false);
            let bottom_px = (bottom_reserve_logical as f32 * mon.dpi_scale).round() as i32;

            let target_top = b.y + tucked_top_px(top_px, tuck_px, tucked);
            let target_bottom = if hide_native_taskbar {
                b.bottom() - bottom_px
            } else {
                let taskbar_top = orig.bottom();
                taskbar_top.min(b.bottom() - bottom_px)
            };

            self.target_workareas.push(PhysicalRect {
                x: b.x,
                y: target_top,
                width: b.width,
                height: (target_bottom - target_top).max(200),
            });
        }
        self.drift_retries.clear();
        self.drift_retries.resize(self.target_workareas.len(), 0);

        if self.active {
            save_workarea_backup(&self.original_workareas);
            set_monitor_workareas(&self.target_workareas, false);
            nudge_maximized_windows(&self.target_workareas);
        }
    }

    /// Re-applies target work areas on drifted monitors, capped per monitor to prevent feedback loops.
    pub fn ensure_applied(&mut self) {
        if !self.active || self.target_workareas.is_empty() {
            return;
        }
        let current = enumerate_monitors();
        if current.len() != self.target_workareas.len() {
            return;
        }
        if self.drift_retries.len() != self.target_workareas.len() {
            self.drift_retries.resize(self.target_workareas.len(), 0);
        }
        let mut drifted = Vec::new();
        for ((mon, &target), retries) in current
            .iter()
            .zip(self.target_workareas.iter())
            .zip(self.drift_retries.iter_mut())
        {
            if should_retry_drift(mon.physical_workarea, target, retries) {
                drifted.push(target);
            }
        }
        if !drifted.is_empty() {
            set_monitor_workareas(&drifted, false);
            nudge_maximized_windows(&drifted);
        }
    }

    pub fn restore_now(&mut self) {
        if !self.active {
            return;
        }
        set_monitor_workareas(&self.original_workareas, true);
        if !self.original_workareas.is_empty() {
            nudge_maximized_windows(&self.original_workareas);
        }
        clear_workarea_backup();
    }
}

impl Drop for WorkAreaGuard {
    fn drop(&mut self) {
        self.restore_now();
    }
}

struct NudgeCtx<'a> {
    own_pid: u32,
    workareas: &'a [PhysicalRect],
}

unsafe extern "system" fn nudge_maximized_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let ctx = unsafe { &*(lparam.0 as *const NudgeCtx<'_>) };
    if !unsafe { IsWindowVisible(hwnd) }.as_bool()
        || unsafe { IsIconic(hwnd) }.as_bool()
        || !unsafe { IsZoomed(hwnd) }.as_bool()
    {
        return BOOL(1);
    }

    let mut pid = 0u32;
    unsafe {
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
    }
    if pid == 0 || pid == ctx.own_pid {
        return BOOL(1);
    }

    let mut cls = [0u16; 64];
    let clen = unsafe { GetClassNameW(hwnd, &mut cls) };
    let cname = String::from_utf16_lossy(&cls[..clen.max(0) as usize]);
    if matches!(
        cname.as_str(),
        "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd"
    ) {
        return BOOL(1);
    }

    let mut win_rc = RECT::default();
    if unsafe { GetWindowRect(hwnd, &mut win_rc) }.is_err() {
        return BOOL(1);
    }
    let mut ext_rc = RECT::default();
    let has_ext = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            (&raw mut ext_rc).cast(),
            size_of::<RECT>() as u32,
        )
    }
    .is_ok();
    let visible = if has_ext { ext_rc } else { win_rc };

    let cx = (visible.left + visible.right) / 2;
    let cy = (visible.top + visible.bottom) / 2;
    let Some(target) = ctx
        .workareas
        .iter()
        .find(|w| cx >= w.x && cx < w.right() && cy >= w.y - 120 && cy < w.bottom() + 120)
    else {
        return BOOL(1);
    };

    if (visible.top - target.y).abs() <= 4 && (visible.bottom - target.bottom()).abs() <= 4 {
        return BOOL(1);
    }

    // Preserve invisible DWM resize-border insets on maximized Win32 windows so edges stay flush.
    let pad_l = (visible.left - win_rc.left).max(0);
    let pad_t = (visible.top - win_rc.top).max(0);
    let pad_r = (win_rc.right - visible.right).max(0);
    let pad_b = (win_rc.bottom - visible.bottom).max(0);

    unsafe {
        let _ = SetWindowPos(
            hwnd,
            None,
            target.x - pad_l,
            target.y - pad_t,
            target.width + pad_l + pad_r,
            target.height + pad_t + pad_b,
            SWP_NOZORDER | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS,
        );
    }

    BOOL(1)
}

fn nudge_maximized_windows(workareas: &[PhysicalRect]) {
    if workareas.is_empty() {
        return;
    }
    let ctx = NudgeCtx {
        own_pid: unsafe { GetCurrentProcessId() },
        workareas,
    };
    let ptr = &ctx as *const NudgeCtx<'_> as isize;
    unsafe {
        let _ = EnumWindows(Some(nudge_maximized_proc), LPARAM(ptr));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tucked_top_moves_up_but_never_above_screen() {
        // 46px Top Bar, 34px title bar -> app starts at y = 12.
        assert_eq!(tucked_top_px(46, 34, true), 12);
        // Not tucked -> unchanged.
        assert_eq!(tucked_top_px(46, 34, false), 46);
        // A huge title bar is clamped so the window keeps a 2px margin from the top.
        assert_eq!(tucked_top_px(46, 200, true), 2);
        // 150% DPI.
        assert_eq!(tucked_top_px(69, 51, true), 18);
    }

    #[test]
    fn drift_retry_budget_caps_oscillation_and_resets_on_match() {
        let target = PhysicalRect {
            x: 1920,
            y: 34,
            width: 1920,
            height: 1046,
        };
        let drifted = PhysicalRect {
            x: 1920,
            y: 0,
            width: 1920,
            height: 1032,
        };
        let mut retries = 0u8;
        assert!(should_retry_drift(drifted, target, &mut retries));
        assert_eq!(retries, 1);
        assert!(should_retry_drift(drifted, target, &mut retries));
        assert_eq!(retries, 2);
        // Once budget is exhausted, persistent Explorer resistance no longer triggers oscillations.
        assert!(!should_retry_drift(drifted, target, &mut retries));
        assert_eq!(retries, 2);

        // Matching resets the budget so genuine later drift gets fresh retries.
        assert!(!should_retry_drift(target, target, &mut retries));
        assert_eq!(retries, 0);
        assert!(should_retry_drift(drifted, target, &mut retries));
        assert_eq!(retries, 1);
    }
}
