use crate::windows::{cached_window_icon, hbitmap_to_rgba, process_exe_info};
use std::collections::HashSet;
use std::ffi::c_void;
use std::mem::size_of;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use tokio::sync::mpsc::UnboundedSender;
use vita_core::RgbaIcon;
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Gdi::DeleteObject;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};
use windows::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows::Win32::System::Memory::{
    MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE, VirtualAllocEx, VirtualFreeEx,
};
use windows::Win32::System::Threading::{
    GetCurrentThreadId, OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_OPERATION,
    PROCESS_VM_READ,
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
    AllowSetForegroundWindow, DispatchMessageW, FindWindowExW, GetIconInfo, GetMessageW,
    GetWindowThreadProcessId, HICON, ICONINFO, IsWindow, KillTimer, MSG, PM_NOREMOVE, PeekMessageW,
    PostMessageW, PostThreadMessageW, SMTO_ABORTIFHUNG, SendMessageTimeoutW, SetForegroundWindow,
    SetTimer, TranslateMessage, WM_CONTEXTMENU, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_QUIT, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_TIMER, WM_USER,
};
use windows::core::{PCWSTR, w};

const REMOTE_BUF_SIZE: usize = 4096;
const MAX_TOOLBAR_BUTTONS: usize = 256;
const WM_TRAY_REFRESH: u32 = WM_USER + 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayIconInfo {
    pub id: String,
    pub tooltip: String,
    pub exe_name: String,
    pub owner_hwnd: isize,
    pub uid: u32,
    pub callback_msg: u32,
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

/// Enumerates active system tray icons from both classic Win32 `ToolbarWindow32` hosts
/// and Windows 11 XAML UI Automation surfaces, even when `Shell_TrayWnd` is hidden.
pub fn enumerate_tray_icons() -> Vec<TrayIconInfo> {
    let _com = ComGuard::init_sta();
    let uia: Option<IUIAutomation> =
        unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) }.ok();
    let items = enumerate_tray_icons_with_uia(uia.as_ref());
    drop(uia);
    items
}

fn enumerate_tray_icons_with_uia(uia: Option<&IUIAutomation>) -> Vec<TrayIconInfo> {
    let classic = enumerate_classic_tray_icons();
    let uia_items = uia.map(enumerate_uia_tray_icons).unwrap_or_default();

    // Ensure classic entries remain uniquely ordered and merge UIA fallback entries.
    deduplicate_tray_items(classic, uia_items)
}

fn find_top_level_windows(class_name: PCWSTR) -> Vec<HWND> {
    let mut out = Vec::new();
    let mut prev: Option<HWND> = None;
    for _ in 0..8 {
        let Ok(hwnd) = (unsafe { FindWindowExW(None, prev, class_name, None) }) else {
            break;
        };
        if hwnd.0.is_null() {
            break;
        }
        out.push(hwnd);
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

    for shell_tray in find_top_level_windows(w!("Shell_TrayWnd")) {
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

    for overflow in find_top_level_windows(w!("NotifyIconOverflowWindow")) {
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

        let icon = hicon_raw_to_rgba(tray.hicon)
            .or_else(|| cached_window_icon(owner_hwnd, &exe_path));

        let owner_isize = owner_hwnd.0 as isize;
        out.push(TrayIconInfo {
            id: format!("win32:{owner_isize:x}:{}", tray.uid),
            tooltip,
            exe_name,
            owner_hwnd: owner_isize,
            uid: tray.uid,
            callback_msg: tray.callback_msg,
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

fn hicon_raw_to_rgba(raw_hicon: u64) -> Option<RgbaIcon> {
    if raw_hicon == 0 {
        return None;
    }
    let hicon = HICON(raw_hicon as usize as *mut c_void);
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
    for shell_hwnd in find_top_level_windows(w!("Shell_TrayWnd")) {
        collect_uia_buttons_in_hwnd(auto, &button_cond, shell_hwnd, false, &mut out);
    }
    for overflow_hwnd in find_top_level_windows(w!("TopLevelWindowForOverflowXamlIsland")) {
        collect_uia_buttons_in_hwnd(auto, &button_cond, overflow_hwnd, true, &mut out);
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

    auto_lower.starts_with("notifyitemicon")
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
    classic: Vec<TrayIconInfo>,
    uia: Vec<TrayIconInfo>,
) -> Vec<TrayIconInfo> {
    let mut out = Vec::with_capacity(classic.len() + uia.len());
    let mut seen_classic_keys: HashSet<(isize, u32)> = HashSet::new();
    let mut seen_tooltip_keys: HashSet<String> = HashSet::new();
    let mut seen_ids: HashSet<String> = HashSet::new();

    for mut item in classic {
        if item.owner_hwnd != 0 && !seen_classic_keys.insert((item.owner_hwnd, item.uid)) {
            continue;
        }
        let tip_key = normalize_tooltip_key(&item.tooltip);
        if !tip_key.is_empty() {
            seen_tooltip_keys.insert(tip_key);
        }
        ensure_unique_id(&mut item.id, &mut seen_ids);
        out.push(item);
    }

    for mut item in uia {
        let tip_key = normalize_tooltip_key(&item.tooltip);
        if !tip_key.is_empty() && seen_tooltip_keys.contains(&tip_key) {
            if let Some(existing) = out
                .iter_mut()
                .find(|c| normalize_tooltip_key(&c.tooltip) == tip_key)
                && existing.uia_automation_id.is_none()
            {
                existing.uia_automation_id = item.uia_automation_id;
            }
            continue;
        }

        if let Some(aid) = item.uia_automation_id.as_deref()
            && !aid.eq_ignore_ascii_case("NotifyItemIcon")
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
    let x = cursor_xy
        .0
        .clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16 as u16 as usize;
    let y = cursor_xy
        .1
        .clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16 as u16 as usize;
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
                post_tray_callback_messages(hwnd, info.callback_msg, info.uid, click, cursor_xy);
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
    click: TrayClickKind,
    cursor_xy: (i32, i32),
) {
    let std_wparam = WPARAM(uid as usize);
    let v4_wparam = WPARAM(pack_cursor_xy(cursor_xy));
    let send_v4_pair = uid != 0 && (uid >> 16) == 0;

    let events: &[u32] = match click {
        TrayClickKind::LeftClick => &[WM_LBUTTONDOWN, WM_LBUTTONUP],
        TrayClickKind::RightClick => &[WM_RBUTTONDOWN, WM_RBUTTONUP],
        TrayClickKind::DoubleClick => &[WM_LBUTTONDBLCLK, WM_LBUTTONUP],
    };

    for &ev in events {
        unsafe {
            let _ = PostMessageW(Some(hwnd), callback_msg, std_wparam, LPARAM(ev as isize));
        }
        if send_v4_pair {
            unsafe {
                let _ = PostMessageW(
                    Some(hwnd),
                    callback_msg,
                    v4_wparam,
                    LPARAM(pack_v4_lparam(ev, uid)),
                );
            }
        }
    }

    match click {
        TrayClickKind::LeftClick => {
            if (uid >> 16) == 0 {
                unsafe {
                    let _ = PostMessageW(
                        Some(hwnd),
                        callback_msg,
                        v4_wparam,
                        LPARAM(pack_v4_lparam(NIN_SELECT, uid)),
                    );
                }
            }
        }
        TrayClickKind::RightClick => {
            if send_v4_pair {
                unsafe {
                    let _ = PostMessageW(
                        Some(hwnd),
                        callback_msg,
                        std_wparam,
                        LPARAM(WM_CONTEXTMENU as isize),
                    );
                }
            }
            if (uid >> 16) == 0 {
                unsafe {
                    let _ = PostMessageW(
                        Some(hwnd),
                        callback_msg,
                        v4_wparam,
                        LPARAM(pack_v4_lparam(WM_CONTEXTMENU, uid)),
                    );
                }
            }
        }
        TrayClickKind::DoubleClick => {}
    }
}

fn invoke_uia_tray_item(info: &TrayIconInfo) -> bool {
    let _com = ComGuard::init_sta();
    let Ok(auto) =
        (unsafe { CoCreateInstance::<_, IUIAutomation>(&CUIAutomation, None, CLSCTX_INPROC_SERVER) })
    else {
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
        for hwnd in find_top_level_windows(class_name) {
            if let Some(el) = find_matching_uia_button(&auto, &button_cond, hwnd, target_aid, &target_key)
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

/// Background watcher emitting `Vec<TrayIconInfo>` snapshots whenever tray icons change.
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

                let _com = ComGuard::init_sta();
                let uia: Option<IUIAutomation> =
                    unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) }.ok();

                let mut last_snapshot = enumerate_tray_icons_with_uia(uia.as_ref());
                if tx.send(last_snapshot.clone()).is_err() {
                    drop(uia);
                    return;
                }

                let timer_id = unsafe { SetTimer(None, 0, 1500, None) };
                if !stop_thread.load(Ordering::Acquire) {
                    let mut msg = MSG::default();
                    while !stop_thread.load(Ordering::Acquire)
                        && unsafe { GetMessageW(&mut msg, None, 0, 0) }.0 > 0
                    {
                        if msg.message == WM_TIMER || msg.message == WM_TRAY_REFRESH {
                            let next = enumerate_tray_icons_with_uia(uia.as_ref());
                            if next != last_snapshot {
                                last_snapshot = next.clone();
                                if tx.send(next).is_err() {
                                    break;
                                }
                            }
                        }
                        unsafe {
                            let _ = TranslateMessage(&msg);
                            DispatchMessageW(&msg);
                        }
                    }
                }

                if timer_id != 0 {
                    unsafe {
                        let _ = KillTimer(None, timer_id);
                    }
                }
                drop(uia);
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::offset_of;

    fn sample_icon(id: &str, tooltip: &str, owner_hwnd: isize, uid: u32) -> TrayIconInfo {
        TrayIconInfo {
            id: id.to_string(),
            tooltip: tooltip.to_string(),
            exe_name: "app.exe".to_string(),
            owner_hwnd,
            uid,
            callback_msg: 0x0401,
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
        let icons = enumerate_tray_icons();
        let mut ids = HashSet::new();
        for item in icons {
            assert!(!item.id.is_empty());
            assert!(ids.insert(item.id));
        }
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
    fn uia_button_filter_rejects_taskbar_chrome() {
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
}
