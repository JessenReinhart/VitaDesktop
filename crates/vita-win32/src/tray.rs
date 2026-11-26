use crate::windows::{cached_window_icon, hbitmap_to_rgba, process_exe_info};
use std::collections::HashSet;
use std::ffi::c_void;
use std::mem::size_of;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::UnboundedSender;
use vita_core::RgbaIcon;
use windows::Win32::Foundation::{
    CloseHandle, ERROR_CLASS_ALREADY_EXISTS, HANDLE, HWND, LPARAM, LRESULT, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS,
    DeleteDC, DeleteObject, GetDC, HBITMAP, HDC, HGDIOBJ, ReleaseDC, SelectObject,
};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoUninitialize,
};
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::{
    MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE, VirtualAllocEx, VirtualFreeEx,
};
use windows::Win32::System::Threading::{
    GetCurrentProcessId, GetCurrentThreadId, OpenProcess, PROCESS_QUERY_INFORMATION,
    PROCESS_VM_OPERATION, PROCESS_VM_READ,
};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationExpandCollapsePattern,
    IUIAutomationInvokePattern, TreeScope_Subtree, UIA_ButtonControlTypeId,
    UIA_ControlTypePropertyId, UIA_ExpandCollapsePatternId, UIA_InvokePatternId,
};
use windows::Win32::UI::Controls::{
    TB_BUTTONCOUNT, TB_GETBUTTON, TB_GETBUTTONTEXTW, TBSTATE_HIDDEN,
};
use windows::Win32::UI::Shell::NIN_SELECT;
use windows::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, ChangeWindowMessageFilterEx, CreateWindowExW, DI_MASK, DI_NORMAL,
    DefWindowProcW, DestroyWindow, DispatchMessageW, DrawIconEx, EnumWindows, FindWindowExW,
    FindWindowW, GWLP_USERDATA, GetClassNameW, GetIconInfo, GetMessageW, GetWindowLongPtrW,
    GetWindowThreadProcessId, HICON, HWND_BROADCAST, HWND_TOPMOST, ICONINFO, IsWindow,
    IsWindowVisible, KillTimer, MSG, MSGFLT_ALLOW, PM_NOREMOVE, PeekMessageW, PostMessageW,
    PostThreadMessageW, RegisterClassExW, RegisterWindowMessageW, SMTO_ABORTIFHUNG, SMTO_NORMAL,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SendMessageTimeoutW, SendNotifyMessageW,
    SetForegroundWindow, SetTimer, SetWindowLongPtrW, SetWindowPos, TranslateMessage,
    UnregisterClassW, WM_APPCOMMAND, WM_COMMAND, WM_CONTEXTMENU, WM_COPYDATA, WM_HOTKEY,
    WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_QUIT, WM_RBUTTONDOWN, WM_RBUTTONUP,
    WM_SYSCOMMAND, WM_TIMER, WM_USER, WNDCLASSEXW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_POPUP,
};
use windows::core::{BOOL, PCWSTR, w};

const REMOTE_BUF_SIZE: usize = 4096;
const MAX_TOOLBAR_BUTTONS: usize = 256;
const WM_TRAY_REFRESH: u32 = WM_USER + 1;

const COPYDATA_SHELLTRAYDATA: usize = 1;
const SHELLTRAYDATA_SIGNATURE: u32 = 0x3475_3423;

const NIM_ADD: u32 = 0;
const NIM_MODIFY: u32 = 1;
const NIM_DELETE: u32 = 2;
const NIM_SETFOCUS: u32 = 3;
const NIM_SETVERSION: u32 = 4;

const NIF_MESSAGE: u32 = 0x0000_0001;
const NIF_ICON: u32 = 0x0000_0002;
const NIF_TIP: u32 = 0x0000_0004;
const NIF_STATE: u32 = 0x0000_0008;
const NIF_GUID: u32 = 0x0000_0020;
const NIF_SHOWTIP: u32 = 0x0000_0080;

const NIS_HIDDEN: u32 = 0x0000_0001;

const DEBOUNCE_TIMER_ID: usize = 1;
const HOUSEKEEP_TIMER_ID: usize = 2;
const DEBOUNCE_MS: u32 = 120;
const HOUSEKEEP_MS: u32 = 4000;
const TASKBAR_CREATED_COOLDOWN: Duration = Duration::from_millis(2000);
const MONOCHROME_ICON_SIZE: i32 = 32;

static PROXY_SNAPSHOT: OnceLock<Mutex<Vec<TrayIconInfo>>> = OnceLock::new();
static PROXY_THREAD_ID: AtomicU32 = AtomicU32::new(0);

#[cfg(test)]
pub(crate) static SHELL_TRAY_TEST_LOCK: Mutex<()> = Mutex::new(());

fn proxy_snapshot_store() -> &'static Mutex<Vec<TrayIconInfo>> {
    PROXY_SNAPSHOT.get_or_init(|| Mutex::new(Vec::new()))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayIconInfo {
    pub id: String,
    pub tooltip: String,
    pub exe_name: String,
    pub owner_hwnd: isize,
    pub uid: u32,
    pub callback_msg: u32,
    pub callback_version: Option<u32>,
    pub uia_automation_id: Option<String>,
    pub is_overflow: bool,
    pub icon: Option<RgbaIcon>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayClickKind {
    LeftClick,
    RightClick,
    DoubleClick,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
struct TbButton64 {
    i_bitmap: i32,
    id_command: i32,
    fs_state: u8,
    fs_style: u8,
    reserved: [u8; 6],
    dw_data: u64,
    i_string: i64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
struct TrayData64 {
    hwnd: u64,
    uid: u32,
    callback_msg: u32,
    reserved: [u32; 2],
    hicon: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedShellTrayData {
    dw_message: u32,
    cb_size: u32,
    owner_hwnd: isize,
    uid: u32,
    u_flags: u32,
    callback_msg: u32,
    hicon_raw: u64,
    tooltip: Option<String>,
    dw_state: u32,
    dw_state_mask: u32,
    u_timeout_or_version: Option<u32>,
    guid_item: Option<[u8; 16]>,
    caller_exe_raw: Option<String>,
}

struct ProxyTrayEntry {
    owner_hwnd: isize,
    uid: u32,
    guid: Option<[u8; 16]>,
    callback_msg: u32,
    callback_version: Option<u32>,
    tooltip: String,
    exe_name: String,
    exe_path: String,
    icon: Option<RgbaIcon>,
    state: u32,
    order: u64,
}

struct TrayProxyState {
    spy_hwnd: HWND,
    wm_taskbar_created: u32,
    entries: Vec<ProxyTrayEntry>,
    next_order: u64,
    tx: UnboundedSender<Vec<TrayIconInfo>>,
    last_emitted: Vec<TrayIconInfo>,
    last_explorer_tray: Option<isize>,
    last_broadcast_at: Option<Instant>,
    debounce_armed: bool,
}

struct ComGuard {
    should_uninit: bool,
}

impl ComGuard {
    fn init_sta() -> Self {
        let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        Self {
            should_uninit: hr.is_ok(),
        }
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.should_uninit {
            unsafe { CoUninitialize() };
        }
    }
}

struct ProcessHandleGuard(HANDLE);

impl Drop for ProcessHandleGuard {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}

struct RemoteMemoryGuard {
    process: HANDLE,
    ptr: *mut c_void,
}

impl RemoteMemoryGuard {
    fn alloc(process: HANDLE, size: usize) -> Option<Self> {
        let ptr = unsafe {
            VirtualAllocEx(
                process,
                None,
                size,
                MEM_COMMIT | MEM_RESERVE,
                PAGE_READWRITE,
            )
        };
        if ptr.is_null() {
            None
        } else {
            Some(Self { process, ptr })
        }
    }
}

impl Drop for RemoteMemoryGuard {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            unsafe {
                let _ = VirtualFreeEx(self.process, self.ptr, 0, MEM_RELEASE);
            }
        }
    }
}

struct ScreenDcGuard(HDC);

impl Drop for ScreenDcGuard {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = ReleaseDC(None, self.0);
            }
        }
    }
}

struct MemDcGuard(HDC);

impl Drop for MemDcGuard {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = DeleteDC(self.0);
            }
        }
    }
}

struct BitmapGuard(HBITMAP);

impl Drop for BitmapGuard {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = DeleteObject(self.0.into());
            }
        }
    }
}

struct SelectedObjectGuard {
    hdc: HDC,
    prev: HGDIOBJ,
}

impl Drop for SelectedObjectGuard {
    fn drop(&mut self) {
        if !self.hdc.is_invalid() && !self.prev.is_invalid() {
            unsafe {
                let _ = SelectObject(self.hdc, self.prev);
            }
        }
    }
}

/// Enumerates active system tray icons from the `Shell_TrayWnd` proxy cache, classic
/// `ToolbarWindow32` hosts, and (only if empty and Explorer's taskbar is visible) Windows 11 UIA.
pub fn enumerate_tray_icons() -> Vec<TrayIconInfo> {
    let proxy_items = read_live_proxy_snapshot();
    let classic = enumerate_classic_tray_icons();
    let merged = deduplicate_tray_items(proxy_items, classic);
    if !merged.is_empty() {
        return merged;
    }

    if !is_explorer_taskbar_visible() {
        return Vec::new();
    }

    let _com = ComGuard::init_sta();
    let uia: Option<IUIAutomation> =
        unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) }.ok();
    let uia_items = uia
        .as_ref()
        .map(enumerate_uia_tray_icons)
        .unwrap_or_default();
    drop(uia);
    deduplicate_tray_items(Vec::new(), uia_items)
}

/// Requests the background tray proxy to re-broadcast `TaskbarCreated` (rate-limited)
/// and refresh its snapshot immediately.
pub fn refresh_tray_icons() {
    let tid = PROXY_THREAD_ID.load(Ordering::Acquire);
    if tid != 0 {
        unsafe {
            let _ = PostThreadMessageW(tid, WM_TRAY_REFRESH, WPARAM(0), LPARAM(0));
        }
    }
}

fn read_live_proxy_snapshot() -> Vec<TrayIconInfo> {
    let guard = proxy_snapshot_store()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    guard
        .iter()
        .filter(|item| {
            item.owner_hwnd == 0
                || unsafe { IsWindow(Some(HWND(item.owner_hwnd as *mut c_void))) }.as_bool()
        })
        .cloned()
        .collect()
}

fn window_class_equals(hwnd: HWND, expected: &str) -> bool {
    let mut buf = [0u16; 96];
    let len = unsafe { GetClassNameW(hwnd, &mut buf) };
    if len <= 0 {
        return false;
    }
    String::from_utf16_lossy(&buf[..len as usize]) == expected
}

fn find_real_explorer_tray_hwnd() -> Option<HWND> {
    struct Search {
        own_pid: u32,
        found: Option<HWND>,
    }

    unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let search = unsafe { &mut *(lparam.0 as *mut Search) };
        let mut pid: u32 = 0;
        unsafe {
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
        }
        if pid != 0 && pid != search.own_pid && window_class_equals(hwnd, "Shell_TrayWnd") {
            search.found = Some(hwnd);
            return BOOL(0);
        }
        BOOL(1)
    }

    let mut search = Search {
        own_pid: unsafe { GetCurrentProcessId() },
        found: None,
    };
    let ptr = (&raw mut search) as isize;
    unsafe {
        let _ = EnumWindows(Some(enum_proc), LPARAM(ptr));
    }
    search.found
}

fn is_explorer_taskbar_visible() -> bool {
    find_real_explorer_tray_hwnd().is_some_and(|hwnd| unsafe { IsWindowVisible(hwnd) }.as_bool())
}

fn find_foreign_top_level_windows(class_name: PCWSTR) -> Vec<HWND> {
    let own_pid = unsafe { GetCurrentProcessId() };
    let mut out = Vec::new();
    let mut prev: Option<HWND> = None;
    for _ in 0..16 {
        let Ok(hwnd) = (unsafe { FindWindowExW(None, prev, class_name, None) }) else {
            break;
        };
        if hwnd.0.is_null() {
            break;
        }
        let mut pid: u32 = 0;
        unsafe {
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
        }
        if pid != 0 && pid != own_pid {
            out.push(hwnd);
        }
        prev = Some(hwnd);
    }
    out
}

fn find_child_window(parent: HWND, after: Option<HWND>, class_name: PCWSTR) -> Option<HWND> {
    let hwnd = unsafe { FindWindowExW(Some(parent), after, class_name, None) }.ok()?;
    (!hwnd.0.is_null()).then_some(hwnd)
}

fn find_child_toolbars(parent: HWND) -> Vec<HWND> {
    let mut out = Vec::new();
    let mut prev: Option<HWND> = None;
    for _ in 0..8 {
        let Some(tb) = find_child_window(parent, prev, w!("ToolbarWindow32")) else {
            break;
        };
        out.push(tb);
        prev = Some(tb);
    }
    out
}

fn locate_classic_toolbars() -> Vec<(HWND, bool)> {
    let mut toolbars = Vec::new();

    for shell_tray in find_foreign_top_level_windows(w!("Shell_TrayWnd")) {
        if let Some(tray_notify) = find_child_window(shell_tray, None, w!("TrayNotifyWnd")) {
            if let Some(sys_pager) = find_child_window(tray_notify, None, w!("SysPager")) {
                for tb in find_child_toolbars(sys_pager) {
                    toolbars.push((tb, false));
                }
            } else {
                for tb in find_child_toolbars(tray_notify) {
                    toolbars.push((tb, false));
                }
            }
        }
    }

    for overflow in find_foreign_top_level_windows(w!("NotifyIconOverflowWindow")) {
        for tb in find_child_toolbars(overflow) {
            toolbars.push((tb, true));
        }
    }

    toolbars
}

fn enumerate_classic_tray_icons() -> Vec<TrayIconInfo> {
    let mut items = Vec::new();
    for (toolbar_hwnd, is_overflow) in locate_classic_toolbars() {
        items.extend(read_toolbar_buttons(toolbar_hwnd, is_overflow));
    }
    items
}

fn read_toolbar_buttons(toolbar_hwnd: HWND, is_overflow: bool) -> Vec<TrayIconInfo> {
    let mut explorer_pid: u32 = 0;
    unsafe {
        GetWindowThreadProcessId(toolbar_hwnd, Some(&mut explorer_pid));
    }
    if explorer_pid == 0 {
        return Vec::new();
    }

    let mut count_raw: usize = 0;
    let count_sent = unsafe {
        SendMessageTimeoutW(
            toolbar_hwnd,
            TB_BUTTONCOUNT,
            WPARAM(0),
            LPARAM(0),
            SMTO_ABORTIFHUNG,
            250,
            Some(&raw mut count_raw),
        )
    };
    if count_sent.0 == 0 {
        return Vec::new();
    }
    let count = (count_raw as isize).clamp(0, MAX_TOOLBAR_BUTTONS as isize) as usize;
    if count == 0 {
        return Vec::new();
    }

    let Ok(proc_handle) = (unsafe {
        OpenProcess(
            PROCESS_VM_OPERATION | PROCESS_VM_READ | PROCESS_QUERY_INFORMATION,
            false,
            explorer_pid,
        )
    }) else {
        return Vec::new();
    };
    let process = ProcessHandleGuard(proc_handle);
    let Some(remote) = RemoteMemoryGuard::alloc(process.0, REMOTE_BUF_SIZE) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for idx in 0..count {
        let mut got_btn: usize = 0;
        let btn_sent = unsafe {
            SendMessageTimeoutW(
                toolbar_hwnd,
                TB_GETBUTTON,
                WPARAM(idx),
                LPARAM(remote.ptr as isize),
                SMTO_ABORTIFHUNG,
                250,
                Some(&raw mut got_btn),
            )
        };
        if btn_sent.0 == 0 || got_btn == 0 {
            continue;
        }

        let mut btn = TbButton64::default();
        let mut bytes_read: usize = 0;
        let btn_read = unsafe {
            ReadProcessMemory(
                process.0,
                remote.ptr,
                (&raw mut btn).cast(),
                size_of::<TbButton64>(),
                Some(&raw mut bytes_read),
            )
        };
        if btn_read.is_err() || bytes_read != size_of::<TbButton64>() {
            continue;
        }

        if (u32::from(btn.fs_state) & TBSTATE_HIDDEN) != 0 || btn.dw_data == 0 {
            continue;
        }

        let mut tray = TrayData64::default();
        let mut tray_bytes: usize = 0;
        let tray_read = unsafe {
            ReadProcessMemory(
                process.0,
                btn.dw_data as usize as *const c_void,
                (&raw mut tray).cast(),
                size_of::<TrayData64>(),
                Some(&raw mut tray_bytes),
            )
        };
        if tray_read.is_err() || tray_bytes != size_of::<TrayData64>() || tray.hwnd == 0 {
            continue;
        }

        let owner_hwnd = HWND(tray.hwnd as usize as *mut c_void);
        if !unsafe { IsWindow(Some(owner_hwnd)) }.as_bool() {
            continue;
        }

        let mut owner_pid: u32 = 0;
        unsafe {
            GetWindowThreadProcessId(owner_hwnd, Some(&mut owner_pid));
        }
        let (exe_path, exe_name) = if owner_pid != 0 {
            process_exe_info(owner_pid)
        } else {
            (String::new(), String::new())
        };

        let raw_tooltip = read_toolbar_button_tooltip(toolbar_hwnd, process.0, &remote, &btn);
        let mut tooltip = clean_tooltip(&raw_tooltip);
        if tooltip.is_empty() {
            tooltip = exe_name
                .strip_suffix(".exe")
                .or_else(|| exe_name.strip_suffix(".EXE"))
                .unwrap_or(&exe_name)
                .to_string();
        }
        if tooltip.is_empty() {
            tooltip = "Tray icon".to_string();
        }

        let icon =
            hicon_raw_to_rgba(tray.hicon).or_else(|| cached_window_icon(owner_hwnd, &exe_path));

        let owner_isize = owner_hwnd.0 as isize;
        out.push(TrayIconInfo {
            id: format!("win32:{owner_isize:x}:{}", tray.uid),
            tooltip,
            exe_name,
            owner_hwnd: owner_isize,
            uid: tray.uid,
            callback_msg: tray.callback_msg,
            callback_version: None,
            uia_automation_id: None,
            is_overflow,
            icon,
        });
    }

    out
}

fn read_toolbar_button_tooltip(
    toolbar_hwnd: HWND,
    process: HANDLE,
    remote: &RemoteMemoryGuard,
    btn: &TbButton64,
) -> String {
    let mut len_raw: usize = 0;
    let len_sent = unsafe {
        SendMessageTimeoutW(
            toolbar_hwnd,
            TB_GETBUTTONTEXTW,
            WPARAM(btn.id_command as usize),
            LPARAM(0),
            SMTO_ABORTIFHUNG,
            250,
            Some(&raw mut len_raw),
        )
    };
    let char_len = len_raw as isize;
    let max_chars = (REMOTE_BUF_SIZE / size_of::<u16>()).saturating_sub(1);

    if len_sent.0 != 0 && char_len > 0 && (char_len as usize) <= max_chars {
        let mut copied_raw: usize = 0;
        let copy_sent = unsafe {
            SendMessageTimeoutW(
                toolbar_hwnd,
                TB_GETBUTTONTEXTW,
                WPARAM(btn.id_command as usize),
                LPARAM(remote.ptr as isize),
                SMTO_ABORTIFHUNG,
                250,
                Some(&raw mut copied_raw),
            )
        };
        let copied = copied_raw as isize;
        if copy_sent.0 != 0 && copied > 0 {
            let count = (copied as usize).min(max_chars);
            let mut buf = vec![0u16; count + 1];
            let mut bytes_read: usize = 0;
            let ok = unsafe {
                ReadProcessMemory(
                    process,
                    remote.ptr,
                    buf.as_mut_ptr().cast(),
                    buf.len() * size_of::<u16>(),
                    Some(&raw mut bytes_read),
                )
            };
            if ok.is_ok() && bytes_read >= size_of::<u16>() {
                let u16_read = (bytes_read / size_of::<u16>()).min(buf.len());
                return decode_utf16_until_nul(&buf[..u16_read]);
            }
        }
    }

    if btn.i_string > 0xFFFF {
        return read_remote_utf16_string(process, btn.i_string as usize as *const c_void);
    }

    String::new()
}

fn read_remote_utf16_string(process: HANDLE, remote_ptr: *const c_void) -> String {
    if remote_ptr.is_null() {
        return String::new();
    }

    const CHUNK_U16: usize = 64;
    const MAX_U16: usize = 512;
    let mut collected = Vec::new();
    let mut offset = 0usize;

    while collected.len() < MAX_U16 {
        let mut chunk = [0u16; CHUNK_U16];
        let mut bytes_read: usize = 0;
        let addr = unsafe { remote_ptr.cast::<u16>().add(offset) }.cast();
        let ok = unsafe {
            ReadProcessMemory(
                process,
                addr,
                chunk.as_mut_ptr().cast(),
                chunk.len() * size_of::<u16>(),
                Some(&raw mut bytes_read),
            )
        };
        if ok.is_err() || bytes_read < size_of::<u16>() {
            break;
        }
        let n = (bytes_read / size_of::<u16>()).min(CHUNK_U16);
        if let Some(nul) = chunk[..n].iter().position(|&c| c == 0) {
            collected.extend_from_slice(&chunk[..nul]);
            break;
        }
        collected.extend_from_slice(&chunk[..n]);
        offset += n;
        if n < CHUNK_U16 {
            break;
        }
    }

    String::from_utf16_lossy(&collected)
}

fn decode_utf16_until_nul(units: &[u16]) -> String {
    let end = units.iter().position(|&c| c == 0).unwrap_or(units.len());
    String::from_utf16_lossy(&units[..end])
}

fn decode_utf16_le_bytes_until_nul(bytes: &[u8]) -> String {
    let mut units = Vec::with_capacity(bytes.len() / 2);
    for &pair in bytes.as_chunks::<2>().0 {
        let u = u16::from_le_bytes(pair);
        if u == 0 {
            break;
        }
        units.push(u);
    }
    String::from_utf16_lossy(&units)
}

fn read_u32_le(bytes: &[u8], offset: usize) -> Option<u32> {
    let slice = bytes.get(offset..offset + 4)?;
    Some(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

fn parse_shell_tray_data(bytes: &[u8]) -> Option<ParsedShellTrayData> {
    if bytes.len() < 32 {
        return None;
    }
    let dw_signature = read_u32_le(bytes, 0)?;
    if dw_signature != SHELLTRAYDATA_SIGNATURE {
        return None;
    }
    let dw_message = read_u32_le(bytes, 4)?;
    let cb_size = read_u32_le(bytes, 8)?;
    if cb_size < 24 {
        return None;
    }

    let hwnd_raw = read_u32_le(bytes, 12)?;
    let owner_hwnd = (hwnd_raw as i32) as isize;
    let uid = read_u32_le(bytes, 16)?;
    let u_flags = read_u32_le(bytes, 20)?;
    let callback_msg = read_u32_le(bytes, 24)?;
    let hicon_raw = u64::from(read_u32_le(bytes, 28)?);

    let tooltip = if (u_flags & NIF_TIP) != 0 && bytes.len() >= 0x22 {
        let tip_end = bytes.len().min(0x120);
        Some(decode_utf16_le_bytes_until_nul(&bytes[0x20..tip_end]))
    } else {
        None
    };

    let dw_state = read_u32_le(bytes, 0x120).unwrap_or(0);
    let dw_state_mask = read_u32_le(bytes, 0x124).unwrap_or(0);
    let u_timeout_or_version = read_u32_le(bytes, 0x328);

    let guid_item = bytes.get(0x3B0..0x3C0).and_then(|slice| {
        let mut g = [0u8; 16];
        g.copy_from_slice(slice);
        (g != [0u8; 16]).then_some(g)
    });

    let caller_exe_raw = if bytes.len() > 0x3C4 {
        let tail_end = bytes.len().min(0x3C4 + 520);
        let decoded = decode_utf16_le_bytes_until_nul(&bytes[0x3C4..tail_end]);
        let trimmed = decoded.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    } else {
        None
    };

    Some(ParsedShellTrayData {
        dw_message,
        cb_size,
        owner_hwnd,
        uid,
        u_flags,
        callback_msg,
        hicon_raw,
        tooltip,
        dw_state,
        dw_state_mask,
        u_timeout_or_version,
        guid_item,
        caller_exe_raw,
    })
}

fn expand_known_folder_path(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let mappings: &[(&str, &str)] = &[
        ("{6D809377-6AF0-444B-8957-A3773F02200E}", "ProgramW6432"),
        (
            "{7C5A40EF-A0FB-4BFC-874A-C0F2E0B9FA8E}",
            "ProgramFiles(x86)",
        ),
        ("{F1B32785-6FBA-4FCF-9D55-7B8E7F157091}", "LOCALAPPDATA"),
        ("{3EB685DB-65F9-4CF6-A03A-E3EF65729F3D}", "APPDATA"),
        ("{62AB5D82-FDC1-4DC3-A9DD-070D1D495D97}", "ProgramData"),
    ];

    let upper = trimmed.to_ascii_uppercase();
    let system_root = std::env::var("SystemRoot")
        .or_else(|_| std::env::var("WINDIR"))
        .unwrap_or_else(|_| r"C:\Windows".to_string());

    if let Some(rest) = upper
        .strip_prefix("{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}")
        .map(|_| &trimmed[38..])
    {
        return format!(r"{system_root}\System32{rest}");
    }
    if let Some(rest) = upper
        .strip_prefix("{D65231B0-B2F1-4857-A4CE-A8E7C6EA7D27}")
        .map(|_| &trimmed[38..])
    {
        return format!(r"{system_root}\SysWOW64{rest}");
    }
    if let Some(rest) = upper
        .strip_prefix("{F38BF404-1D43-42F2-9305-67DE0B28FC23}")
        .map(|_| &trimmed[38..])
    {
        return format!("{system_root}{rest}");
    }

    for &(guid, env_key) in mappings {
        if upper.starts_with(guid) {
            let rest = &trimmed[guid.len()..];
            let base = std::env::var(env_key)
                .or_else(|_| {
                    if env_key == "ProgramW6432" {
                        std::env::var("ProgramFiles")
                    } else {
                        Err(std::env::VarError::NotPresent)
                    }
                })
                .ok();
            if let Some(resolved) = base {
                return format!("{resolved}{rest}");
            }
        }
    }

    trimmed.to_string()
}

fn exe_name_from_path(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    Path::new(trimmed)
        .file_name()
        .and_then(|s| s.to_str())
        .or_else(|| trimmed.rsplit(['\\', '/']).next())
        .unwrap_or_default()
        .to_string()
}

fn format_guid_hex(guid: &[u8; 16]) -> String {
    let mut out = String::with_capacity(32);
    for b in guid {
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{b:02x}");
    }
    out
}

fn hicon_raw_to_rgba(raw_hicon: u64) -> Option<RgbaIcon> {
    if raw_hicon == 0 {
        return None;
    }
    let hicon = HICON(raw_hicon as usize as *mut c_void);
    let mut info = ICONINFO::default();
    let got_info = unsafe { GetIconInfo(hicon, &mut info) }.is_ok();
    let color_bmp = info.hbmColor;
    let mask_bmp = info.hbmMask;

    let from_color = if got_info && !color_bmp.is_invalid() {
        let mask_opt = (!mask_bmp.is_invalid()).then_some(mask_bmp);
        hbitmap_to_rgba(color_bmp, mask_opt)
    } else {
        None
    };

    if got_info {
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
    }

    from_color.or_else(|| draw_hicon_to_rgba(hicon, MONOCHROME_ICON_SIZE))
}

fn draw_hicon_to_rgba(hicon: HICON, size: i32) -> Option<RgbaIcon> {
    if size <= 0 {
        return None;
    }

    let screen_dc = ScreenDcGuard(unsafe { GetDC(None) });
    let ref_dc = (!screen_dc.0.is_invalid()).then_some(screen_dc.0);
    let mem_dc = MemDcGuard(unsafe { CreateCompatibleDC(ref_dc) });
    if mem_dc.0.is_invalid() {
        return None;
    }

    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: size,
            biHeight: -size, // top-down DIB
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };

    let mut bits_ptr: *mut c_void = std::ptr::null_mut();
    let hbm = unsafe {
        CreateDIBSection(
            Some(mem_dc.0),
            &bmi,
            DIB_RGB_COLORS,
            &raw mut bits_ptr,
            None,
            0,
        )
    }
    .ok()?;
    let bitmap = BitmapGuard(hbm);
    if bitmap.0.is_invalid() || bits_ptr.is_null() {
        return None;
    }

    let prev = unsafe { SelectObject(mem_dc.0, bitmap.0.into()) };
    if prev.is_invalid() {
        return None;
    }
    let _selected = SelectedObjectGuard {
        hdc: mem_dc.0,
        prev,
    };

    let byte_len = (size as usize) * (size as usize) * 4;
    unsafe {
        std::ptr::write_bytes(bits_ptr.cast::<u8>(), 0, byte_len);
    }

    if unsafe { DrawIconEx(mem_dc.0, 0, 0, hicon, size, size, 0, None, DI_NORMAL) }.is_err() {
        return None;
    }

    let mut pixels = vec![0u8; byte_len];
    unsafe {
        std::ptr::copy_nonoverlapping(bits_ptr.cast::<u8>(), pixels.as_mut_ptr(), byte_len);
    }

    let mut any_alpha = false;
    for px in pixels.as_chunks_mut::<4>().0 {
        px.swap(0, 2); // BGRA -> RGBA
        if px[3] != 0 {
            any_alpha = true;
        }
    }

    // Monochrome / masked icons leave alpha=0 under DI_NORMAL; recover crisp opacity via DI_MASK.
    if !any_alpha {
        unsafe {
            std::ptr::write_bytes(bits_ptr.cast::<u8>(), 0xFF, byte_len);
        }
        if unsafe { DrawIconEx(mem_dc.0, 0, 0, hicon, size, size, 0, None, DI_MASK) }.is_ok() {
            let mask_slice = unsafe { std::slice::from_raw_parts(bits_ptr.cast::<u8>(), byte_len) };
            for (px, mask_px) in pixels
                .as_chunks_mut::<4>()
                .0
                .iter_mut()
                .zip(mask_slice.as_chunks::<4>().0.iter())
            {
                let opaque_from_mask = mask_px[0] < 128 || mask_px[1] < 128 || mask_px[2] < 128;
                let opaque_from_rgb = px[0] != 0 || px[1] != 0 || px[2] != 0;
                if opaque_from_mask || opaque_from_rgb {
                    px[3] = 255;
                }
            }
        }
    }

    Some(RgbaIcon {
        width: size as u32,
        height: size as u32,
        pixels: pixels.into(),
    })
}

fn find_proxy_entry_index(
    entries: &[ProxyTrayEntry],
    parsed: &ParsedShellTrayData,
) -> Option<usize> {
    if (parsed.u_flags & NIF_GUID) != 0
        && let Some(guid) = parsed.guid_item
        && let Some(idx) = entries.iter().position(|e| e.guid == Some(guid))
    {
        return Some(idx);
    }
    if parsed.owner_hwnd != 0 {
        return entries
            .iter()
            .position(|e| e.owner_hwnd == parsed.owner_hwnd && e.uid == parsed.uid);
    }
    if let Some(guid) = parsed.guid_item {
        return entries.iter().position(|e| e.guid == Some(guid));
    }
    None
}

fn resolve_exe_for_entry(
    owner_hwnd: isize,
    caller_exe_raw: Option<&str>,
    fallback_path: &str,
    fallback_name: &str,
) -> (String, String) {
    if owner_hwnd != 0 {
        let mut pid: u32 = 0;
        unsafe {
            GetWindowThreadProcessId(HWND(owner_hwnd as *mut c_void), Some(&mut pid));
        }
        if pid != 0 {
            let (path, name) = process_exe_info(pid);
            if !name.is_empty() {
                return (path, name);
            }
        }
    }

    if let Some(raw) = caller_exe_raw {
        let expanded = expand_known_folder_path(raw);
        let name = exe_name_from_path(&expanded);
        if !name.is_empty() {
            return (expanded, name);
        }
    }

    (fallback_path.to_string(), fallback_name.to_string())
}

fn apply_parsed_tray_message(state: &mut TrayProxyState, parsed: ParsedShellTrayData) -> bool {
    let existing_idx = find_proxy_entry_index(&state.entries, &parsed);

    match parsed.dw_message {
        NIM_ADD | NIM_MODIFY => {
            if parsed.owner_hwnd != 0
                && !unsafe { IsWindow(Some(HWND(parsed.owner_hwnd as *mut c_void))) }.as_bool()
            {
                if let Some(idx) = existing_idx {
                    state.entries.remove(idx);
                    return true;
                }
                return false;
            }

            if let Some(idx) = existing_idx {
                let entry = &mut state.entries[idx];
                if parsed.owner_hwnd != 0 {
                    entry.owner_hwnd = parsed.owner_hwnd;
                    entry.uid = parsed.uid;
                }
                if (parsed.u_flags & NIF_GUID) != 0
                    && let Some(guid) = parsed.guid_item
                {
                    entry.guid = Some(guid);
                }
                if (parsed.u_flags & NIF_MESSAGE) != 0 {
                    entry.callback_msg = parsed.callback_msg;
                }
                if (parsed.u_flags & NIF_SHOWTIP) != 0 && entry.callback_version.is_none() {
                    entry.callback_version = Some(4);
                }
                if (parsed.u_flags & NIF_TIP) != 0
                    && let Some(tip) = parsed.tooltip.as_deref()
                {
                    entry.tooltip = clean_tooltip(tip);
                }
                if (parsed.u_flags & NIF_STATE) != 0 {
                    entry.state = (entry.state & !parsed.dw_state_mask)
                        | (parsed.dw_state & parsed.dw_state_mask);
                }
                let (exe_path, exe_name) = resolve_exe_for_entry(
                    entry.owner_hwnd,
                    parsed.caller_exe_raw.as_deref(),
                    &entry.exe_path,
                    &entry.exe_name,
                );
                entry.exe_path = exe_path;
                entry.exe_name = exe_name;

                if (parsed.u_flags & NIF_ICON) != 0 && parsed.hicon_raw != 0 {
                    if let Some(img) = hicon_raw_to_rgba(parsed.hicon_raw) {
                        entry.icon = Some(img);
                    }
                } else if entry.icon.is_none() && entry.owner_hwnd != 0 {
                    entry.icon =
                        cached_window_icon(HWND(entry.owner_hwnd as *mut c_void), &entry.exe_path);
                }
                true
            } else {
                // Only synthesize on NIM_MODIFY when owner_hwnd is valid (catches apps that skipped TaskbarCreated).
                if parsed.dw_message == NIM_MODIFY && parsed.owner_hwnd == 0 {
                    return false;
                }
                if parsed.owner_hwnd == 0 && parsed.guid_item.is_none() {
                    return false;
                }

                let (exe_path, exe_name) = resolve_exe_for_entry(
                    parsed.owner_hwnd,
                    parsed.caller_exe_raw.as_deref(),
                    "",
                    "",
                );
                let tooltip = parsed
                    .tooltip
                    .as_deref()
                    .map(clean_tooltip)
                    .unwrap_or_default();
                let icon = if (parsed.u_flags & NIF_ICON) != 0 && parsed.hicon_raw != 0 {
                    hicon_raw_to_rgba(parsed.hicon_raw)
                } else {
                    None
                }
                .or_else(|| {
                    (parsed.owner_hwnd != 0).then(|| {
                        cached_window_icon(HWND(parsed.owner_hwnd as *mut c_void), &exe_path)
                    })?
                });
                let state_bits = if (parsed.u_flags & NIF_STATE) != 0 {
                    parsed.dw_state & parsed.dw_state_mask
                } else {
                    0
                };
                let callback_msg = if (parsed.u_flags & NIF_MESSAGE) != 0 {
                    parsed.callback_msg
                } else {
                    0
                };
                let callback_version = ((parsed.u_flags & NIF_SHOWTIP) != 0).then_some(4);

                state.next_order += 1;
                let order = state.next_order;
                state.entries.push(ProxyTrayEntry {
                    owner_hwnd: parsed.owner_hwnd,
                    uid: parsed.uid,
                    guid: parsed.guid_item,
                    callback_msg,
                    callback_version,
                    tooltip,
                    exe_name,
                    exe_path,
                    icon,
                    state: state_bits,
                    order,
                });
                true
            }
        }
        NIM_DELETE => {
            if let Some(idx) = existing_idx {
                state.entries.remove(idx);
                true
            } else {
                false
            }
        }
        NIM_SETVERSION => {
            if let Some(idx) = existing_idx
                && let Some(ver) = parsed.u_timeout_or_version
            {
                state.entries[idx].callback_version = Some(ver);
                return true;
            }
            false
        }
        NIM_SETFOCUS => false,
        _ => false,
    }
}

fn entry_to_tray_icon_info(entry: &ProxyTrayEntry) -> TrayIconInfo {
    let id = match entry.guid {
        Some(guid) => format!("guid:{}", format_guid_hex(&guid)),
        None => format!("win32:{:x}:{}", entry.owner_hwnd, entry.uid),
    };
    let mut tooltip = clean_tooltip(&entry.tooltip);
    if tooltip.is_empty() {
        tooltip = entry
            .exe_name
            .strip_suffix(".exe")
            .or_else(|| entry.exe_name.strip_suffix(".EXE"))
            .unwrap_or(&entry.exe_name)
            .to_string();
    }
    if tooltip.is_empty() {
        tooltip = "Tray icon".to_string();
    }

    TrayIconInfo {
        id,
        tooltip,
        exe_name: entry.exe_name.clone(),
        owner_hwnd: entry.owner_hwnd,
        uid: entry.uid,
        callback_msg: entry.callback_msg,
        callback_version: entry.callback_version,
        uia_automation_id: None,
        is_overflow: false,
        icon: entry.icon.clone(),
    }
}

fn prune_dead_entries(state: &mut TrayProxyState) -> bool {
    let before = state.entries.len();
    state.entries.retain(|entry| {
        entry.owner_hwnd == 0
            || unsafe { IsWindow(Some(HWND(entry.owner_hwnd as *mut c_void))) }.as_bool()
    });
    state.entries.len() != before
}

fn build_proxy_snapshot(state: &mut TrayProxyState) -> Vec<TrayIconInfo> {
    prune_dead_entries(state);
    state.entries.sort_by_key(|e| e.order);

    let proxy_items: Vec<TrayIconInfo> = state
        .entries
        .iter()
        .filter(|e| (e.state & NIS_HIDDEN) == 0)
        .map(entry_to_tray_icon_info)
        .collect();

    let classic = enumerate_classic_tray_icons();
    deduplicate_tray_items(proxy_items, classic)
}

fn publish_snapshot(state: &mut TrayProxyState, force: bool) {
    let next = build_proxy_snapshot(state);
    {
        let mut store = proxy_snapshot_store()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        *store = next.clone();
    }
    if force || next != state.last_emitted {
        state.last_emitted = next.clone();
        let _ = state.tx.send(next);
    }
}

fn take_taskbar_created_message(state: &mut TrayProxyState, force: bool) -> Option<u32> {
    if state.wm_taskbar_created == 0 {
        return None;
    }
    let now = Instant::now();
    if !force
        && let Some(prev) = state.last_broadcast_at
        && now.duration_since(prev) < TASKBAR_CREATED_COOLDOWN
    {
        return None;
    }
    state.last_broadcast_at = Some(now);
    Some(state.wm_taskbar_created)
}

fn send_taskbar_created_broadcast(wm: u32) {
    if wm != 0 {
        unsafe {
            let _ = SendNotifyMessageW(HWND_BROADCAST, wm, WPARAM(0), LPARAM(0));
        }
    }
}

fn ensure_spy_hwnd_first(spy_hwnd: HWND) {
    let top_shell_tray = unsafe { FindWindowW(w!("Shell_TrayWnd"), None) }.ok();
    if top_shell_tray != Some(spy_hwnd) {
        unsafe {
            let _ = SetWindowPos(
                spy_hwnd,
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
    }
}

fn forward_to_explorer_tray(msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<usize> {
    let real_tray = find_real_explorer_tray_hwnd()?;
    let mut res: usize = 0;
    let ok = unsafe {
        SendMessageTimeoutW(
            real_tray,
            msg,
            wparam,
            lparam,
            SMTO_ABORTIFHUNG | SMTO_NORMAL,
            500,
            Some(&raw mut res),
        )
    };
    (ok.0 != 0).then_some(res)
}

fn arm_debounce_timer(state: &mut TrayProxyState) {
    if !state.debounce_armed {
        state.debounce_armed = true;
        unsafe {
            let _ = SetTimer(Some(state.spy_hwnd), DEBOUNCE_TIMER_ID, DEBOUNCE_MS, None);
        }
    }
}

unsafe extern "system" fn tray_proxy_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_COPYDATA {
        let cds_opt = (!((lparam.0 as *const COPYDATASTRUCT).is_null()))
            .then(|| unsafe { &*(lparam.0 as *const COPYDATASTRUCT) });

        let mut handled_tray_payload = false;
        if let Some(cds) = cds_opt
            && cds.dwData == COPYDATA_SHELLTRAYDATA
            && cds.cbData > 0
            && !cds.lpData.is_null()
        {
            let slice =
                unsafe { std::slice::from_raw_parts(cds.lpData.cast::<u8>(), cds.cbData as usize) };
            if let Some(parsed) = parse_shell_tray_data(slice) {
                let state_ptr =
                    unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut TrayProxyState;
                if !state_ptr.is_null() {
                    let state = unsafe { &mut *state_ptr };
                    if apply_parsed_tray_message(state, parsed) {
                        arm_debounce_timer(state);
                    }
                    handled_tray_payload = true;
                }
            }
        }

        // Mutable borrow of TrayProxyState is strictly dropped before any SendMessageTimeoutW!
        let fwd_result = forward_to_explorer_tray(WM_COPYDATA, wparam, lparam).unwrap_or(0);
        if handled_tray_payload {
            return LRESULT(1);
        }
        return LRESULT(fwd_result as isize);
    }

    if msg == WM_TIMER {
        let state_ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut TrayProxyState;
        if !state_ptr.is_null() {
            match wparam.0 {
                DEBOUNCE_TIMER_ID => {
                    {
                        let state = unsafe { &mut *state_ptr };
                        state.debounce_armed = false;
                        unsafe {
                            let _ = KillTimer(Some(hwnd), DEBOUNCE_TIMER_ID);
                        }
                        publish_snapshot(state, false);
                    }
                    return LRESULT(0);
                }
                HOUSEKEEP_TIMER_ID => {
                    ensure_spy_hwnd_first(hwnd);
                    let real_now = find_real_explorer_tray_hwnd().map(|h| h.0 as isize);
                    let broadcast_wm = {
                        let state = unsafe { &mut *state_ptr };
                        let wm = if let Some(real_val) = real_now
                            && state.last_explorer_tray != Some(real_val)
                        {
                            state.last_explorer_tray = Some(real_val);
                            take_taskbar_created_message(state, false)
                        } else {
                            None
                        };
                        publish_snapshot(state, false);
                        wm
                    };
                    if let Some(wm) = broadcast_wm {
                        send_taskbar_created_broadcast(wm);
                    }
                    return LRESULT(0);
                }
                _ => {}
            }
        }
    }

    if (matches!(msg, WM_COMMAND | WM_SYSCOMMAND | WM_HOTKEY | WM_APPCOMMAND)
        || (WM_USER..0xC000).contains(&msg))
        && let Some(res) = forward_to_explorer_tray(msg, wparam, lparam)
    {
        return LRESULT(res as isize);
    }

    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

fn enumerate_uia_tray_icons(auto: &IUIAutomation) -> Vec<TrayIconInfo> {
    let Ok(button_cond) = (unsafe {
        auto.CreatePropertyCondition(
            UIA_ControlTypePropertyId,
            &VARIANT::from(UIA_ButtonControlTypeId.0),
        )
    }) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for shell_hwnd in find_foreign_top_level_windows(w!("Shell_TrayWnd")) {
        if unsafe { IsWindowVisible(shell_hwnd) }.as_bool() {
            collect_uia_buttons_in_hwnd(auto, &button_cond, shell_hwnd, false, &mut out);
        }
    }
    for overflow_hwnd in find_foreign_top_level_windows(w!("TopLevelWindowForOverflowXamlIsland")) {
        if unsafe { IsWindowVisible(overflow_hwnd) }.as_bool() {
            let target = find_child_window(
                overflow_hwnd,
                None,
                w!("Windows.UI.Composition.DesktopWindowContentBridge"),
            )
            .unwrap_or(overflow_hwnd);
            collect_uia_buttons_in_hwnd(auto, &button_cond, target, true, &mut out);
        }
    }
    out
}

fn collect_uia_buttons_in_hwnd(
    auto: &IUIAutomation,
    button_cond: &windows::Win32::UI::Accessibility::IUIAutomationCondition,
    hwnd: HWND,
    is_overflow: bool,
    out: &mut Vec<TrayIconInfo>,
) {
    let Ok(root) = (unsafe { auto.ElementFromHandle(hwnd) }) else {
        return;
    };
    let Ok(buttons) = (unsafe { root.FindAll(TreeScope_Subtree, button_cond) }) else {
        return;
    };
    let Ok(len) = (unsafe { buttons.Length() }) else {
        return;
    };

    for idx in 0..len {
        let Ok(el) = (unsafe { buttons.GetElement(idx) }) else {
            continue;
        };
        let auto_id = unsafe { el.CurrentAutomationId() }
            .map(|s| s.to_string())
            .unwrap_or_default();
        let raw_name = unsafe { el.CurrentName() }
            .map(|s| s.to_string())
            .unwrap_or_default();
        let class_name = unsafe { el.CurrentClassName() }
            .map(|s| s.to_string())
            .unwrap_or_default();

        if !is_tray_uia_button(&auto_id, &raw_name, &class_name, is_overflow) {
            continue;
        }

        let cleaned_name = clean_tooltip(&raw_name);
        let tooltip = if cleaned_name.is_empty() {
            auto_id.trim().to_string()
        } else {
            cleaned_name
        };
        if tooltip.is_empty() {
            continue;
        }

        let scope = if is_overflow { "overflow" } else { "bar" };
        let key_part = normalize_tooltip_key(&tooltip);
        let auto_opt = (!auto_id.trim().is_empty()).then(|| auto_id.trim().to_string());
        let id = match auto_opt.as_deref() {
            Some(aid) => format!("uia:{scope}:{aid}:{key_part}"),
            None => format!("uia:{scope}:{key_part}"),
        };

        out.push(TrayIconInfo {
            id,
            tooltip,
            exe_name: String::new(),
            owner_hwnd: 0,
            uid: 0,
            callback_msg: 0,
            callback_version: None,
            uia_automation_id: auto_opt,
            is_overflow,
            icon: None,
        });
    }
}

fn is_tray_uia_button(auto_id: &str, name: &str, class_name: &str, is_overflow: bool) -> bool {
    let auto_trim = auto_id.trim();
    let name_trim = name.trim();
    let auto_lower = auto_trim.to_ascii_lowercase();
    let name_lower = name_trim.to_ascii_lowercase();
    let class_lower = class_name.trim().to_ascii_lowercase();

    if name_lower.is_empty() && !auto_lower.starts_with("notifyitemicon") {
        return false;
    }

    if auto_lower.starts_with("appid:")
        || matches!(
            auto_lower.as_str(),
            "startbutton"
                | "searchbutton"
                | "taskviewbutton"
                | "widgetsbutton"
                | "chatbutton"
                | "copilotbutton"
                | "experiencetogglebutton"
                | "systemtraychevron"
                | "overflowbutton"
                | "notifyiconoverflowbutton"
                | "notificationcenterbutton"
                | "quicksettingsbutton"
                | "controlcenterbutton"
                | "clockbutton"
                | "showdesktopbutton"
        )
        || matches!(
            name_lower.as_str(),
            "start"
                | "search"
                | "task view"
                | "widgets"
                | "show hidden icons"
                | "notification center"
                | "quick settings"
                | "show desktop"
                | "close"
        )
    {
        return false;
    }

    if is_overflow {
        return true;
    }

    auto_lower == "systemtrayicon"
        || auto_lower.starts_with("notifyitemicon")
        || auto_lower.contains("notifyicon")
        || (class_lower.contains("systemtray.") && !name_trim.is_empty())
}

fn clean_tooltip(raw: &str) -> String {
    let before_nul = raw.split('\0').next().unwrap_or("");
    let lines: Vec<String> = before_nul
        .lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|line| !line.is_empty())
        .collect();
    lines.join(" · ")
}

fn normalize_tooltip_key(s: &str) -> String {
    s.split(|c: char| c.is_whitespace() || c == '·' || c == '—' || c == '-')
        .filter(|part| !part.is_empty())
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>()
        .join(" ")
}

fn deduplicate_tray_items(
    primary: Vec<TrayIconInfo>,
    secondary: Vec<TrayIconInfo>,
) -> Vec<TrayIconInfo> {
    let mut out = Vec::with_capacity(primary.len() + secondary.len());
    let mut seen_classic_keys: HashSet<(isize, u32)> = HashSet::new();
    let mut seen_tooltip_keys: HashSet<String> = HashSet::new();
    let mut seen_ids: HashSet<String> = HashSet::new();

    for mut item in primary {
        if item.owner_hwnd != 0
            && !item.id.starts_with("guid:")
            && !seen_classic_keys.insert((item.owner_hwnd, item.uid))
        {
            continue;
        }
        let tip_key = normalize_tooltip_key(&item.tooltip);
        if !tip_key.is_empty() {
            seen_tooltip_keys.insert(tip_key);
        }
        ensure_unique_id(&mut item.id, &mut seen_ids);
        out.push(item);
    }

    for mut item in secondary {
        if item.owner_hwnd != 0
            && !item.id.starts_with("guid:")
            && !seen_classic_keys.insert((item.owner_hwnd, item.uid))
        {
            continue;
        }

        let tip_key = normalize_tooltip_key(&item.tooltip);
        if !tip_key.is_empty() && seen_tooltip_keys.contains(&tip_key) {
            if let Some(existing) = out
                .iter_mut()
                .find(|c| normalize_tooltip_key(&c.tooltip) == tip_key)
                && existing.uia_automation_id.is_none()
                && item.uia_automation_id.is_some()
            {
                existing.uia_automation_id = item.uia_automation_id;
            }
            continue;
        }

        if let Some(aid) = item.uia_automation_id.as_deref()
            && !aid.eq_ignore_ascii_case("NotifyItemIcon")
            && !aid.eq_ignore_ascii_case("SystemTrayIcon")
            && out
                .iter()
                .any(|existing| existing.uia_automation_id.as_deref() == Some(aid))
        {
            continue;
        }

        if !tip_key.is_empty() {
            seen_tooltip_keys.insert(tip_key);
        }
        ensure_unique_id(&mut item.id, &mut seen_ids);
        out.push(item);
    }

    out
}

fn ensure_unique_id(id: &mut String, seen_ids: &mut HashSet<String>) {
    if seen_ids.insert(id.clone()) {
        return;
    }
    let base = id.clone();
    let mut suffix = 2usize;
    loop {
        let candidate = format!("{base}#{suffix}");
        if seen_ids.insert(candidate.clone()) {
            *id = candidate;
            return;
        }
        suffix += 1;
    }
}

fn pack_cursor_xy(cursor_xy: (i32, i32)) -> usize {
    let x = cursor_xy.0.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16 as u16 as usize;
    let y = cursor_xy.1.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16 as u16 as usize;
    x | (y << 16)
}

fn pack_v4_lparam(event_msg: u32, uid: u32) -> isize {
    let lo = (event_msg & 0xFFFF) as usize;
    let hi = (uid & 0xFFFF) as usize;
    (lo | (hi << 16)) as isize
}

/// Activates a mirrored tray icon via its Win32 callback message or UIA Invoke pattern.
pub fn activate_tray_icon(info: &TrayIconInfo, click: TrayClickKind, cursor_xy: (i32, i32)) {
    if info.owner_hwnd != 0 {
        let hwnd = HWND(info.owner_hwnd as usize as *mut c_void);
        if unsafe { IsWindow(Some(hwnd)) }.as_bool() {
            let mut owner_pid: u32 = 0;
            unsafe {
                GetWindowThreadProcessId(hwnd, Some(&mut owner_pid));
                if owner_pid != 0 {
                    let _ = AllowSetForegroundWindow(owner_pid);
                }
                let _ = SetForegroundWindow(hwnd);
            }

            if info.callback_msg != 0 {
                post_tray_callback_messages(
                    hwnd,
                    info.callback_msg,
                    info.uid,
                    info.callback_version,
                    click,
                    cursor_xy,
                );
                return;
            }
        }
    }

    if info.uia_automation_id.is_some() || !info.tooltip.is_empty() {
        let _ = invoke_uia_tray_item(info);
    }
}

fn post_tray_callback_messages(
    hwnd: HWND,
    callback_msg: u32,
    uid: u32,
    callback_version: Option<u32>,
    click: TrayClickKind,
    cursor_xy: (i32, i32),
) {
    let use_v4 = callback_version.is_some_and(|v| v >= 4);

    if use_v4 {
        let v4_wparam = WPARAM(pack_cursor_xy(cursor_xy));
        let events: &[u32] = match click {
            TrayClickKind::LeftClick => &[WM_LBUTTONDOWN, WM_LBUTTONUP, NIN_SELECT],
            TrayClickKind::RightClick => &[WM_RBUTTONDOWN, WM_RBUTTONUP, WM_CONTEXTMENU],
            TrayClickKind::DoubleClick => &[WM_LBUTTONDBLCLK, WM_LBUTTONUP, NIN_SELECT],
        };
        for &ev in events {
            unsafe {
                let _ = PostMessageW(
                    Some(hwnd),
                    callback_msg,
                    v4_wparam,
                    LPARAM(pack_v4_lparam(ev, uid)),
                );
            }
        }
    } else {
        let std_wparam = WPARAM(uid as usize);
        match click {
            TrayClickKind::LeftClick => {
                for ev in [WM_LBUTTONDOWN, WM_LBUTTONUP] {
                    unsafe {
                        let _ =
                            PostMessageW(Some(hwnd), callback_msg, std_wparam, LPARAM(ev as isize));
                    }
                }
            }
            TrayClickKind::DoubleClick => {
                for ev in [WM_LBUTTONDBLCLK, WM_LBUTTONUP] {
                    unsafe {
                        let _ =
                            PostMessageW(Some(hwnd), callback_msg, std_wparam, LPARAM(ev as isize));
                    }
                }
            }
            TrayClickKind::RightClick => {
                for ev in [WM_RBUTTONDOWN, WM_RBUTTONUP] {
                    unsafe {
                        let _ =
                            PostMessageW(Some(hwnd), callback_msg, std_wparam, LPARAM(ev as isize));
                    }
                }
                if callback_version == Some(3) {
                    unsafe {
                        let _ = PostMessageW(
                            Some(hwnd),
                            callback_msg,
                            std_wparam,
                            LPARAM(WM_CONTEXTMENU as isize),
                        );
                    }
                }
            }
        }
    }
}

fn invoke_uia_tray_item(info: &TrayIconInfo) -> bool {
    let _com = ComGuard::init_sta();
    let Ok(auto) = (unsafe {
        CoCreateInstance::<_, IUIAutomation>(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
    }) else {
        return false;
    };
    let Ok(button_cond) = (unsafe {
        auto.CreatePropertyCondition(
            UIA_ControlTypePropertyId,
            &VARIANT::from(UIA_ButtonControlTypeId.0),
        )
    }) else {
        return false;
    };

    let primary_class = if info.is_overflow {
        w!("TopLevelWindowForOverflowXamlIsland")
    } else {
        w!("Shell_TrayWnd")
    };
    let secondary_class = if info.is_overflow {
        w!("Shell_TrayWnd")
    } else {
        w!("TopLevelWindowForOverflowXamlIsland")
    };

    let target_key = normalize_tooltip_key(&info.tooltip);
    let target_aid = info.uia_automation_id.as_deref();

    for class_name in [primary_class, secondary_class] {
        for hwnd in find_foreign_top_level_windows(class_name) {
            let target_hwnd = if class_name == w!("TopLevelWindowForOverflowXamlIsland") {
                find_child_window(
                    hwnd,
                    None,
                    w!("Windows.UI.Composition.DesktopWindowContentBridge"),
                )
                .unwrap_or(hwnd)
            } else {
                hwnd
            };
            if let Some(el) =
                find_matching_uia_button(&auto, &button_cond, target_hwnd, target_aid, &target_key)
                && invoke_uia_element(&el)
            {
                return true;
            }
        }
    }

    false
}

fn find_matching_uia_button(
    auto: &IUIAutomation,
    button_cond: &windows::Win32::UI::Accessibility::IUIAutomationCondition,
    hwnd: HWND,
    target_aid: Option<&str>,
    target_key: &str,
) -> Option<IUIAutomationElement> {
    let root = unsafe { auto.ElementFromHandle(hwnd) }.ok()?;
    let buttons = unsafe { root.FindAll(TreeScope_Subtree, button_cond) }.ok()?;
    let len = unsafe { buttons.Length() }.ok()?;

    let mut aid_fallback: Option<IUIAutomationElement> = None;
    let mut name_fallback: Option<IUIAutomationElement> = None;

    for idx in 0..len {
        let Ok(el) = (unsafe { buttons.GetElement(idx) }) else {
            continue;
        };
        let aid = unsafe { el.CurrentAutomationId() }
            .map(|s| s.to_string())
            .unwrap_or_default();
        let name = unsafe { el.CurrentName() }
            .map(|s| s.to_string())
            .unwrap_or_default();
        let name_key = normalize_tooltip_key(&clean_tooltip(&name));

        let aid_matches = target_aid.is_some_and(|t| !t.is_empty() && aid.trim() == t);
        let name_matches = !target_key.is_empty() && name_key == target_key;

        if aid_matches && name_matches {
            return Some(el);
        }
        if aid_matches
            && aid.trim() != "NotifyItemIcon"
            && aid.trim() != "SystemTrayIcon"
            && aid_fallback.is_none()
        {
            aid_fallback = Some(el);
        } else if name_matches && name_fallback.is_none() {
            name_fallback = Some(el);
        }
    }

    aid_fallback.or(name_fallback)
}

fn invoke_uia_element(el: &IUIAutomationElement) -> bool {
    if let Ok(inv) =
        unsafe { el.GetCurrentPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId) }
        && unsafe { inv.Invoke() }.is_ok()
    {
        return true;
    }
    if let Ok(exp) = unsafe {
        el.GetCurrentPatternAs::<IUIAutomationExpandCollapsePattern>(UIA_ExpandCollapsePatternId)
    } && unsafe { exp.Expand() }.is_ok()
    {
        return true;
    }
    false
}

/// Background watcher running a transparent hidden `Shell_TrayWnd` proxy and emitting
/// `Vec<TrayIconInfo>` snapshots whenever tray icons change.
pub struct TrayTracker {
    stop: Arc<AtomicBool>,
    thread_id: u32,
    thread: Option<JoinHandle<()>>,
}

impl TrayTracker {
    pub fn start(tx: UnboundedSender<Vec<TrayIconInfo>>) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = Arc::clone(&stop);
        let (tid_tx, tid_rx) = std::sync::mpsc::channel::<u32>();

        let thread = std::thread::Builder::new()
            .name("vita-tray-tracker".into())
            .spawn(move || {
                run_tray_proxy_thread(tx, stop_thread, tid_tx);
            })
            .ok();

        let thread_id = tid_rx.recv().unwrap_or(0);
        Self {
            stop,
            thread_id,
            thread,
        }
    }

    pub fn refresh(&self) {
        if self.thread_id != 0 {
            unsafe {
                let _ = PostThreadMessageW(self.thread_id, WM_TRAY_REFRESH, WPARAM(0), LPARAM(0));
            }
        }
    }
}

impl Drop for TrayTracker {
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

fn run_tray_proxy_thread(
    tx: UnboundedSender<Vec<TrayIconInfo>>,
    stop_thread: Arc<AtomicBool>,
    tid_tx: std::sync::mpsc::Sender<u32>,
) {
    // Force Win32 message queue creation before publishing tid so Drop cannot lose WM_QUIT.
    let mut init_msg = MSG::default();
    unsafe {
        let _ = PeekMessageW(&mut init_msg, None, WM_USER, WM_USER, PM_NOREMOVE);
    }
    let tid = unsafe { GetCurrentThreadId() };
    let _com = ComGuard::init_sta();

    let hinst = unsafe { GetModuleHandleW(None) }.unwrap_or_default();
    let class_name = w!("Shell_TrayWnd");
    let wc = WNDCLASSEXW {
        cbSize: size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(tray_proxy_wnd_proc),
        hInstance: hinst.into(),
        lpszClassName: class_name,
        ..Default::default()
    };
    let atom = unsafe { RegisterClassExW(&wc) };
    let registered_here = if atom != 0 {
        true
    } else {
        std::io::Error::last_os_error().raw_os_error() == Some(ERROR_CLASS_ALREADY_EXISTS.0 as i32)
    };

    let spy_hwnd = if registered_here {
        unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST,
                class_name,
                w!(""),
                WS_POPUP,
                0,
                0,
                0,
                0,
                None,
                None,
                Some(hinst.into()),
                None,
            )
        }
        .ok()
        .filter(|h| !h.0.is_null())
    } else {
        None
    };

    let wm_taskbar_created = unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) };
    let state_ptr: *mut TrayProxyState = Box::into_raw(Box::new(TrayProxyState {
        spy_hwnd: spy_hwnd.unwrap_or_default(),
        wm_taskbar_created,
        entries: Vec::new(),
        next_order: 0,
        tx,
        last_emitted: Vec::new(),
        last_explorer_tray: find_real_explorer_tray_hwnd().map(|h| h.0 as isize),
        last_broadcast_at: None,
        debounce_armed: false,
    }));

    if let Some(hwnd) = spy_hwnd {
        unsafe {
            let _ = SetWindowLongPtrW(hwnd, GWLP_USERDATA, state_ptr as isize);
            let _ = SetWindowPos(
                hwnd,
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
            let _ = ChangeWindowMessageFilterEx(hwnd, WM_COPYDATA, MSGFLT_ALLOW, None);
        }
    }

    PROXY_THREAD_ID.store(tid, Ordering::Release);
    let _ = tid_tx.send(tid);

    if !stop_thread.load(Ordering::Acquire) {
        let initial_wm = {
            let state = unsafe { &mut *state_ptr };
            publish_snapshot(state, true);
            if spy_hwnd.is_some() {
                take_taskbar_created_message(state, true)
            } else {
                None
            }
        };
        if let Some(wm) = initial_wm {
            send_taskbar_created_broadcast(wm);
        }
        if spy_hwnd.is_some() {
            unsafe {
                let _ = SetTimer(spy_hwnd, HOUSEKEEP_TIMER_ID, HOUSEKEEP_MS, None);
            }
        }

        let mut msg = MSG::default();
        while !stop_thread.load(Ordering::Acquire) {
            let res = unsafe { GetMessageW(&mut msg, None, 0, 0) };
            if res.0 <= 0 {
                break;
            }
            if msg.hwnd.0.is_null() && msg.message == WM_TRAY_REFRESH {
                if let Some(hwnd) = spy_hwnd {
                    ensure_spy_hwnd_first(hwnd);
                }
                let refresh_wm = {
                    let state = unsafe { &mut *state_ptr };
                    let wm = if spy_hwnd.is_some() {
                        take_taskbar_created_message(state, false)
                    } else {
                        None
                    };
                    publish_snapshot(state, false);
                    wm
                };
                if let Some(wm) = refresh_wm {
                    send_taskbar_created_broadcast(wm);
                }
                continue;
            }
            unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }

    PROXY_THREAD_ID.store(0, Ordering::Release);
    if let Some(hwnd) = spy_hwnd {
        unsafe {
            let _ = KillTimer(Some(hwnd), DEBOUNCE_TIMER_ID);
            let _ = KillTimer(Some(hwnd), HOUSEKEEP_TIMER_ID);
            let _ = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            let _ = DestroyWindow(hwnd);
        }
    }
    if atom != 0 {
        unsafe {
            let _ = UnregisterClassW(class_name, Some(hinst.into()));
        }
    }
    if let Ok(mut store) = proxy_snapshot_store().lock() {
        store.clear();
    }
    unsafe {
        drop(Box::from_raw(state_ptr));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::offset_of;
    use std::sync::mpsc;
    use tokio::sync::mpsc::unbounded_channel;
    use windows::Win32::UI::Shell::{
        NIF_GUID as WIN_NIF_GUID, NIF_ICON as WIN_NIF_ICON, NIF_MESSAGE as WIN_NIF_MESSAGE,
        NIF_STATE as WIN_NIF_STATE, NIF_TIP as WIN_NIF_TIP, NIM_ADD as WIN_NIM_ADD,
        NIM_DELETE as WIN_NIM_DELETE, NIM_MODIFY as WIN_NIM_MODIFY,
        NIM_SETVERSION as WIN_NIM_SETVERSION, NIS_HIDDEN as WIN_NIS_HIDDEN, NOTIFY_ICON_STATE,
        NOTIFYICONDATAW, Shell_NotifyIconW,
    };
    use windows::Win32::UI::WindowsAndMessaging::{IDI_APPLICATION, LoadIconW};

    fn sample_icon(id: &str, tooltip: &str, owner_hwnd: isize, uid: u32) -> TrayIconInfo {
        TrayIconInfo {
            id: id.to_string(),
            tooltip: tooltip.to_string(),
            exe_name: "app.exe".to_string(),
            owner_hwnd,
            uid,
            callback_msg: 0x0401,
            callback_version: None,
            uia_automation_id: None,
            is_overflow: false,
            icon: None,
        }
    }

    #[test]
    fn tbbutton64_layout_size() {
        assert_eq!(size_of::<TbButton64>(), 32);
        assert_eq!(offset_of!(TbButton64, i_bitmap), 0);
        assert_eq!(offset_of!(TbButton64, id_command), 4);
        assert_eq!(offset_of!(TbButton64, fs_state), 8);
        assert_eq!(offset_of!(TbButton64, fs_style), 9);
        assert_eq!(offset_of!(TbButton64, dw_data), 16);
        assert_eq!(offset_of!(TbButton64, i_string), 24);

        assert_eq!(size_of::<TrayData64>(), 32);
        assert_eq!(offset_of!(TrayData64, hwnd), 0);
        assert_eq!(offset_of!(TrayData64, uid), 8);
        assert_eq!(offset_of!(TrayData64, callback_msg), 12);
        assert_eq!(offset_of!(TrayData64, hicon), 24);

        #[cfg(target_pointer_width = "64")]
        assert_eq!(
            size_of::<windows::Win32::UI::Controls::TBBUTTON>(),
            size_of::<TbButton64>()
        );
    }

    #[test]
    fn enumerate_tray_icons_never_panics() {
        let _lock = SHELL_TRAY_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let icons = enumerate_tray_icons();
        let mut ids = HashSet::new();
        for item in icons {
            assert!(!item.id.is_empty());
            assert!(ids.insert(item.id));
        }
    }

    #[test]
    fn parse_shell_tray_data_matches_win11_wire_offsets() {
        let mut buf = vec![0u8; 1484];
        buf[0x00..0x04].copy_from_slice(&SHELLTRAYDATA_SIGNATURE.to_le_bytes());
        buf[0x04..0x08].copy_from_slice(&NIM_ADD.to_le_bytes());
        buf[0x08..0x0C].copy_from_slice(&956u32.to_le_bytes());
        buf[0x0C..0x10].copy_from_slice(&0x0057_0982u32.to_le_bytes());
        buf[0x10..0x14].copy_from_slice(&0x1234u32.to_le_bytes());
        buf[0x14..0x18].copy_from_slice(
            &(NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_STATE | NIF_GUID).to_le_bytes(),
        );
        buf[0x18..0x1C].copy_from_slice(&0x0456u32.to_le_bytes());
        buf[0x1C..0x20].copy_from_slice(&0x0001_002Bu32.to_le_bytes());

        let tip_utf16: Vec<u16> = "HelloTray\0".encode_utf16().collect();
        for (i, u) in tip_utf16.iter().enumerate() {
            buf[0x20 + i * 2..0x20 + i * 2 + 2].copy_from_slice(&u.to_le_bytes());
        }

        buf[0x120..0x124].copy_from_slice(&NIS_HIDDEN.to_le_bytes());
        buf[0x124..0x128].copy_from_slice(&0x03u32.to_le_bytes());
        buf[0x328..0x32C].copy_from_slice(&4u32.to_le_bytes());

        let guid: [u8; 16] = [
            0x44, 0x33, 0x22, 0x11, 0x66, 0x55, 0x88, 0x77, 0x99, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE,
            0xFF, 0x00,
        ];
        buf[0x3B0..0x3C0].copy_from_slice(&guid);

        let exe_utf16: Vec<u16> = r"{6D809377-6AF0-444B-8957-A3773F02200E}\App\widget.exe"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        for (i, u) in exe_utf16.iter().enumerate() {
            buf[0x3C4 + i * 2..0x3C4 + i * 2 + 2].copy_from_slice(&u.to_le_bytes());
        }

        let parsed = parse_shell_tray_data(&buf).expect("valid wire payload");
        assert_eq!(parsed.dw_message, NIM_ADD);
        assert_eq!(parsed.cb_size, 956);
        assert_eq!(parsed.owner_hwnd, 0x0057_0982);
        assert_eq!(parsed.uid, 0x1234);
        assert_eq!(parsed.callback_msg, 0x0456);
        assert_eq!(parsed.hicon_raw, 0x0001_002B);
        assert_eq!(parsed.tooltip.as_deref(), Some("HelloTray"));
        assert_eq!(parsed.dw_state, NIS_HIDDEN);
        assert_eq!(parsed.dw_state_mask, 0x03);
        assert_eq!(parsed.u_timeout_or_version, Some(4));
        assert_eq!(parsed.guid_item, Some(guid));
        assert_eq!(
            parsed.caller_exe_raw.as_deref(),
            Some(r"{6D809377-6AF0-444B-8957-A3773F02200E}\App\widget.exe")
        );

        // Wrong signature must be rejected cleanly.
        let mut bad = buf.clone();
        bad[0] = 0;
        assert!(parse_shell_tray_data(&bad).is_none());
    }

    #[test]
    fn expand_known_folder_and_exe_name_fallback() {
        let pf = expand_known_folder_path(
            r"{6D809377-6AF0-444B-8957-A3773F02200E}\Bitdefender\bdagent.exe",
        );
        assert!(
            pf.ends_with(r"\Bitdefender\bdagent.exe"),
            "unexpected expanded ProgramFiles path: {pf}"
        );
        assert!(!pf.starts_with('{'));
        assert_eq!(exe_name_from_path(&pf), "bdagent.exe");

        let sys32 = expand_known_folder_path(
            r"{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\WindowsPowerShell\v1.0\powershell.exe",
        );
        assert!(
            sys32.ends_with(r"\System32\WindowsPowerShell\v1.0\powershell.exe"),
            "unexpected expanded System32 path: {sys32}"
        );
        assert_eq!(exe_name_from_path(&sys32), "powershell.exe");
    }

    #[test]
    fn tooltip_cleaning_and_normalization_handle_multiline_text() {
        assert_eq!(
            clean_tooltip("  OneDrive - Personal \r\n Up to date \0extra"),
            "OneDrive - Personal · Up to date"
        );
        assert_eq!(clean_tooltip("\r\n\t\0"), "");
        assert_eq!(
            normalize_tooltip_key("OneDrive - Personal · Up to date"),
            normalize_tooltip_key("onedrive   personal\nup to date")
        );
    }

    #[test]
    fn deduplicate_merges_classic_and_uia_without_losing_hwnd_items() {
        let classic = vec![
            sample_icon("win32:100:1", "OneDrive · Up to date", 0x100, 1),
            sample_icon("win32:100:1", "OneDrive · Up to date", 0x100, 1),
        ];
        let uia = vec![
            TrayIconInfo {
                id: "uia:bar:NotifyItemIcon_1:onedrive up to date".to_string(),
                tooltip: "OneDrive - Up to date".to_string(),
                exe_name: String::new(),
                owner_hwnd: 0,
                uid: 0,
                callback_msg: 0,
                callback_version: None,
                uia_automation_id: Some("NotifyItemIcon_1".to_string()),
                is_overflow: false,
                icon: None,
            },
            TrayIconInfo {
                id: "uia:overflow:NotifyItemIcon:bluetooth".to_string(),
                tooltip: "Bluetooth Devices".to_string(),
                exe_name: String::new(),
                owner_hwnd: 0,
                uid: 0,
                callback_msg: 0,
                callback_version: None,
                uia_automation_id: Some("NotifyItemIcon".to_string()),
                is_overflow: true,
                icon: None,
            },
            TrayIconInfo {
                id: "uia:overflow:NotifyItemIcon:vpn".to_string(),
                tooltip: "WireGuard".to_string(),
                exe_name: String::new(),
                owner_hwnd: 0,
                uid: 0,
                callback_msg: 0,
                callback_version: None,
                uia_automation_id: Some("NotifyItemIcon".to_string()),
                is_overflow: true,
                icon: None,
            },
        ];

        let merged = deduplicate_tray_items(classic, uia);
        assert_eq!(merged.len(), 3);
        assert_eq!(merged[0].owner_hwnd, 0x100);
        assert_eq!(
            merged[0].uia_automation_id.as_deref(),
            Some("NotifyItemIcon_1")
        );
        assert_eq!(merged[1].tooltip, "Bluetooth Devices");
        assert_eq!(merged[2].tooltip, "WireGuard");
    }

    #[test]
    fn uia_button_filter_accepts_win11_system_tray_icon_and_rejects_chrome() {
        assert!(!is_tray_uia_button(
            "StartButton",
            "Start",
            "ToggleButton",
            false
        ));
        assert!(!is_tray_uia_button(
            "Appid:Microsoft.WindowsTerminal",
            "Terminal",
            "TaskListButton",
            false
        ));
        assert!(!is_tray_uia_button(
            "SystemTrayChevron",
            "Show hidden icons",
            "SystemTray.NormalButton",
            false
        ));
        assert!(!is_tray_uia_button(
            "SystemTrayIcon",
            "Show hidden icons",
            "SystemTray.OmniButton",
            false
        ));
        assert!(is_tray_uia_button(
            "SystemTrayIcon",
            "Bitdefender Total Security",
            "SystemTray.NormalButton",
            false
        ));
        assert!(is_tray_uia_button(
            "NotifyItemIcon_42",
            "Discord",
            "SystemTray.NormalButton",
            false
        ));
        assert!(is_tray_uia_button(
            "",
            "Safely Remove Hardware",
            "Button",
            true
        ));
    }

    #[test]
    fn v4_packing_preserves_signed_coordinates_and_uid() {
        let xy = pack_cursor_xy((-120, 480));
        let x = (xy & 0xFFFF) as u16 as i16;
        let y = ((xy >> 16) & 0xFFFF) as u16 as i16;
        assert_eq!((x, y), (-120, 480));

        let lp = pack_v4_lparam(WM_CONTEXTMENU, 7) as usize;
        assert_eq!((lp & 0xFFFF) as u32, WM_CONTEXTMENU);
        assert_eq!(((lp >> 16) & 0xFFFF) as u32, 7);
    }

    #[test]
    fn deduplicate_handles_classic_only_snapshots_cleanly() {
        let classic = vec![
            sample_icon("win32:100:1", "Tailscale · Connected", 0x100, 1),
            sample_icon("win32:100:1", "Tailscale · Connected", 0x100, 1),
            sample_icon("win32:200:2", "Docker Desktop", 0x200, 2),
        ];
        let deduped = deduplicate_tray_items(classic, Vec::new());
        assert_eq!(deduped.len(), 2);
        assert_eq!(deduped[0].owner_hwnd, 0x100);
        assert_eq!(deduped[1].owner_hwnd, 0x200);
    }

    #[test]
    fn shell_tray_proxy_end_to_end_real_shell_notify_icon() {
        let _lock = SHELL_TRAY_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());

        let (tx, mut rx) = unbounded_channel();
        let tracker = TrayTracker::start(tx);

        // Drain initial startup snapshot.
        let _ = rx.blocking_recv();

        const TEST_CALLBACK_MSG: u32 = WM_USER + 77;
        const TEST_UID: u32 = 0x51A7;

        let (cb_tx, cb_rx) = mpsc::channel::<(usize, isize)>();
        let cb_tx_ptr = Box::into_raw(Box::new(cb_tx));

        unsafe extern "system" fn test_owner_wnd_proc(
            hwnd: HWND,
            msg: u32,
            wparam: WPARAM,
            lparam: LPARAM,
        ) -> LRESULT {
            if msg == WM_USER + 77 {
                let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) }
                    as *const mpsc::Sender<(usize, isize)>;
                if !ptr.is_null() {
                    let sender = unsafe { &*ptr };
                    let _ = sender.send((wparam.0, lparam.0));
                }
                return LRESULT(0);
            }
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }

        let hinst = unsafe { GetModuleHandleW(None) }.unwrap_or_default();
        let owner_cls = w!("VitaTrayE2ETestOwner");
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(test_owner_wnd_proc),
            hInstance: hinst.into(),
            lpszClassName: owner_cls,
            ..Default::default()
        };
        let atom = unsafe { RegisterClassExW(&wc) };
        let owner_hwnd = unsafe {
            CreateWindowExW(
                Default::default(),
                owner_cls,
                w!(""),
                WS_POPUP,
                0,
                0,
                0,
                0,
                None,
                None,
                Some(hinst.into()),
                None,
            )
        }
        .expect("create test tray owner window");
        unsafe {
            let _ = SetWindowLongPtrW(owner_hwnd, GWLP_USERDATA, cb_tx_ptr as isize);
        }

        let test_hicon = unsafe { LoadIconW(None, IDI_APPLICATION) }.expect("load IDI_APPLICATION");
        let owner_isize = owner_hwnd.0 as isize;

        let wait_for_match = |rx: &mut tokio::sync::mpsc::UnboundedReceiver<Vec<TrayIconInfo>>,
                              pred: &dyn Fn(&[TrayIconInfo]) -> bool|
         -> Vec<TrayIconInfo> {
            let deadline = Instant::now() + Duration::from_secs(3);
            while Instant::now() < deadline {
                if let Ok(snap) = rx.try_recv()
                    && pred(&snap)
                {
                    return snap;
                }
                let remaining = deadline.saturating_duration_since(Instant::now());
                std::thread::sleep(remaining.min(Duration::from_millis(25)));
            }
            let final_snap = enumerate_tray_icons();
            assert!(
                pred(&final_snap),
                "timed out waiting for expected tray snapshot: {final_snap:?}"
            );
            final_snap
        };

        let fill_utf16 = |dst: &mut [u16], s: &str| {
            dst.fill(0);
            for (i, u) in s
                .encode_utf16()
                .take(dst.len().saturating_sub(1))
                .enumerate()
            {
                dst[i] = u;
            }
        };

        let mut nid = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: owner_hwnd,
            uID: TEST_UID,
            uFlags: WIN_NIF_MESSAGE | WIN_NIF_ICON | WIN_NIF_TIP,
            uCallbackMessage: TEST_CALLBACK_MSG,
            hIcon: test_hicon,
            ..Default::default()
        };
        fill_utf16(&mut nid.szTip, "VitaTestIcon");

        assert!(unsafe { Shell_NotifyIconW(WIN_NIM_ADD, &nid) }.as_bool());

        let snap_add = wait_for_match(&mut rx, &|list| {
            list.iter().any(|i| {
                i.owner_hwnd == owner_isize && i.uid == TEST_UID && i.tooltip == "VitaTestIcon"
            })
        });
        let added = snap_add
            .iter()
            .find(|i| i.owner_hwnd == owner_isize && i.uid == TEST_UID)
            .expect("added item present");
        assert!(
            added.icon.is_some(),
            "expected RGBA icon extracted for HICON"
        );

        // NIM_SETVERSION(4)
        nid.Anonymous.uVersion = 4;
        assert!(unsafe { Shell_NotifyIconW(WIN_NIM_SETVERSION, &nid) }.as_bool());

        // NIM_MODIFY tooltip
        nid.uFlags = WIN_NIF_TIP;
        fill_utf16(&mut nid.szTip, "VitaTestUpdated");
        assert!(unsafe { Shell_NotifyIconW(WIN_NIM_MODIFY, &nid) }.as_bool());

        let snap_mod = wait_for_match(&mut rx, &|list| {
            list.iter().any(|i| {
                i.owner_hwnd == owner_isize
                    && i.uid == TEST_UID
                    && i.tooltip == "VitaTestUpdated"
                    && i.callback_version == Some(4)
            })
        });
        let updated_item = snap_mod
            .iter()
            .find(|i| i.owner_hwnd == owner_isize && i.uid == TEST_UID)
            .cloned()
            .expect("updated item present");

        // Hide via NIS_HIDDEN -> disappears from snapshot
        nid.uFlags = WIN_NIF_STATE;
        nid.dwState = WIN_NIS_HIDDEN;
        nid.dwStateMask = WIN_NIS_HIDDEN;
        assert!(unsafe { Shell_NotifyIconW(WIN_NIM_MODIFY, &nid) }.as_bool());
        wait_for_match(&mut rx, &|list| {
            !list
                .iter()
                .any(|i| i.owner_hwnd == owner_isize && i.uid == TEST_UID)
        });

        // Unhide -> reappears with updated tooltip preserved
        nid.dwState = NOTIFY_ICON_STATE(0);
        assert!(unsafe { Shell_NotifyIconW(WIN_NIM_MODIFY, &nid) }.as_bool());
        wait_for_match(&mut rx, &|list| {
            list.iter().any(|i| {
                i.owner_hwnd == owner_isize && i.uid == TEST_UID && i.tooltip == "VitaTestUpdated"
            })
        });

        // Activate via RightClick -> verify v4 WM_RBUTTONDOWN, WM_RBUTTONUP, WM_CONTEXTMENU
        activate_tray_icon(&updated_item, TrayClickKind::RightClick, (123, 456));
        let mut msg = MSG::default();
        let pump_until = Instant::now() + Duration::from_millis(300);
        let mut received = Vec::new();
        while Instant::now() < pump_until && received.len() < 3 {
            while unsafe {
                PeekMessageW(
                    &mut msg,
                    Some(owner_hwnd),
                    0,
                    0,
                    windows::Win32::UI::WindowsAndMessaging::PM_REMOVE,
                )
            }
            .as_bool()
            {
                unsafe {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            while let Ok(pair) = cb_rx.try_recv() {
                received.push(pair);
            }
            if received.len() < 3 {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        assert_eq!(
            received.len(),
            3,
            "expected 3 v4 right-click callbacks: {received:?}"
        );
        assert_eq!(received[0].0, pack_cursor_xy((123, 456)));
        assert_eq!(received[0].1, pack_v4_lparam(WM_RBUTTONDOWN, TEST_UID));
        assert_eq!(received[1].1, pack_v4_lparam(WM_RBUTTONUP, TEST_UID));
        assert_eq!(received[2].1, pack_v4_lparam(WM_CONTEXTMENU, TEST_UID));

        // NIM_DELETE -> removed
        assert!(unsafe { Shell_NotifyIconW(WIN_NIM_DELETE, &nid) }.as_bool());
        wait_for_match(&mut rx, &|list| {
            !list
                .iter()
                .any(|i| i.owner_hwnd == owner_isize && i.uid == TEST_UID)
        });

        // Verify enumerate_tray_icons + drop
        drop(tracker);
        unsafe {
            let _ = SetWindowLongPtrW(owner_hwnd, GWLP_USERDATA, 0);
            let _ = DestroyWindow(owner_hwnd);
            if atom != 0 {
                let _ = UnregisterClassW(owner_cls, Some(hinst.into()));
            }
            drop(Box::from_raw(cb_tx_ptr));
        }

        // Prevent unused constant warnings in test scope
        let _ = WIN_NIF_GUID;
    }
}
