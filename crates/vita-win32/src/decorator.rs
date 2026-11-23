use crate::monitors::monitor_index_for_rect;
use crate::windows::{cached_window_icon, process_exe_info};
use std::cell::RefCell;
use std::collections::HashSet;
use std::ffi::c_void;
use std::mem::size_of;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::UnboundedSender;
use vita_core::{DecoratedWindowState, DecoratorMode, MonitorInfo, PhysicalRect, RgbaColor};
use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DWMWA_BORDER_COLOR, DWMWA_CAPTION_COLOR, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS,
    DWMWA_TEXT_COLOR, DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE,
    DWMWCP_ROUND, DwmGetWindowAttribute, DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{CLR_INVALID, ClientToScreen, GetDC, GetPixel, ReleaseDC};
use windows::Win32::System::Threading::{GetCurrentProcessId, GetCurrentThreadId};
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GW_OWNER, GWL_EXSTYLE, GWL_STYLE, GetClassNameW, GetForegroundWindow,
    GetMessageW, GetWindow, GetWindowLongPtrW, GetWindowRect, GetWindowTextLengthW,
    GetWindowTextW, GetWindowThreadProcessId, HWND_NOTOPMOST, HWND_TOPMOST, IsIconic, IsWindow,
    IsWindowVisible, IsZoomed, KillTimer, MSG, PM_NOREMOVE, PeekMessageW, PostMessageW,
    PostThreadMessageW, SC_MOVE, SW_MAXIMIZE, SW_MINIMIZE, SW_RESTORE, SWP_NOACTIVATE, SWP_NOMOVE,
    SWP_NOSIZE, SetForegroundWindow, SetTimer, SetWindowPos, ShowWindow, TranslateMessage, WM_QUIT,
    WM_SYSCOMMAND, WM_TIMER, WM_USER, WS_CAPTION, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_THICKFRAME,
};

const EVENT_SYSTEM_FOREGROUND: u32 = 0x0003;
const EVENT_SYSTEM_MOVESIZEEND: u32 = 0x000B;
const EVENT_SYSTEM_MINIMIZESTART: u32 = 0x0016;
const EVENT_SYSTEM_MINIMIZEEND: u32 = 0x0017;
const EVENT_OBJECT_LOCATIONCHANGE: u32 = 0x800B;
const EVENT_OBJECT_NAMECHANGE: u32 = 0x800C;
const EVENT_OBJECT_CLOAKED: u32 = 0x8017;
const EVENT_OBJECT_UNCLOAKED: u32 = 0x8018;
const WINEVENT_OUTOFCONTEXT: u32 = 0x0000;
const WINEVENT_SKIPOWNPROCESS: u32 = 0x0002;
const OBJID_WINDOW: i32 = 0;

const DWMWA_COLOR_DEFAULT: u32 = 0xFFFF_FFFF;

pub const fn rgb_to_colorref(r: f32, g: f32, b: f32) -> u32 {
    let rb = (r.clamp(0.0, 1.0) * 255.0) as u32;
    let gb = (g.clamp(0.0, 1.0) * 255.0) as u32;
    let bb = (b.clamp(0.0, 1.0) * 255.0) as u32;
    (bb << 16) | (gb << 8) | rb
}

/// Inspects the current Windows foreground window and returns decorator metadata if it is an
/// eligible external user application window.
pub fn inspect_foreground_window(monitors: &[MonitorInfo]) -> Option<DecoratedWindowState> {
    let hwnd = unsafe { GetForegroundWindow() };
    inspect_hwnd(hwnd, monitors)
}

pub fn inspect_hwnd(hwnd: HWND, monitors: &[MonitorInfo]) -> Option<DecoratedWindowState> {
    if hwnd.0.is_null() || !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
        return None;
    }
    if !unsafe { IsWindowVisible(hwnd) }.as_bool() || unsafe { IsIconic(hwnd) }.as_bool() {
        return None;
    }

    let own_pid = unsafe { GetCurrentProcessId() };
    let mut pid = 0u32;
    unsafe {
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
    }
    if pid == 0 || pid == own_pid {
        return None;
    }

    let mut cloaked = 0u32;
    let cloaked_ok = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            (&raw mut cloaked).cast(),
            size_of::<u32>() as u32,
        )
    };
    if cloaked_ok.is_ok() && cloaked != 0 {
        return None;
    }

    let ex_style = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32;
    let is_tool = (ex_style & WS_EX_TOOLWINDOW.0) != 0;
    let is_app = (ex_style & WS_EX_APPWINDOW.0) != 0;
    if is_tool && !is_app {
        return None;
    }
    let owner = unsafe { GetWindow(hwnd, GW_OWNER) };
    if owner.is_ok_and(|o| !o.0.is_null()) && !is_app {
        return None;
    }

    let mut cls = [0u16; 64];
    let clen = unsafe { GetClassNameW(hwnd, &mut cls) };
    let cname = String::from_utf16_lossy(&cls[..clen.max(0) as usize]);
    if matches!(
        cname.as_str(),
        "Progman"
            | "WorkerW"
            | "Shell_TrayWnd"
            | "Shell_SecondaryTrayWnd"
            | "Windows.UI.Core.CoreWindow"
            | "MultitaskingViewFrame"
            | "XamlExplorerHostIslandWindow"
            | "ForegroundStaging"
    ) {
        return None;
    }

    let style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) } as u32;
    let has_caption = (style & WS_CAPTION.0) == WS_CAPTION.0;
    let has_thickframe = (style & WS_THICKFRAME.0) != 0;
    if !has_caption && !has_thickframe {
        return None;
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
    let vis_rc = if has_ext && ext_rc.right > ext_rc.left && ext_rc.bottom > ext_rc.top {
        ext_rc
    } else {
        let mut r = RECT::default();
        if unsafe { GetWindowRect(hwnd, &mut r) }.is_err() {
            return None;
        }
        r
    };

    let frame_rect = PhysicalRect {
        x: vis_rc.left,
        y: vis_rc.top,
        width: vis_rc.right - vis_rc.left,
        height: vis_rc.bottom - vis_rc.top,
    };
    if frame_rect.width < 180 || frame_rect.height < 90 {
        return None;
    }

    let title_len = unsafe { GetWindowTextLengthW(hwnd) };
    if title_len <= 0 {
        return None;
    }
    let mut tbuf = vec![0u16; (title_len + 1) as usize];
    let tlen = unsafe { GetWindowTextW(hwnd, &mut tbuf) };
    if tlen <= 0 {
        return None;
    }
    let title = String::from_utf16_lossy(&tbuf[..tlen as usize]);

    let monitor_idx = monitor_index_for_rect(monitors, frame_rect);
    let mon = monitors.get(monitor_idx);
    let dpi_scale = mon.map_or(1.0, |m| m.dpi_scale);
    let is_maximized = unsafe { IsZoomed(hwnd) }.as_bool();

    let is_fullscreen = if let Some(m) = mon {
        let b = m.physical_bounds;
        !is_maximized
            && frame_rect.x <= b.x + 2
            && frame_rect.y <= b.y + 2
            && frame_rect.right() >= b.right() - 2
            && frame_rect.bottom() >= b.bottom() - 2
    } else {
        false
    };

    let mut pt = POINT { x: 0, y: 0 };
    let nc_top = if unsafe { ClientToScreen(hwnd, &mut pt) }.as_bool() {
        pt.y - frame_rect.y
    } else {
        0
    };
    let mode = if nc_top >= 24 {
        DecoratorMode::FullHeader
    } else {
        DecoratorMode::ControlsOnly
    };

    let (exe_path, exe_name) = process_exe_info(pid);
    let icon = cached_window_icon(hwnd, &exe_path);
    let titlebar_bg = sample_titlebar_bg(frame_rect, dpi_scale);

    Some(DecoratedWindowState {
        hwnd: hwnd.0 as isize,
        title,
        exe_name,
        frame_rect,
        dpi_scale,
        is_maximized,
        is_fullscreen,
        mode,
        monitor_idx,
        titlebar_bg,
        icon,
    })
}

fn sample_titlebar_bg(frame_rect: PhysicalRect, dpi_scale: f32) -> Option<RgbaColor> {
    let dpi = dpi_scale.max(1.0);
    let sample_x = frame_rect.right() - (146.0 * dpi).round() as i32;
    let sample_y = frame_rect.y + (6.0 * dpi).round() as i32;
    if sample_x <= frame_rect.x + 16 || sample_y >= frame_rect.bottom() {
        return None;
    }
    let hdc = unsafe { GetDC(None) };
    if hdc.is_invalid() {
        return None;
    }
    let raw = unsafe { GetPixel(hdc, sample_x, sample_y) }.0;
    unsafe {
        let _ = ReleaseDC(None, hdc);
    }
    if raw == CLR_INVALID {
        return None;
    }
    // Quantize by 2 to prevent 1-bit DWM noise from triggering redundant state updates.
    let r = ((raw & 0xFF) & !1) as f32 / 255.0;
    let g = (((raw >> 8) & 0xFF) & !1) as f32 / 255.0;
    let b = (((raw >> 16) & 0xFF) & !1) as f32 / 255.0;
    Some(RgbaColor::rgba(r, g, b, 1.0))
}

pub fn minimize_window(hwnd_raw: isize) {
    let hwnd = HWND(hwnd_raw as *mut c_void);
    if unsafe { IsWindow(Some(hwnd)) }.as_bool() {
        unsafe {
            let _ = ShowWindow(hwnd, SW_MINIMIZE);
        }
    }
}

pub fn toggle_maximize_window(hwnd_raw: isize) {
    let hwnd = HWND(hwnd_raw as *mut c_void);
    if unsafe { IsWindow(Some(hwnd)) }.as_bool() {
        unsafe {
            let cmd = if IsZoomed(hwnd).as_bool() {
                SW_RESTORE
            } else {
                SW_MAXIMIZE
            };
            let _ = ShowWindow(hwnd, cmd);
        }
    }
}

pub fn is_window_always_on_top(hwnd_raw: isize) -> bool {
    let hwnd = HWND(hwnd_raw as *mut c_void);
    if !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
        return false;
    }
    let ex = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32;
    (ex & WS_EX_TOPMOST.0) != 0
}

pub fn toggle_always_on_top(hwnd_raw: isize) {
    let hwnd = HWND(hwnd_raw as *mut c_void);
    if !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
        return;
    }
    let target_z = if is_window_always_on_top(hwnd_raw) {
        HWND_NOTOPMOST
    } else {
        HWND_TOPMOST
    };
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            Some(target_z),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
    }
}

/// Initiates native Windows caption drag (including Snap / Snap Layouts) on `hwnd_raw`.
pub fn begin_drag_window(hwnd_raw: isize) {
    let hwnd = HWND(hwnd_raw as *mut c_void);
    if !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
        return;
    }
    unsafe {
        let _ = ReleaseCapture();
        let _ = SetForegroundWindow(hwnd);
        // HTCAPTION == 2; SC_MOVE + HTCAPTION starts native title-bar move loop.
        let _ = PostMessageW(Some(hwnd), WM_SYSCOMMAND, WPARAM((SC_MOVE as usize) + 2), LPARAM(0));
    }
}

/// Synchronizes Windows 11 DWM border, caption, dark-mode, and rounded-corner attributes
/// for external windows and cleanly restores defaults when deactivated.
#[derive(Default)]
pub struct DwmWindowThemer {
    themed: HashSet<isize>,
}

impl DwmWindowThemer {
    pub fn apply(
        &mut self,
        hwnd_raw: isize,
        dark_mode: bool,
        border_colorref: u32,
        caption_colorref: u32,
        text_colorref: u32,
    ) {
        if hwnd_raw == 0 {
            return;
        }
        let hwnd = HWND(hwnd_raw as *mut c_void);
        if !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
            return;
        }
        self.themed.insert(hwnd_raw);

        let dark_val: i32 = i32::from(dark_mode);
        let corner = DWMWCP_ROUND;
        unsafe {
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_WINDOW_CORNER_PREFERENCE,
                (&raw const corner).cast(),
                size_of::<windows::Win32::Graphics::Dwm::DWM_WINDOW_CORNER_PREFERENCE>() as u32,
            );
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_USE_IMMERSIVE_DARK_MODE,
                (&raw const dark_val).cast(),
                size_of::<i32>() as u32,
            );
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_BORDER_COLOR,
                (&raw const border_colorref).cast(),
                size_of::<u32>() as u32,
            );
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_CAPTION_COLOR,
                (&raw const caption_colorref).cast(),
                size_of::<u32>() as u32,
            );
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_TEXT_COLOR,
                (&raw const text_colorref).cast(),
                size_of::<u32>() as u32,
            );
        }
    }

    pub fn restore_hwnd(&mut self, hwnd_raw: isize) {
        if !self.themed.remove(&hwnd_raw) {
            return;
        }
        reset_hwnd_dwm(hwnd_raw);
    }

    pub fn restore_all(&mut self) {
        for hwnd_raw in self.themed.drain() {
            reset_hwnd_dwm(hwnd_raw);
        }
    }
}

impl Drop for DwmWindowThemer {
    fn drop(&mut self) {
        self.restore_all();
    }
}

fn reset_hwnd_dwm(hwnd_raw: isize) {
    let hwnd = HWND(hwnd_raw as *mut c_void);
    if !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
        return;
    }
    let default_color = DWMWA_COLOR_DEFAULT;
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR,
            (&raw const default_color).cast(),
            size_of::<u32>() as u32,
        );
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_CAPTION_COLOR,
            (&raw const default_color).cast(),
            size_of::<u32>() as u32,
        );
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_TEXT_COLOR,
            (&raw const default_color).cast(),
            size_of::<u32>() as u32,
        );
    }
}

struct DecoHookCtx {
    tx: UnboundedSender<Option<DecoratedWindowState>>,
    monitors: Vec<MonitorInfo>,
    last_emit: Instant,
    pending_timer: Option<usize>,
    last_state: Option<DecoratedWindowState>,
}

thread_local! {
    static DECO_CTX: RefCell<Option<DecoHookCtx>> = const { RefCell::new(None) };
}

fn emit_foreground_state(force: bool) {
    DECO_CTX.with(|cell| {
        if let Some(ctx) = cell.borrow_mut().as_mut() {
            if force {
                ctx.monitors = crate::monitors::enumerate_monitors();
            }
            let next = inspect_foreground_window(&ctx.monitors);
            if force || next != ctx.last_state {
                ctx.last_state = next.clone();
                ctx.last_emit = Instant::now();
                let _ = ctx.tx.send(next);
            }
        }
    });
}

unsafe extern "system" fn deco_timer_proc(_hwnd: HWND, _msg: u32, timer_id: usize, _time: u32) {
    unsafe {
        let _ = KillTimer(None, timer_id);
    }
    DECO_CTX.with(|cell| {
        if let Some(ctx) = cell.borrow_mut().as_mut() {
            ctx.pending_timer = None;
        }
    });
    emit_foreground_state(false);
}

unsafe extern "system" fn deco_event_proc(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    id_object: i32,
    id_child: i32,
    _thread_id: u32,
    _time: u32,
) {
    let relevant = matches!(
        event,
        EVENT_SYSTEM_FOREGROUND
            | EVENT_SYSTEM_MOVESIZEEND
            | EVENT_SYSTEM_MINIMIZESTART
            | EVENT_SYSTEM_MINIMIZEEND
            | EVENT_OBJECT_LOCATIONCHANGE
            | EVENT_OBJECT_NAMECHANGE
            | EVENT_OBJECT_CLOAKED
            | EVENT_OBJECT_UNCLOAKED
    );
    if !relevant {
        return;
    }

    let is_sys = matches!(
        event,
        EVENT_SYSTEM_FOREGROUND
            | EVENT_SYSTEM_MOVESIZEEND
            | EVENT_SYSTEM_MINIMIZESTART
            | EVENT_SYSTEM_MINIMIZEEND
    );
    if !is_sys && (id_object != OBJID_WINDOW || id_child != 0) {
        return;
    }
    if event == EVENT_OBJECT_LOCATIONCHANGE && unsafe { GetForegroundWindow() } != hwnd {
        return;
    }

    DECO_CTX.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(ctx) = borrow.as_mut() else {
            return;
        };
        let elapsed = ctx.last_emit.elapsed();
        let immediate = is_sys || elapsed >= Duration::from_millis(16);
        if immediate {
            if let Some(tid) = ctx.pending_timer.take() {
                unsafe {
                    let _ = KillTimer(None, tid);
                }
            }
            drop(borrow);
            emit_foreground_state(event == EVENT_SYSTEM_FOREGROUND);
        } else if ctx.pending_timer.is_none() {
            let tid = unsafe { SetTimer(None, 0, 18, Some(deco_timer_proc)) };
            if tid != 0 {
                ctx.pending_timer = Some(tid);
            }
        }
    });
}

/// Low-overhead WinEvent listener emitting foreground window bounds and chrome state at up to 60Hz.
pub struct DecoratorTracker {
    stop: Arc<AtomicBool>,
    thread_id: u32,
    thread: Option<JoinHandle<()>>,
}

impl DecoratorTracker {
    pub fn start(tx: UnboundedSender<Option<DecoratedWindowState>>) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = Arc::clone(&stop);
        let (tid_tx, tid_rx) = std::sync::mpsc::channel::<u32>();
        let thread = std::thread::Builder::new()
            .name("vita-decorator-hook".into())
            .spawn(move || {
                // Force Win32 message queue creation before publishing tid so Drop cannot lose WM_QUIT.
                let mut init_msg = MSG::default();
                unsafe {
                    let _ = PeekMessageW(&mut init_msg, None, WM_USER, WM_USER, PM_NOREMOVE);
                }
                let tid = unsafe { GetCurrentThreadId() };
                let _ = tid_tx.send(tid);
                if stop_thread.load(Ordering::Acquire) {
                    return;
                }

                let monitors = crate::monitors::enumerate_monitors();
                let initial = inspect_foreground_window(&monitors);
                let _ = tx.send(initial.clone());

                DECO_CTX.with(|cell| {
                    *cell.borrow_mut() = Some(DecoHookCtx {
                        tx,
                        monitors,
                        last_emit: Instant::now(),
                        pending_timer: None,
                        last_state: initial,
                    });
                });

                let hook_sys = unsafe {
                    SetWinEventHook(
                        EVENT_SYSTEM_FOREGROUND,
                        EVENT_SYSTEM_MINIMIZEEND,
                        None,
                        Some(deco_event_proc),
                        0,
                        0,
                        WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
                    )
                };
                let hook_obj = unsafe {
                    SetWinEventHook(
                        EVENT_OBJECT_LOCATIONCHANGE,
                        EVENT_OBJECT_UNCLOAKED,
                        None,
                        Some(deco_event_proc),
                        0,
                        0,
                        WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
                    )
                };
                // Safety fallback timer so state converges cleanly even if an elevated app misses an event.
                let poll_timer = unsafe { SetTimer(None, 0, 250, None) };

                if !stop_thread.load(Ordering::Acquire) {
                    let mut msg = MSG::default();
                    while !stop_thread.load(Ordering::Acquire)
                        && unsafe { GetMessageW(&mut msg, None, 0, 0) }.0 > 0
                    {
                        if msg.message == WM_TIMER {
                            emit_foreground_state(false);
                        }
                        unsafe {
                            let _ = TranslateMessage(&msg);
                            DispatchMessageW(&msg);
                        }
                    }
                }

                if poll_timer != 0 {
                    unsafe {
                        let _ = KillTimer(None, poll_timer);
                    }
                }
                if !hook_sys.is_invalid() {
                    unsafe {
                        let _ = UnhookWinEvent(hook_sys);
                    }
                }
                if !hook_obj.is_invalid() {
                    unsafe {
                        let _ = UnhookWinEvent(hook_obj);
                    }
                }
                DECO_CTX.with(|cell| {
                    *cell.borrow_mut() = None;
                });
            })
            .ok();

        let thread_id = tid_rx.recv().unwrap_or(0);
        Self {
            stop,
            thread_id,
            thread,
        }
    }
}

impl Drop for DecoratorTracker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if self.thread_id != 0 {
            unsafe {
                let _ = PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
            }
        }
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}
