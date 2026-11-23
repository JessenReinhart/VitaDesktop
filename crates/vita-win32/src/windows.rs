use crate::monitors::{enumerate_monitors, monitor_index_for_rect};
use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::mem::size_of;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::UnboundedSender;
use vita_core::{MonitorInfo, PhysicalRect, RgbaIcon, RunningWindow};
use windows::Win32::Foundation::{
    CloseHandle, HWND, LPARAM, RECT, SIZE, WPARAM,
};
use windows::Win32::Graphics::Dwm::{
    DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, DeleteObject, GetDC, GetDIBits,
    GetObjectW, HBITMAP, ReleaseDC,
};
use windows::Win32::Storage::FileSystem::SearchPathW;
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW,
};
use windows::Win32::System::Threading::{
    GetCurrentProcessId, GetCurrentThreadId, OpenProcess, PROCESS_NAME_FORMAT,
    PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::Shell::{
    IShellItemImageFactory, SHCreateItemFromParsingName, SHFILEINFOW, SHGFI_ICON,
    SHGFI_LARGEICON, SHGetFileInfoW, SIIGBF_BIGGERSIZEOK, SIIGBF_RESIZETOFIT, ShellExecuteW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DestroyIcon, DispatchMessageW, EnumWindows, GCLP_HICON, GW_OWNER, GWL_EXSTYLE,
    GetClassLongPtrW, GetClassNameW, GetForegroundWindow, GetIconInfo, GetMessageW,
    GetWindow, GetWindowLongPtrW, GetWindowRect, GetWindowTextLengthW, GetWindowTextW,
    GetWindowThreadProcessId, HICON, ICON_BIG, ICON_SMALL2, ICONINFO, IsIconic, IsWindow,
    IsWindowVisible, IsZoomed, KillTimer, MSG, PM_NOREMOVE, PeekMessageW, PostMessageW,
    PostThreadMessageW, SMTO_ABORTIFHUNG, SW_MINIMIZE, SW_RESTORE, SW_SHOWNORMAL,
    SendMessageTimeoutW, SetForegroundWindow, SetTimer, ShowWindow, TranslateMessage, WM_CLOSE,
    WM_GETICON, WM_QUIT, WM_USER, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
};
use windows::core::{BOOL, PCWSTR, PWSTR, w};

const EVENT_OBJECT_CREATE: u32 = 0x8000;
const EVENT_OBJECT_DESTROY: u32 = 0x8001;
const EVENT_OBJECT_NAMECHANGE: u32 = 0x800C;
const EVENT_OBJECT_CLOAKED: u32 = 0x8017;
const EVENT_OBJECT_UNCLOAKED: u32 = 0x8018;
const EVENT_SYSTEM_FOREGROUND: u32 = 0x0003;
const EVENT_SYSTEM_MINIMIZESTART: u32 = 0x0016;
const EVENT_SYSTEM_MINIMIZEEND: u32 = 0x0017;
const WINEVENT_OUTOFCONTEXT: u32 = 0x0000;
const WINEVENT_SKIPOWNPROCESS: u32 = 0x0002;
const OBJID_WINDOW: i32 = 0;

static ICON_CACHE: OnceLock<Mutex<HashMap<String, Option<RgbaIcon>>>> = OnceLock::new();

fn icon_cache() -> &'static Mutex<HashMap<String, Option<RgbaIcon>>> {
    ICON_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

struct HookThreadContext {
    tx: UnboundedSender<Vec<RunningWindow>>,
    pending_timer: Option<usize>,
    last_emit: Instant,
}

thread_local! {
    static HOOK_CTX: RefCell<Option<HookThreadContext>> = const { RefCell::new(None) };
}

unsafe extern "system" fn win_event_proc(
    _hook: HWINEVENTHOOK,
    event: u32,
    _hwnd: HWND,
    id_object: i32,
    id_child: i32,
    _thread_id: u32,
    _time: u32,
) {
    let is_system_event = matches!(
        event,
        EVENT_SYSTEM_FOREGROUND | EVENT_SYSTEM_MINIMIZESTART | EVENT_SYSTEM_MINIMIZEEND
    );
    let is_tracked_object_event = matches!(
        event,
        EVENT_OBJECT_CREATE
            | EVENT_OBJECT_DESTROY
            | EVENT_OBJECT_NAMECHANGE
            | EVENT_OBJECT_CLOAKED
            | EVENT_OBJECT_UNCLOAKED
    );
    if !is_system_event && (!is_tracked_object_event || id_object != OBJID_WINDOW || id_child != 0)
    {
        return;
    }

    HOOK_CTX.with(|cell| {
        if let Some(ctx) = cell.borrow_mut().as_mut()
            && ctx.pending_timer.is_none()
        {
            let timer_id = unsafe { SetTimer(None, 0, 35, Some(debounce_timer_proc)) };
            if timer_id != 0 {
                ctx.pending_timer = Some(timer_id);
            }
        }
    });
}

unsafe extern "system" fn debounce_timer_proc(_hwnd: HWND, _msg: u32, timer_id: usize, _time: u32) {
    unsafe {
        let _ = KillTimer(None, timer_id);
    }
    HOOK_CTX.with(|cell| {
        if let Some(ctx) = cell.borrow_mut().as_mut() {
            ctx.pending_timer = None;
            if ctx.last_emit.elapsed() >= Duration::from_millis(25) {
                ctx.last_emit = Instant::now();
                let snapshot = enumerate_running_windows();
                let _ = ctx.tx.send(snapshot);
            }
        }
    });
}

pub struct WindowTracker {
    stop: Arc<AtomicBool>,
    thread_id: u32,
    thread: Option<JoinHandle<()>>,
}

impl WindowTracker {
    pub fn start(tx: UnboundedSender<Vec<RunningWindow>>) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = Arc::clone(&stop);
        let (tid_tx, tid_rx) = std::sync::mpsc::channel::<u32>();

        let thread = std::thread::Builder::new()
            .name("vita-window-tracker".into())
            .spawn(move || {
                // Force Win32 message queue creation before publishing tid so Drop cannot lose WM_QUIT.
                let mut init_msg = MSG::default();
                unsafe {
                    let _ = PeekMessageW(&mut init_msg, None, WM_USER, WM_USER, PM_NOREMOVE);
                }
                let current_tid = unsafe { GetCurrentThreadId() };
                let _ = tid_tx.send(current_tid);
                if stop_thread.load(Ordering::Acquire) {
                    return;
                }

                // Emit initial snapshot immediately on startup.
                let _ = tx.send(enumerate_running_windows());

                HOOK_CTX.with(|cell| {
                    *cell.borrow_mut() = Some(HookThreadContext {
                        tx,
                        pending_timer: None,
                        last_emit: Instant::now(),
                    });
                });

                let flags = WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS;
                let hook_fg = unsafe {
                    SetWinEventHook(
                        EVENT_SYSTEM_FOREGROUND,
                        EVENT_SYSTEM_MINIMIZEEND,
                        None,
                        Some(win_event_proc),
                        0,
                        0,
                        flags,
                    )
                };
                let hook_obj = unsafe {
                    SetWinEventHook(
                        EVENT_OBJECT_CREATE,
                        EVENT_OBJECT_UNCLOAKED,
                        None,
                        Some(win_event_proc),
                        0,
                        0,
                        flags,
                    )
                };

                if !stop_thread.load(Ordering::Acquire) {
                    let mut msg = MSG::default();
                    while !stop_thread.load(Ordering::Acquire)
                        && unsafe { GetMessageW(&mut msg, None, 0, 0) }.0 > 0
                    {
                        unsafe {
                            let _ = TranslateMessage(&msg);
                            DispatchMessageW(&msg);
                        }
                    }
                }

                if !hook_fg.is_invalid() {
                    unsafe {
                        let _ = UnhookWinEvent(hook_fg);
                    }
                }
                if !hook_obj.is_invalid() {
                    unsafe {
                        let _ = UnhookWinEvent(hook_obj);
                    }
                }
                HOOK_CTX.with(|cell| {
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

impl Drop for WindowTracker {
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

struct EnumWindowsState {
    own_pid: u32,
    foreground_hwnd: isize,
    monitors: Vec<MonitorInfo>,
    windows: Vec<RunningWindow>,
}

unsafe extern "system" fn enum_window_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let state = unsafe { &mut *(lparam.0 as *mut EnumWindowsState) };
    if let Some(win) = inspect_window(hwnd, state.own_pid, state.foreground_hwnd, &state.monitors) {
        state.windows.push(win);
    }
    BOOL(1)
}

/// Enumerates all visible, uncloaked taskbar-eligible user windows on the current desktop.
pub fn enumerate_running_windows() -> Vec<RunningWindow> {
    let own_pid = unsafe { GetCurrentProcessId() };
    let fg = unsafe { GetForegroundWindow() };
    let monitors = enumerate_monitors();
    let mut state = EnumWindowsState {
        own_pid,
        foreground_hwnd: fg.0 as isize,
        monitors,
        windows: Vec::new(),
    };
    let ptr = &mut state as *mut EnumWindowsState as isize;
    unsafe {
        let _ = EnumWindows(Some(enum_window_proc), LPARAM(ptr));
    }
    state.windows
}

fn inspect_window(
    hwnd: HWND,
    own_pid: u32,
    foreground_hwnd: isize,
    monitors: &[MonitorInfo],
) -> Option<RunningWindow> {
    if !unsafe { IsWindowVisible(hwnd) }.as_bool() {
        return None;
    }

    let mut pid: u32 = 0;
    unsafe {
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
    }
    if pid == 0 || pid == own_pid {
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

    // Skip windows cloaked by Windows (suspended UWP apps, virtual desktops, hidden shell hosts).
    let mut cloaked: u32 = 0;
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

    let title_len = unsafe { GetWindowTextLengthW(hwnd) };
    if title_len <= 0 {
        return None;
    }
    let mut title_buf = vec![0u16; (title_len + 1) as usize];
    let read_len = unsafe { GetWindowTextW(hwnd, &mut title_buf) };
    if read_len <= 0 {
        return None;
    }
    let title = String::from_utf16_lossy(&title_buf[..read_len as usize]);

    let mut class_buf = [0u16; 64];
    let class_len = unsafe { GetClassNameW(hwnd, &mut class_buf) };
    let class_name = String::from_utf16_lossy(&class_buf[..class_len.max(0) as usize]);
    if matches!(
        class_name.as_str(),
        "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd" | "Windows.UI.Core.CoreWindow"
    ) {
        return None;
    }

    let mut rect = RECT::default();
    if unsafe { GetWindowRect(hwnd, &mut rect) }.is_ok()
        && (rect.right - rect.left <= 0 || rect.bottom - rect.top <= 0)
    {
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
        rect
    };
    let frame_rect = PhysicalRect {
        x: vis_rc.left,
        y: vis_rc.top,
        width: vis_rc.right - vis_rc.left,
        height: vis_rc.bottom - vis_rc.top,
    };
    let monitor_idx = monitor_index_for_rect(monitors, frame_rect);

    let (exe_path, exe_name) = process_exe_info(pid);

    let cache_key = if !exe_path.is_empty() {
        exe_path.to_ascii_lowercase()
    } else {
        format!("hwnd:{}", hwnd.0 as isize)
    };
    let icon = get_or_extract_window_icon(&cache_key, hwnd, &exe_path);

    Some(RunningWindow {
        hwnd: hwnd.0 as isize,
        title,
        exe_name,
        exe_path,
        pid,
        is_focused: hwnd.0 as isize == foreground_hwnd,
        is_minimized: unsafe { IsIconic(hwnd) }.as_bool(),
        is_maximized: unsafe { IsZoomed(hwnd) }.as_bool(),
        frame_rect,
        monitor_idx,
        icon,
    })
}

pub(crate) fn process_exe_info(pid: u32) -> (String, String) {
    let exe_path = process_image_path(pid).unwrap_or_default();
    let exe_name = Path::new(&exe_path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_string();
    (exe_path, exe_name)
}

pub(crate) fn cached_window_icon(hwnd: HWND, exe_path: &str) -> Option<RgbaIcon> {
    let cache_key = if !exe_path.is_empty() {
        exe_path.to_ascii_lowercase()
    } else {
        format!("hwnd:{}", hwnd.0 as isize)
    };
    get_or_extract_window_icon(&cache_key, hwnd, exe_path)
}

fn process_image_path(pid: u32) -> Option<String> {
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
    let mut buf = [0u16; 1024];
    let mut len = buf.len() as u32;
    let ok = unsafe {
        QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_FORMAT(0),
            PWSTR(buf.as_mut_ptr()),
            &mut len,
        )
    };
    unsafe {
        let _ = CloseHandle(handle);
    }
    if ok.is_ok() && len > 0 {
        Some(String::from_utf16_lossy(&buf[..len as usize]))
    } else {
        None
    }
}

fn get_or_extract_window_icon(cache_key: &str, hwnd: HWND, exe_path: &str) -> Option<RgbaIcon> {
    if let Ok(guard) = icon_cache().lock()
        && let Some(cached) = guard.get(cache_key)
    {
        return cached.clone();
    }

    let extracted = extract_window_icon_uncached(hwnd, exe_path);
    if let Ok(mut guard) = icon_cache().lock() {
        guard.insert(cache_key.to_string(), extracted.clone());
    }
    extracted
}

/// Extracts and caches a crisp 64x64 icon for a pinned app target (AUMID, Win32 exe, or URI).
pub fn resolve_pinned_app_icon(target: &str, exe_name: &str) -> Option<RgbaIcon> {
    let key = if !exe_name.trim().is_empty() {
        format!("pin:{}", exe_name.to_ascii_lowercase())
    } else {
        format!("pin:{}", target.to_ascii_lowercase())
    };
    if let Ok(guard) = icon_cache().lock()
        && let Some(cached) = guard.get(&key)
    {
        return cached.clone();
    }

    let known_aumid = known_aumid_for_app(target, exe_name).map(str::to_string).or_else(|| {
        if target.contains('!') || target.starts_with('{') {
            Some(target.to_string())
        } else {
            None
        }
    });
    let icon = known_aumid
        .as_deref()
        .and_then(|aumid| extract_shell_item_icon(&format!("shell:AppsFolder\\{aumid}")))
        .or_else(|| {
            let resolved_path =
                resolve_executable_path(target).or_else(|| resolve_executable_path(exe_name))?;
            extract_shell_item_icon(&resolved_path).or_else(|| extract_file_icon(&resolved_path))
        });

    if let Ok(mut guard) = icon_cache().lock() {
        guard.insert(key, icon.clone());
    }
    icon
}

fn known_aumid_for_app(target: &str, exe_name: &str) -> Option<&'static str> {
    if target.eq_ignore_ascii_case("wt.exe")
        || exe_name.eq_ignore_ascii_case("WindowsTerminal.exe")
    {
        Some("Microsoft.WindowsTerminal_8wekyb3d8bbwe!App")
    } else if target.eq_ignore_ascii_case("ms-settings:")
        || exe_name.eq_ignore_ascii_case("SystemSettings.exe")
    {
        Some("windows.immersivecontrolpanel_cw5n1h2txyewy!microsoft.windows.immersivecontrolpanel")
    } else if exe_name.eq_ignore_ascii_case("notepad.exe") {
        Some("Microsoft.WindowsNotepad_8wekyb3d8bbwe!App")
    } else if exe_name.eq_ignore_ascii_case("msedge.exe") {
        Some("MSEdge")
    } else {
        None
    }
}

fn extract_shell_item_icon(parsing_name: &str) -> Option<RgbaIcon> {
    let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    let should_uninit = hr.is_ok();

    let result = (|| -> Option<RgbaIcon> {
        let wide: Vec<u16> = parsing_name.encode_utf16().chain(Some(0)).collect();
        let factory: IShellItemImageFactory =
            unsafe { SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None) }.ok()?;
        let hbitmap = unsafe {
            factory.GetImage(
                SIZE { cx: 64, cy: 64 },
                SIIGBF_RESIZETOFIT | SIIGBF_BIGGERSIZEOK,
            )
        }
        .ok()?;
        let rgba = hbitmap_to_rgba(hbitmap, None);
        unsafe {
            let _ = DeleteObject(hbitmap.into());
        }
        rgba
    })();

    if should_uninit {
        unsafe { CoUninitialize() };
    }
    result
}

fn resolve_executable_path(name: &str) -> Option<String> {
    if name.is_empty() || name.contains(':') && !name.contains(":\\") {
        return None;
    }
    if Path::new(name).exists() {
        return Some(name.to_string());
    }

    if let Some(app_path) = lookup_app_paths_registry(name) {
        return Some(app_path);
    }

    let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    let mut out = [0u16; 1024];
    let len = unsafe {
        SearchPathW(
            None,
            PCWSTR(wide.as_ptr()),
            None,
            Some(&mut out),
            None,
        )
    };
    if len > 0 && (len as usize) < out.len() {
        return Some(String::from_utf16_lossy(&out[..len as usize]));
    }

    // Check %LOCALAPPDATA%\Microsoft\WindowsApps and Windows ImmersiveControlPanel.
    if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
        let candidate = Path::new(&local_app_data)
            .join("Microsoft")
            .join("WindowsApps")
            .join(name);
        if candidate.exists() {
            return candidate.to_str().map(str::to_string);
        }
    }
    if name.eq_ignore_ascii_case("SystemSettings.exe") {
        let sys_settings = Path::new(r"C:\Windows\ImmersiveControlPanel\SystemSettings.exe");
        if sys_settings.exists() {
            return sys_settings.to_str().map(str::to_string);
        }
    }
    None
}

fn lookup_app_paths_registry(exe_name: &str) -> Option<String> {
    let subkey: Vec<u16> = format!(
        "Software\\Microsoft\\Windows\\CurrentVersion\\App Paths\\{exe_name}"
    )
    .encode_utf16()
    .chain(Some(0))
    .collect();
    for root in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        let mut buf = [0u16; 1024];
        let mut bytes = (buf.len() * 2) as u32;
        let status = unsafe {
            RegGetValueW(
                root,
                PCWSTR(subkey.as_ptr()),
                None,
                RRF_RT_REG_SZ,
                None,
                Some(buf.as_mut_ptr().cast()),
                Some(&mut bytes),
            )
        };
        if status.is_ok() && bytes >= 2 {
            let len = (bytes as usize / 2).saturating_sub(1);
            let path = String::from_utf16_lossy(&buf[..len])
                .trim_matches('"')
                .to_string();
            if Path::new(&path).exists() {
                return Some(path);
            }
        }
    }
    None
}

fn extract_window_icon_uncached(hwnd: HWND, exe_path: &str) -> Option<RgbaIcon> {
    for icon_size in [ICON_BIG as usize, ICON_SMALL2 as usize] {
        let mut raw_icon: usize = 0;
        let sent = unsafe {
            SendMessageTimeoutW(
                hwnd,
                WM_GETICON,
                WPARAM(icon_size),
                LPARAM(0),
                SMTO_ABORTIFHUNG,
                40,
                Some(&mut raw_icon),
            )
        };
        if sent.0 != 0 && raw_icon != 0 {
            let hicon = HICON(raw_icon as *mut c_void);
            if let Some(rgba) = hicon_to_rgba(hicon) {
                return Some(rgba);
            }
        }
    }

    let class_icon = unsafe { GetClassLongPtrW(hwnd, GCLP_HICON) };
    if class_icon != 0 {
        let hicon = HICON(class_icon as *mut c_void);
        if let Some(rgba) = hicon_to_rgba(hicon) {
            return Some(rgba);
        }
    }

    if !exe_path.is_empty() {
        return extract_file_icon(exe_path);
    }
    None
}

fn extract_file_icon(path: &str) -> Option<RgbaIcon> {
    let wide: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
    let mut shfi = SHFILEINFOW::default();
    let res = unsafe {
        SHGetFileInfoW(
            PCWSTR(wide.as_ptr()),
            Default::default(),
            Some(&mut shfi),
            size_of::<SHFILEINFOW>() as u32,
            SHGFI_ICON | SHGFI_LARGEICON,
        )
    };
    if res == 0 || shfi.hIcon.is_invalid() {
        return None;
    }
    let rgba = hicon_to_rgba(shfi.hIcon);
    unsafe {
        let _ = DestroyIcon(shfi.hIcon);
    }
    rgba
}

fn hicon_to_rgba(hicon: HICON) -> Option<RgbaIcon> {
    let mut info = ICONINFO::default();
    if unsafe { GetIconInfo(hicon, &mut info) }.is_err() {
        return None;
    }

    let color_bmp = info.hbmColor;
    let mask_bmp = info.hbmMask;
    let result = if !color_bmp.is_invalid() {
        let mask_opt = (!mask_bmp.is_invalid()).then_some(mask_bmp);
        hbitmap_to_rgba(color_bmp, mask_opt)
    } else {
        None
    };

    if !color_bmp.is_invalid() {
        unsafe {
            let _ = DeleteObject(color_bmp.into());
        }
    }
    if !mask_bmp.is_invalid() {
        unsafe {
            let _ = DeleteObject(mask_bmp.into());
        }
    }
    result
}

pub(crate) fn hbitmap_to_rgba(color_bmp: HBITMAP, mask_bmp: Option<HBITMAP>) -> Option<RgbaIcon> {
    let mut bmp = BITMAP::default();
    let got = unsafe {
        GetObjectW(
            color_bmp.into(),
            size_of::<BITMAP>() as i32,
            Some((&raw mut bmp).cast()),
        )
    };
    if got == 0 || bmp.bmWidth <= 0 || bmp.bmHeight <= 0 {
        return None;
    }
    let w = bmp.bmWidth;
    let h = bmp.bmHeight;

    let hdc = unsafe { GetDC(None) };
    if hdc.is_invalid() {
        return None;
    }

    let mut bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h, // top-down
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };

    let mut pixels = vec![0u8; (w * h * 4) as usize];
    let lines = unsafe {
        GetDIBits(
            hdc,
            color_bmp,
            0,
            h as u32,
            Some(pixels.as_mut_ptr().cast()),
            &mut bmi,
            DIB_RGB_COLORS,
        )
    };

    let mut any_alpha = false;
    if lines > 0 {
        for px in pixels.as_chunks_mut::<4>().0 {
            px.swap(0, 2); // BGRA -> RGBA
            if px[3] != 0 {
                any_alpha = true;
            }
        }
        if !any_alpha && let Some(mask) = mask_bmp {
            let mut mask_pixels = vec![0u8; (w * h * 4) as usize];
            let mask_lines = unsafe {
                GetDIBits(
                    hdc,
                    mask,
                    0,
                    h as u32,
                    Some(mask_pixels.as_mut_ptr().cast()),
                    &mut bmi,
                    DIB_RGB_COLORS,
                )
            };
            if mask_lines > 0 {
                for (px, m) in pixels
                    .as_chunks_mut::<4>()
                    .0
                    .iter_mut()
                    .zip(mask_pixels.as_chunks::<4>().0)
                {
                    px[3] = if m[0] == 0 { 255 } else { 0 };
                }
            }
        }
    }

    unsafe {
        let _ = ReleaseDC(None, hdc);
    }

    (lines > 0).then(|| RgbaIcon {
        width: w as u32,
        height: h as u32,
        pixels: Arc::from(pixels),
    })
}

/// Focuses, restores, or minimizes a running user window cleanly.
pub fn focus_or_toggle_window(hwnd_raw: isize) {
    if hwnd_raw == 0 {
        return;
    }
    let hwnd = HWND(hwnd_raw as *mut c_void);
    if !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
        return;
    }
    let fg = unsafe { GetForegroundWindow() };
    let minimized = unsafe { IsIconic(hwnd) }.as_bool();
    unsafe {
        if minimized {
            let _ = ShowWindow(hwnd, SW_RESTORE);
            let _ = SetForegroundWindow(hwnd);
        } else if fg == hwnd {
            let _ = ShowWindow(hwnd, SW_MINIMIZE);
        } else {
            let _ = SetForegroundWindow(hwnd);
        }
    }
}

/// Sends `WM_CLOSE` to politely close a running user window.
pub fn close_window(hwnd_raw: isize) {
    if hwnd_raw == 0 {
        return;
    }
    let hwnd = HWND(hwnd_raw as *mut c_void);
    if unsafe { IsWindow(Some(hwnd)) }.as_bool() {
        unsafe {
            let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
    }
}

/// Launches an executable, document, or URI (`ms-settings:`, `explorer.exe`, etc.) via `ShellExecuteW`.
pub fn launch_target(target: &str) {
    if target.trim().is_empty() {
        return;
    }
    let wide_target: Vec<u16> = target.encode_utf16().chain(Some(0)).collect();
    unsafe {
        let _ = ShellExecuteW(
            None,
            w!("open"),
            PCWSTR(wide_target.as_ptr()),
            None,
            None,
            SW_SHOWNORMAL,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enumerate_running_windows_does_not_panic() {
        let list = enumerate_running_windows();
        for w in &list {
            assert!(!w.title.is_empty());
        }
    }
}
