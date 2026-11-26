use crate::global_menu::friendly_app_name;
use crate::windows::process_exe_info;
use directories::BaseDirs;
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom};
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc::UnboundedSender;
use vita_core::NotificationLevel;
use windows::Win32::Foundation::{FILETIME, HWND, LPARAM, RECT, WPARAM};
use windows::Win32::Storage::FileSystem::FileTimeToLocalFileTime;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoUninitialize,
};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Accessibility::{
    CUIAutomation, HWINEVENTHOOK, IUIAutomation, SetWinEventHook, TreeScope_Subtree,
    UIA_TextControlTypeId, UnhookWinEvent,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetClassNameW, GetMessageW, GetWindowRect, GetWindowTextLengthW,
    GetWindowTextW, GetWindowThreadProcessId, IsWindow, IsWindowVisible, KillTimer, MSG,
    PM_NOREMOVE, PeekMessageW, PostThreadMessageW, SetTimer, TranslateMessage, WM_QUIT, WM_TIMER,
    WM_USER,
};

const FILE_SHARE_ALL: u32 = 0x7;
const MAX_SCAN_BYTES: u64 = 64 * 1024 * 1024;
const MAX_TOAST_BYTES: usize = 32 * 1024;
const DEFAULT_WATCH_LIMIT: usize = 40;
const FS_DEBOUNCE_MS: u32 = 250;
const TOAST_RETRY_DELAY_MS: u32 = 60;
const MAX_LIVE_TOASTS: usize = 16;
const LIVE_TOAST_TTL_SECS: u64 = 300;
const UNIX_TO_WIN_EPOCH_SECS: u64 = 11_644_473_600;
const MIN_VALID_FILETIME: u64 = (1_577_836_800 + UNIX_TO_WIN_EPOCH_SECS) * 10_000_000; // 2020-01-01

const EVENT_OBJECT_SHOW: u32 = 0x8002;
const WINEVENT_OUTOFCONTEXT: u32 = 0x0000;
const WINEVENT_SKIPOWNPROCESS: u32 = 0x0002;
const OBJID_WINDOW: i32 = 0;

const WM_WPN_FS_EVENT: u32 = WM_USER + 1;
const WM_WPN_TOAST_SHOW: u32 = WM_USER + 2;

thread_local! {
    static HOOK_THREAD_ID: Cell<u32> = const { Cell::new(0) };
}

/// A real Windows notification item extracted from the Windows Push Notification (WPN)
/// store (`wpndatabase.db` / `wpndatabase.db-wal`) or a live UIA toast banner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemNotification {
    pub wpn_id: String,
    pub app_name: String,
    pub title: String,
    pub body: String,
    pub timestamp: String,
    pub level: NotificationLevel,
}

#[derive(Debug, Clone)]
struct ScannedCandidate {
    seq: usize,
    blob_idx: usize,
    arrival_ft: Option<u64>,
    notif: SystemNotification,
}

#[derive(Debug, Clone)]
struct LiveToastEntry {
    notif: SystemNotification,
    arrival_ft: u64,
    captured_at: Instant,
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

struct WatcherState {
    limit: usize,
    live_toasts: Vec<LiveToastEntry>,
    last_emitted: Option<Vec<SystemNotification>>,
}

/// Watches Windows 11 notifications in real time via a debounced `notify` watcher on
/// `%LOCALAPPDATA%\Microsoft\Windows\Notifications` combined with a Win32 STA COM +
/// `SetWinEventHook(EVENT_OBJECT_SHOW)` UIA listener for `"Windows.UI.Core.CoreWindow"`
/// toast banners.
pub struct WindowsNotificationWatcher {
    stop: Arc<AtomicBool>,
    thread_id: u32,
    thread: Option<JoinHandle<()>>,
}

impl WindowsNotificationWatcher {
    /// Starts watching system notifications and immediately emits the initial snapshot on `tx`.
    pub fn start(tx: UnboundedSender<Vec<SystemNotification>>) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = Arc::clone(&stop);
        let (tid_tx, tid_rx) = std::sync::mpsc::channel::<u32>();

        let thread = std::thread::Builder::new()
            .name("vita-notification-watcher".into())
            .spawn(move || {
                // Force Win32 message queue creation before publishing tid so Drop cannot lose WM_QUIT.
                let mut init_msg = MSG::default();
                unsafe {
                    let _ = PeekMessageW(&mut init_msg, None, WM_USER, WM_USER, PM_NOREMOVE);
                }
                let tid = unsafe { GetCurrentThreadId() };
                HOOK_THREAD_ID.set(tid);
                let _ = tid_tx.send(tid);
                if stop_thread.load(Ordering::Acquire) {
                    HOOK_THREAD_ID.set(0);
                    return;
                }

                let com_guard = ComGuard::init_sta();
                let uia: Option<IUIAutomation> = if !stop_thread.load(Ordering::Acquire) {
                    unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) }.ok()
                } else {
                    None
                };

                let mut state = WatcherState {
                    limit: DEFAULT_WATCH_LIMIT,
                    live_toasts: Vec::new(),
                    last_emitted: None,
                };

                if !stop_thread.load(Ordering::Acquire) {
                    let initial = read_windows_notifications(state.limit);
                    state.last_emitted = Some(initial.clone());
                    let _ = tx.send(initial);
                }

                let fs_watcher = if !stop_thread.load(Ordering::Acquire) {
                    setup_fs_watcher(tid, Arc::clone(&stop_thread))
                } else {
                    None
                };

                let hook: HWINEVENTHOOK = if !stop_thread.load(Ordering::Acquire) {
                    unsafe {
                        SetWinEventHook(
                            EVENT_OBJECT_SHOW,
                            EVENT_OBJECT_SHOW,
                            None,
                            Some(toast_show_proc),
                            0,
                            0,
                            WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
                        )
                    }
                } else {
                    HWINEVENTHOOK::default()
                };

                let mut fs_timer: usize = 0;
                let mut retry_timer: usize = 0;
                let mut pending_retry_hwnd: Option<isize> = None;

                if !stop_thread.load(Ordering::Acquire) {
                    let mut msg = MSG::default();
                    while !stop_thread.load(Ordering::Acquire)
                        && unsafe { GetMessageW(&mut msg, None, 0, 0) }.0 > 0
                    {
                        match msg.message {
                            WM_WPN_FS_EVENT => {
                                if fs_timer == 0 {
                                    fs_timer = unsafe { SetTimer(None, 0, FS_DEBOUNCE_MS, None) };
                                }
                            }
                            WM_WPN_TOAST_SHOW => {
                                let hwnd_raw = msg.wParam.0 as isize;
                                if handle_toast_hwnd(hwnd_raw, uia.as_ref(), &mut state, &tx) {
                                    if fs_timer == 0 {
                                        fs_timer = unsafe {
                                            SetTimer(None, 0, FS_DEBOUNCE_MS + 100, None)
                                        };
                                    }
                                } else if hwnd_raw != 0 {
                                    pending_retry_hwnd = Some(hwnd_raw);
                                    if retry_timer != 0 {
                                        unsafe {
                                            let _ = KillTimer(None, retry_timer);
                                        }
                                    }
                                    retry_timer =
                                        unsafe { SetTimer(None, 0, TOAST_RETRY_DELAY_MS, None) };
                                }
                            }
                            WM_TIMER => {
                                let timer_id = msg.wParam.0;
                                if fs_timer != 0 && timer_id == fs_timer {
                                    unsafe {
                                        let _ = KillTimer(None, fs_timer);
                                    }
                                    fs_timer = 0;
                                    refresh_snapshot(&mut state, &tx);
                                } else if retry_timer != 0 && timer_id == retry_timer {
                                    unsafe {
                                        let _ = KillTimer(None, retry_timer);
                                    }
                                    retry_timer = 0;
                                    if let Some(hwnd_raw) = pending_retry_hwnd.take()
                                        && handle_toast_hwnd(
                                            hwnd_raw,
                                            uia.as_ref(),
                                            &mut state,
                                            &tx,
                                        )
                                        && fs_timer == 0
                                    {
                                        fs_timer = unsafe {
                                            SetTimer(None, 0, FS_DEBOUNCE_MS + 100, None)
                                        };
                                    }
                                }
                            }
                            _ => {}
                        }

                        unsafe {
                            let _ = TranslateMessage(&msg);
                            DispatchMessageW(&msg);
                        }
                    }
                }

                if fs_timer != 0 {
                    unsafe {
                        let _ = KillTimer(None, fs_timer);
                    }
                }
                if retry_timer != 0 {
                    unsafe {
                        let _ = KillTimer(None, retry_timer);
                    }
                }
                drop(fs_watcher);
                if !hook.is_invalid() {
                    unsafe {
                        let _ = UnhookWinEvent(hook);
                    }
                }
                drop(uia);
                drop(com_guard);
                HOOK_THREAD_ID.set(0);
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

impl Drop for WindowsNotificationWatcher {
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

/// Reads and parses recent Windows notifications from `%LOCALAPPDATA%\Microsoft\Windows\Notifications`
/// (`wpndatabase.db` and `wpndatabase.db-wal`) using shared-read access (`FILE_SHARE_ALL`),
/// returning deduplicated entries in newest-first order capped to `limit`.
pub fn read_windows_notifications(limit: usize) -> Vec<SystemNotification> {
    if limit == 0 {
        return Vec::new();
    }
    let Some(dir) = wpn_notifications_dir() else {
        return Vec::new();
    };
    let db_bytes = read_shared_bytes(&dir.join("wpndatabase.db"));
    let wal_bytes = read_shared_bytes(&dir.join("wpndatabase.db-wal"));
    scan_toasts_from_blobs(&[&db_bytes, &wal_bytes], limit)
}

fn wpn_notifications_dir() -> Option<PathBuf> {
    let local_app_data = BaseDirs::new()
        .map(|b| b.data_local_dir().to_path_buf())
        .or_else(|| std::env::var_os("LOCALAPPDATA").map(PathBuf::from))?;
    Some(
        local_app_data
            .join("Microsoft")
            .join("Windows")
            .join("Notifications"),
    )
}

fn read_shared_bytes(path: &Path) -> Vec<u8> {
    let Ok(mut file) = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_ALL)
        .open(path)
    else {
        return Vec::new();
    };
    if let Ok(meta) = file.metadata()
        && meta.len() > MAX_SCAN_BYTES
    {
        let _ = file.seek(SeekFrom::End(-(MAX_SCAN_BYTES as i64)));
    }
    let mut buf = Vec::new();
    let _ = file.take(MAX_SCAN_BYTES).read_to_end(&mut buf);
    buf
}

fn setup_fs_watcher(tid: u32, stop: Arc<AtomicBool>) -> Option<RecommendedWatcher> {
    let dir = wpn_notifications_dir()?;
    if !dir.is_dir() {
        return None;
    }

    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if stop.load(Ordering::Acquire) {
            return;
        }
        if let Ok(event) = res
            && matches!(
                event.kind,
                EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_) | EventKind::Any
            )
        {
            let is_wpn_file = event.paths.is_empty()
                || event.paths.iter().any(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.to_ascii_lowercase().starts_with("wpndatabase"))
                });
            if is_wpn_file {
                unsafe {
                    let _ = PostThreadMessageW(tid, WM_WPN_FS_EVENT, WPARAM(0), LPARAM(0));
                }
            }
        }
    })
    .ok()?;

    if let Err(err) = watcher.watch(&dir, RecursiveMode::NonRecursive) {
        tracing::debug!(?err, path = %dir.display(), "Failed to watch WPN directory");
        return None;
    }
    Some(watcher)
}

unsafe extern "system" fn toast_show_proc(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    id_object: i32,
    id_child: i32,
    _thread_id: u32,
    _time: u32,
) {
    if event != EVENT_OBJECT_SHOW
        || id_object != OBJID_WINDOW
        || id_child != 0
        || hwnd.0.is_null()
        || !is_core_window(hwnd)
    {
        return;
    }
    let tid = HOOK_THREAD_ID.get();
    if tid != 0 {
        unsafe {
            let _ = PostThreadMessageW(tid, WM_WPN_TOAST_SHOW, WPARAM(hwnd.0 as usize), LPARAM(0));
        }
    }
}

fn is_core_window(hwnd: HWND) -> bool {
    let mut buf = [0u16; 64];
    let len = unsafe { GetClassNameW(hwnd, &mut buf) };
    if len <= 0 {
        return false;
    }
    String::from_utf16_lossy(&buf[..len as usize]) == "Windows.UI.Core.CoreWindow"
}

fn handle_toast_hwnd(
    hwnd_raw: isize,
    uia: Option<&IUIAutomation>,
    state: &mut WatcherState,
    tx: &UnboundedSender<Vec<SystemNotification>>,
) -> bool {
    let Some(live) = inspect_toast_core_window(hwnd_raw, uia) else {
        return false;
    };
    state
        .live_toasts
        .retain(|e| e.notif.wpn_id != live.notif.wpn_id);
    state.live_toasts.push(live);
    if state.live_toasts.len() > MAX_LIVE_TOASTS {
        let excess = state.live_toasts.len() - MAX_LIVE_TOASTS;
        state.live_toasts.drain(..excess);
    }
    refresh_snapshot(state, tx);
    true
}

fn refresh_snapshot(state: &mut WatcherState, tx: &UnboundedSender<Vec<SystemNotification>>) {
    let now = Instant::now();
    state
        .live_toasts
        .retain(|e| now.duration_since(e.captured_at).as_secs() < LIVE_TOAST_TTL_SECS);

    let disk = read_windows_notifications(state.limit);
    let mut disk_keys: HashSet<(String, String)> = HashSet::with_capacity(disk.len());
    for item in &disk {
        disk_keys.insert((
            item.title.to_ascii_lowercase(),
            item.body.to_ascii_lowercase(),
        ));
    }

    let mut merged: Vec<SystemNotification> = Vec::with_capacity(state.limit);
    let mut live_sorted: Vec<&LiveToastEntry> = state.live_toasts.iter().collect();
    live_sorted.sort_by_key(|e| std::cmp::Reverse(e.arrival_ft));

    for live in live_sorted {
        if merged.len() >= state.limit {
            break;
        }
        let key = (
            live.notif.title.to_ascii_lowercase(),
            live.notif.body.to_ascii_lowercase(),
        );
        if !disk_keys.contains(&key) {
            merged.push(live.notif.clone());
        }
    }

    for item in disk {
        if merged.len() >= state.limit {
            break;
        }
        merged.push(item);
    }

    if state.last_emitted.as_ref() != Some(&merged) {
        state.last_emitted = Some(merged.clone());
        let _ = tx.send(merged);
    }
}

fn inspect_toast_core_window(
    hwnd_raw: isize,
    uia: Option<&IUIAutomation>,
) -> Option<LiveToastEntry> {
    if hwnd_raw == 0 {
        return None;
    }
    let hwnd = HWND(hwnd_raw as *mut c_void);
    if !unsafe { IsWindow(Some(hwnd)) }.as_bool()
        || !unsafe { IsWindowVisible(hwnd) }.as_bool()
        || !is_core_window(hwnd)
    {
        return None;
    }

    let mut rect = RECT::default();
    if unsafe { GetWindowRect(hwnd, &mut rect) }.is_err() {
        return None;
    }
    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;
    if !(140..=1100).contains(&width) || !(40..=850).contains(&height) {
        return None;
    }

    let mut pid: u32 = 0;
    unsafe {
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
    }
    if pid != 0 {
        let (_, exe_name) = process_exe_info(pid);
        if matches!(
            exe_name.to_ascii_lowercase().as_str(),
            "startmenuexperiencehost.exe"
                | "searchhost.exe"
                | "searchapp.exe"
                | "lockapp.exe"
                | "screenclippinghost.exe"
                | "textinputhost.exe"
        ) {
            return None;
        }
    }

    let win_title = read_hwnd_text(hwnd);
    let auto = uia?;
    let root = unsafe { auto.ElementFromHandle(hwnd) }.ok()?;
    let root_name = unsafe { root.CurrentName() }
        .map(|s| s.to_string())
        .unwrap_or_default();
    let root_auto_id = unsafe { root.CurrentAutomationId() }
        .map(|s| s.to_string())
        .unwrap_or_default();

    let true_cond = unsafe { auto.CreateTrueCondition() }.ok()?;
    let arr = unsafe { root.FindAll(TreeScope_Subtree, &true_cond) }.ok()?;
    let len = unsafe { arr.Length() }.ok()?.clamp(0, 80);

    let mut has_toast_marker = contains_ascii_ci(&win_title, "notification")
        || contains_ascii_ci(&win_title, "toast")
        || contains_ascii_ci(&root_name, "notification")
        || contains_ascii_ci(&root_name, "toast")
        || contains_ascii_ci(&root_auto_id, "toast")
        || contains_ascii_ci(&root_auto_id, "notification");

    let mut explicit_app: Option<String> = None;
    let mut explicit_title: Option<String> = None;
    let mut explicit_body_parts: Vec<String> = Vec::new();
    let mut ordered_texts: Vec<String> = Vec::new();
    let mut seen_texts: HashSet<String> = HashSet::new();

    for i in 0..len {
        let Ok(el) = (unsafe { arr.GetElement(i) }) else {
            continue;
        };
        let auto_id = unsafe { el.CurrentAutomationId() }
            .map(|s| s.to_string())
            .unwrap_or_default();
        let auto_lower = auto_id.to_ascii_lowercase();

        if auto_lower.contains("quicksettings")
            || auto_lower.contains("controlcenter")
            || auto_lower.contains("volumeslider")
            || auto_lower.contains("brightnessslider")
            || auto_lower.contains("calendarview")
        {
            return None;
        }

        if auto_lower.contains("toast")
            || auto_lower.contains("notification")
            || auto_lower == "appnametext"
            || auto_lower == "titletext"
            || auto_lower == "messagetext"
            || auto_lower == "dismissbutton"
        {
            has_toast_marker = true;
        }

        let Ok(ctrl_type) = (unsafe { el.CurrentControlType() }) else {
            continue;
        };
        if ctrl_type != UIA_TextControlTypeId {
            continue;
        }

        let name = unsafe { el.CurrentName() }
            .map(|s| s.to_string())
            .unwrap_or_default();
        let trimmed = name.trim();
        if trimmed.is_empty()
            || trimmed.eq_ignore_ascii_case("New notification")
            || is_toast_chrome_text(trimmed)
        {
            continue;
        }

        match auto_lower.as_str() {
            "appnametext" | "appname" if explicit_app.is_none() => {
                explicit_app = Some(trimmed.to_string());
            }
            "titletext" if explicit_title.is_none() => {
                explicit_title = Some(trimmed.to_string());
            }
            "messagetext" | "bodytext" | "bodytext1" | "bodytext2"
                if !explicit_body_parts.iter().any(|p| p == trimmed) =>
            {
                explicit_body_parts.push(trimmed.to_string());
            }
            _ => {}
        }

        if seen_texts.insert(trimmed.to_ascii_lowercase()) {
            ordered_texts.push(trimmed.to_string());
        }
    }

    if !has_toast_marker && explicit_app.is_none() && explicit_title.is_none() {
        return None;
    }

    let app_name = explicit_app.unwrap_or_else(|| "Windows".to_string());
    let (title, body) = if let Some(t) = explicit_title {
        let body = if !explicit_body_parts.is_empty() {
            explicit_body_parts.join(" · ")
        } else {
            ordered_texts
                .into_iter()
                .filter(|s| !s.eq_ignore_ascii_case(&t) && !s.eq_ignore_ascii_case(&app_name))
                .collect::<Vec<_>>()
                .join(" · ")
        };
        (t, body)
    } else {
        let mut content: Vec<String> = ordered_texts
            .into_iter()
            .filter(|s| !s.eq_ignore_ascii_case(&app_name))
            .collect();
        if content.is_empty() {
            return None;
        }
        let t = content.remove(0);
        let b = if !explicit_body_parts.is_empty() {
            explicit_body_parts.join(" · ")
        } else {
            content.join(" · ")
        };
        (t, b)
    };

    if title.trim().is_empty() {
        return None;
    }

    let arrival_ft = current_utc_filetime();
    let timestamp = format_filetime_hhmm(arrival_ft).unwrap_or_else(|| "Now".to_string());
    let level = infer_notification_level(None, &app_name, &title, &body);
    let wpn_id = format!(
        "uia:{:016x}",
        fnv1a_64(format!("{app_name}\0{title}\0{body}").as_bytes())
    );

    Some(LiveToastEntry {
        notif: SystemNotification {
            wpn_id,
            app_name,
            title,
            body,
            timestamp,
            level,
        },
        arrival_ft,
        captured_at: Instant::now(),
    })
}

fn is_toast_chrome_text(s: &str) -> bool {
    matches!(
        s.to_ascii_lowercase().as_str(),
        "dismiss"
            | "close"
            | "settings"
            | "see more"
            | "more options"
            | "go to notification settings"
    )
}

fn read_hwnd_text(hwnd: HWND) -> String {
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

fn scan_toasts_from_blobs(blobs: &[&[u8]], limit: usize) -> Vec<SystemNotification> {
    if limit == 0 {
        return Vec::new();
    }

    let mut candidates: Vec<ScannedCandidate> = Vec::new();
    let mut seq = 0usize;

    for (blob_idx, &blob) in blobs.iter().enumerate() {
        let spans = locate_toast_spans(blob);
        for (idx, &(start, end_inclusive)) in spans.iter().enumerate() {
            let prev_end = if idx > 0 { spans[idx - 1].1 + 1 } else { 0 };
            let next_start = spans.get(idx + 1).map_or(blob.len(), |&(s, _)| s);

            let before_start = prev_end.max(start.saturating_sub(384));
            let after_start = end_inclusive + 1;
            let after_end = next_start.min(after_start.saturating_add(256));

            let xml = String::from_utf8_lossy(&blob[start..=end_inclusive]);
            let before = &blob[before_start..start];
            let after = &blob[after_start..after_end];

            if let Some((notif, arrival_ft)) = parse_toast_xml(&xml, before, after) {
                candidates.push(ScannedCandidate {
                    seq,
                    blob_idx,
                    arrival_ft,
                    notif,
                });
                seq += 1;
            }
        }
    }

    let now_ft = current_utc_filetime();
    let mut deduped: Vec<ScannedCandidate> = Vec::new();
    let mut seen: HashMap<String, usize> = HashMap::new();

    for cand in candidates.into_iter().rev() {
        if let Some(&existing_idx) = seen.get(&cand.notif.wpn_id) {
            let existing = &mut deduped[existing_idx];
            if existing.notif.app_name == "Windows" && cand.notif.app_name != "Windows" {
                existing.notif.app_name = cand.notif.app_name;
            }
            if existing.arrival_ft.is_none()
                && let Some(ft) = cand.arrival_ft
            {
                existing.arrival_ft = Some(ft);
                existing.notif.timestamp = cand.notif.timestamp;
            }
        } else {
            seen.insert(cand.notif.wpn_id.clone(), deduped.len());
            deduped.push(cand);
        }
    }

    deduped.sort_by_key(|c| {
        let sort_ft = c
            .arrival_ft
            .unwrap_or(if c.blob_idx > 0 { now_ft } else { 0 });
        std::cmp::Reverse((sort_ft, c.blob_idx, c.seq))
    });

    deduped.truncate(limit);
    deduped.into_iter().map(|c| c.notif).collect()
}

fn locate_toast_spans(bytes: &[u8]) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut cursor = 0usize;

    while let Some(start) = find_toast_open(bytes, cursor) {
        let Some(close_end) = find_toast_close(bytes, start + 6) else {
            cursor = start + 6;
            continue;
        };

        if let Some(inner_start) = find_toast_open(bytes, start + 6)
            && inner_start < close_end
        {
            cursor = inner_start;
            continue;
        }

        if close_end + 1 - start <= MAX_TOAST_BYTES {
            spans.push((start, close_end));
        }
        cursor = close_end + 1;
    }

    spans
}

fn find_toast_open(bytes: &[u8], from: usize) -> Option<usize> {
    const OPEN: &[u8] = b"<toast";
    let mut i = from;
    while i + OPEN.len() < bytes.len() {
        if bytes[i] == b'<'
            && bytes[i..i + OPEN.len()].eq_ignore_ascii_case(OPEN)
            && matches!(bytes[i + OPEN.len()], b'>' | b' ' | b'\t' | b'\r' | b'\n')
        {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn find_toast_close(bytes: &[u8], from: usize) -> Option<usize> {
    const CLOSE: &[u8] = b"</toast";
    let mut i = from;
    let limit = bytes.len().min(from.saturating_add(MAX_TOAST_BYTES));
    while i + CLOSE.len() < limit {
        if bytes[i] == b'<' && bytes[i..i + CLOSE.len()].eq_ignore_ascii_case(CLOSE) {
            let mut j = i + CLOSE.len();
            while j < limit && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if j < limit && bytes[j] == b'>' {
                return Some(j);
            }
        }
        i += 1;
    }
    None
}

fn parse_toast_xml(
    xml: &str,
    before_bytes: &[u8],
    after_bytes: &[u8],
) -> Option<(SystemNotification, Option<u64>)> {
    let trimmed_xml = xml.trim();
    if trimmed_xml.is_empty() {
        return None;
    }

    let toast_tag_end = find_tag_end(trimmed_xml, 0)?;
    let toast_tag = &trimmed_xml[..=toast_tag_end];
    let scenario = extract_xml_attr(toast_tag, "scenario");
    let display_ts_raw = extract_xml_attr(toast_tag, "displayTimestamp");
    let mut hint_attribution = extract_xml_attr(toast_tag, "hint-attribution");

    if hint_attribution.is_none() {
        hint_attribution = find_binding_hint_attribution(trimmed_xml);
    }

    let (texts, placement_attribution) = extract_toast_texts(trimmed_xml);
    if texts.is_empty() {
        return None;
    }

    let title = texts[0].clone();
    let body = if texts.len() > 1 {
        texts[1..].join(" · ")
    } else {
        String::new()
    };

    let app_name = hint_attribution
        .or_else(|| extract_app_from_bytes(before_bytes))
        .or_else(|| extract_app_from_bytes(after_bytes))
        .or(placement_attribution)
        .unwrap_or_else(|| "Windows".to_string());

    let (timestamp, arrival_ft) = if let Some(ref ts) = display_ts_raw {
        let parsed_ft = parse_iso8601_utc_filetime(ts);
        let formatted = parsed_ft
            .and_then(format_filetime_hhmm)
            .or_else(|| extract_hhmm_from_iso(ts))
            .unwrap_or_else(|| "Recent".to_string());
        (formatted, parsed_ft)
    } else {
        let ft = extract_nearby_arrival_filetime(after_bytes);
        let formatted = ft
            .and_then(format_filetime_hhmm)
            .unwrap_or_else(|| "Recent".to_string());
        (formatted, ft)
    };

    let level = infer_notification_level(scenario.as_deref(), &app_name, &title, &body);
    let wpn_id = format!("wpn:{:016x}", fnv1a_64(trimmed_xml.as_bytes()));

    Some((
        SystemNotification {
            wpn_id,
            app_name,
            title,
            body,
            timestamp,
            level,
        },
        arrival_ft,
    ))
}

fn find_binding_hint_attribution(xml: &str) -> Option<String> {
    let bytes = xml.as_bytes();
    const NEEDLE: &[u8] = b"<binding";
    let mut i = 0;
    while i + NEEDLE.len() < bytes.len() {
        if bytes[i] == b'<'
            && bytes[i..i + NEEDLE.len()].eq_ignore_ascii_case(NEEDLE)
            && (bytes[i + NEEDLE.len()].is_ascii_whitespace() || bytes[i + NEEDLE.len()] == b'>')
        {
            let tag_end = find_tag_end(xml, i)?;
            if let Some(attr) = extract_xml_attr(&xml[i..=tag_end], "hint-attribution") {
                return Some(attr);
            }
            i = tag_end + 1;
            continue;
        }
        i += 1;
    }
    None
}

fn extract_toast_texts(xml: &str) -> (Vec<String>, Option<String>) {
    let bytes = xml.as_bytes();
    const OPEN: &[u8] = b"<text";
    const CLOSE: &[u8] = b"</text";

    let mut texts = Vec::new();
    let mut placement_attr: Option<String> = None;
    let mut i = 0usize;

    while i + OPEN.len() < bytes.len() {
        if bytes[i] == b'<'
            && bytes[i..i + OPEN.len()].eq_ignore_ascii_case(OPEN)
            && matches!(
                bytes[i + OPEN.len()],
                b'>' | b'/' | b' ' | b'\t' | b'\r' | b'\n'
            )
        {
            let Some(tag_end) = find_tag_end(xml, i) else {
                break;
            };
            let open_tag = &xml[i..=tag_end];
            let is_self_closing = xml[..tag_end].trim_end().ends_with('/');
            if is_self_closing {
                i = tag_end + 1;
                continue;
            }

            let is_attribution = extract_xml_attr(open_tag, "placement")
                .is_some_and(|p| p.eq_ignore_ascii_case("attribution"));

            let mut j = tag_end + 1;
            let mut close_start = None;
            let mut close_end = None;
            while j + CLOSE.len() < bytes.len() {
                if bytes[j] == b'<' && bytes[j..j + CLOSE.len()].eq_ignore_ascii_case(CLOSE) {
                    let mut k = j + CLOSE.len();
                    while k < bytes.len() && bytes[k].is_ascii_whitespace() {
                        k += 1;
                    }
                    if k < bytes.len() && bytes[k] == b'>' {
                        close_start = Some(j);
                        close_end = Some(k);
                        break;
                    }
                }
                j += 1;
            }

            let (Some(cs), Some(ce)) = (close_start, close_end) else {
                i = tag_end + 1;
                continue;
            };

            let raw_inner = &xml[tag_end + 1..cs];
            let decoded = decode_text_inner(raw_inner);
            let trimmed = decoded.trim();
            if !trimmed.is_empty() {
                if is_attribution && placement_attr.is_none() {
                    let clean_attr = strip_via_prefix(trimmed);
                    if !clean_attr.is_empty() {
                        placement_attr = Some(clean_attr.to_string());
                    }
                }
                texts.push(trimmed.to_string());
            }

            i = ce + 1;
            continue;
        }
        i += 1;
    }

    (texts, placement_attr)
}

fn decode_text_inner(raw: &str) -> String {
    let trimmed = raw.trim();
    if let Some(inner) = trimmed
        .strip_prefix("<![CDATA[")
        .and_then(|s| s.strip_suffix("]]>"))
    {
        return inner.to_string();
    }
    decode_xml_entities(raw)
}

fn strip_via_prefix(text: &str) -> &str {
    if text
        .get(..4)
        .is_some_and(|p| p.eq_ignore_ascii_case("via "))
    {
        text[4..].trim()
    } else {
        text.trim()
    }
}

fn decode_xml_entities(input: &str) -> String {
    if !input.contains('&') {
        return input.to_string();
    }
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(amp_idx) = rest.find('&') {
        out.push_str(&rest[..amp_idx]);
        let after_amp = &rest[amp_idx + 1..];
        if let Some(semi_rel) = after_amp.find(';')
            && semi_rel <= 12
        {
            let entity = &after_amp[..semi_rel];
            if let Some(decoded) = decode_single_entity(entity) {
                out.push(decoded);
                rest = &after_amp[semi_rel + 1..];
                continue;
            }
        }
        out.push('&');
        rest = after_amp;
    }
    out.push_str(rest);
    out
}

fn decode_single_entity(entity: &str) -> Option<char> {
    match entity {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        _ => {
            let num = entity.strip_prefix('#')?;
            let code = if let Some(hex) = num.strip_prefix('x').or_else(|| num.strip_prefix('X')) {
                u32::from_str_radix(hex, 16).ok()?
            } else {
                num.parse::<u32>().ok()?
            };
            (code != 0).then(|| char::from_u32(code)).flatten()
        }
    }
}

fn find_tag_end(xml: &str, open_pos: usize) -> Option<usize> {
    let bytes = xml.as_bytes();
    let mut i = open_pos;
    let mut quote: Option<u8> = None;
    while i < bytes.len() {
        let b = bytes[i];
        match quote {
            Some(q) => {
                if b == q {
                    quote = None;
                }
            }
            None => match b {
                b'"' | b'\'' => quote = Some(b),
                b'>' => return Some(i),
                _ => {}
            },
        }
        i += 1;
    }
    None
}

fn extract_xml_attr(tag: &str, attr_name: &str) -> Option<String> {
    let bytes = tag.as_bytes();
    let name_bytes = attr_name.as_bytes();
    let mut i = 0usize;

    while i + name_bytes.len() < bytes.len() {
        if bytes[i] == b'"' || bytes[i] == b'\'' {
            let q = bytes[i];
            i += 1;
            while i < bytes.len() && bytes[i] != q {
                i += 1;
            }
            i += 1;
            continue;
        }
        if i > 0
            && bytes[i - 1].is_ascii_whitespace()
            && bytes[i..i + name_bytes.len()].eq_ignore_ascii_case(name_bytes)
        {
            let mut j = i + name_bytes.len();
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'=' {
                j += 1;
                while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                    j += 1;
                }
                if j < bytes.len() && (bytes[j] == b'"' || bytes[j] == b'\'') {
                    let q = bytes[j];
                    let val_start = j + 1;
                    let mut val_end = val_start;
                    while val_end < bytes.len() && bytes[val_end] != q {
                        val_end += 1;
                    }
                    if val_end < bytes.len() {
                        let raw = &tag[val_start..val_end];
                        let decoded = decode_xml_entities(raw).trim().to_string();
                        return (!decoded.is_empty()).then_some(decoded);
                    }
                }
            }
        }
        i += 1;
    }
    None
}

fn extract_app_from_bytes(bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() {
        return None;
    }

    let mut tokens: Vec<String> = Vec::new();
    let mut cur = Vec::new();
    for &b in bytes {
        if b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b'!' | b'\\' | b'/' | b':')
        {
            cur.push(b);
        } else if !cur.is_empty()
            && let Ok(s) = String::from_utf8(std::mem::take(&mut cur))
        {
            tokens.push(s);
        }
    }
    if !cur.is_empty()
        && let Ok(s) = String::from_utf8(cur)
    {
        tokens.push(s);
    }

    for token in &tokens {
        if let Some(name) = parse_aumid_token(token) {
            return Some(name);
        }
    }
    for token in &tokens {
        if let Some(name) = parse_exe_token(token) {
            return Some(name);
        }
    }
    for token in &tokens {
        if let Some(name) = parse_known_system_appid(token) {
            return Some(name);
        }
    }
    None
}

fn parse_aumid_token(token: &str) -> Option<String> {
    let (pkg, app_part) = token.split_once('!')?;
    if pkg.len() < 4 || app_part.is_empty() || (!pkg.contains('_') && !pkg.contains('.')) {
        return None;
    }

    let pkg_base = pkg.split('_').next().unwrap_or(pkg);
    if let Some(known) =
        known_app_from_identifier(pkg_base).or_else(|| known_app_from_identifier(app_part))
    {
        return Some(known.to_string());
    }

    let app_clean = app_part.trim();
    let generic_app_id = matches!(
        app_clean.to_ascii_lowercase().as_str(),
        "app" | "application" | "main" | "desktop" | "release"
    ) || app_clean.contains('.');

    if !generic_app_id && app_clean.len() >= 2 {
        return Some(capitalize_first(app_clean));
    }

    let stripped = pkg_base
        .strip_prefix("Microsoft.")
        .or_else(|| pkg_base.strip_prefix("Windows."))
        .unwrap_or(pkg_base);
    let last_seg = stripped.rsplit('.').next().unwrap_or(stripped).trim();
    (!last_seg.is_empty()).then(|| capitalize_first(last_seg))
}

fn parse_exe_token(token: &str) -> Option<String> {
    let lower = token.to_ascii_lowercase();
    if !lower.ends_with(".exe") || token.len() <= 4 {
        return None;
    }
    let file_name = token.rsplit(['\\', '/', ':']).next().unwrap_or(token);
    if file_name.len() <= 4 {
        return None;
    }
    Some(friendly_app_name(file_name, ""))
}

fn parse_known_system_appid(token: &str) -> Option<String> {
    if let Some(rest) = token.strip_prefix("Windows.SystemToast.") {
        return Some(
            known_app_from_identifier(rest)
                .map(str::to_string)
                .unwrap_or_else(|| capitalize_first(rest)),
        );
    }
    if let Some(rest) = token.strip_prefix("com.squirrel.") {
        let app = rest.split('.').next().unwrap_or(rest).trim();
        if !app.is_empty() {
            return Some(capitalize_first(app));
        }
    }
    known_app_from_identifier(token).map(str::to_string)
}

fn known_app_from_identifier(id: &str) -> Option<&'static str> {
    let lower = id.to_ascii_lowercase();
    if lower.contains("windowsterminal") {
        Some("Terminal")
    } else if lower.contains("windowsnotepad") {
        Some("Notepad")
    } else if lower.contains("outlook") {
        Some("Outlook")
    } else if lower.contains("msteams") || lower.contains("microsoft.teams") {
        Some("Teams")
    } else if lower.contains("sechealthui") || lower.contains("securityandmaintenance") {
        Some("Windows Security")
    } else if lower.contains("windowsupdate") {
        Some("Windows Update")
    } else if lower.contains("windowsstore") {
        Some("Microsoft Store")
    } else if lower.contains("microsoftedge") || lower == "msedge" {
        Some("Edge")
    } else if lower.contains("skydrive") || lower.contains("onedrive") {
        Some("OneDrive")
    } else if lower.contains("explorer") {
        Some("Files")
    } else if lower.contains("spotify") {
        Some("Spotify")
    } else if lower.contains("discord") {
        Some("Discord")
    } else if lower.contains("slack") {
        Some("Slack")
    } else {
        None
    }
}

fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str()),
        None => "Windows".to_string(),
    }
}

fn extract_nearby_arrival_filetime(after_bytes: &[u8]) -> Option<u64> {
    if after_bytes.len() < 8 {
        return None;
    }
    let max_ft = current_utc_filetime().saturating_add(300 * 10_000_000);
    let mut best: Option<u64> = None;

    for win in after_bytes.windows(8) {
        let Ok(arr) = <[u8; 8]>::try_from(win) else {
            continue;
        };
        let val = u64::from_be_bytes(arr);
        if val >= MIN_VALID_FILETIME && val <= max_ft {
            best = Some(match best {
                Some(prev) => prev.min(val),
                None => val,
            });
        }
    }
    best
}

fn current_utc_filetime() -> u64 {
    let unix_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    unix_secs
        .saturating_add(UNIX_TO_WIN_EPOCH_SECS)
        .saturating_mul(10_000_000)
}

fn format_filetime_hhmm(utc_ft: u64) -> Option<String> {
    if utc_ft == 0 {
        return None;
    }
    let utc = FILETIME {
        dwLowDateTime: (utc_ft & 0xFFFF_FFFF) as u32,
        dwHighDateTime: (utc_ft >> 32) as u32,
    };
    let mut local = FILETIME::default();
    if unsafe { FileTimeToLocalFileTime(&raw const utc, &raw mut local) }.is_err() {
        return None;
    }
    let local_ft = (u64::from(local.dwHighDateTime) << 32) | u64::from(local.dwLowDateTime);
    let total_secs = local_ft / 10_000_000;
    let day_secs = total_secs % 86_400;
    let hour = day_secs / 3_600;
    let minute = (day_secs % 3_600) / 60;
    Some(format!("{hour:02}:{minute:02}"))
}

fn parse_iso8601_utc_filetime(ts: &str) -> Option<u64> {
    let trimmed = ts.trim().strip_suffix('Z')?;
    let (date_part, time_part) = trimmed.split_once('T')?;
    let mut d_parts = date_part.split('-');
    let year: i64 = d_parts.next()?.parse().ok()?;
    let month: u32 = d_parts.next()?.parse().ok()?;
    let day: u32 = d_parts.next()?.parse().ok()?;
    if !(1970..=2100).contains(&year) || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }

    let time_no_frac = time_part.split('.').next().unwrap_or(time_part);
    let mut t_parts = time_no_frac.split(':');
    let hour: u64 = t_parts.next()?.parse().ok()?;
    let minute: u64 = t_parts.next()?.parse().ok()?;
    let second: u64 = t_parts.next().unwrap_or("0").parse().ok()?;
    if hour > 23 || minute > 59 || second > 59 {
        return None;
    }

    let days = days_from_civil(year, month, day)?;
    if days < 0 {
        return None;
    }
    let unix_secs = (days as u64) * 86_400 + hour * 3_600 + minute * 60 + second;
    Some(
        unix_secs
            .saturating_add(UNIX_TO_WIN_EPOCH_SECS)
            .saturating_mul(10_000_000),
    )
}

fn days_from_civil(year: i64, month: u32, day: u32) -> Option<i64> {
    let y = year - i64::from(month <= 2);
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m = i64::from(month);
    let d = i64::from(day);
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

fn extract_hhmm_from_iso(ts: &str) -> Option<String> {
    let time_part = ts.split('T').nth(1).unwrap_or(ts);
    let mut parts = time_part.split(':');
    let h: u32 = parts.next()?.trim().parse().ok()?;
    let m_raw = parts.next()?.get(..2)?;
    let m: u32 = m_raw.parse().ok()?;
    (h < 24 && m < 60).then(|| format!("{h:02}:{m:02}"))
}

fn infer_notification_level(
    scenario: Option<&str>,
    app_name: &str,
    title: &str,
    body: &str,
) -> NotificationLevel {
    if let Some(sc) = scenario
        && matches!(
            sc.to_ascii_lowercase().as_str(),
            "alarm" | "urgent" | "incomingcall"
        )
    {
        return NotificationLevel::Caution;
    }

    let combined = format!("{app_name} {title} {body}").to_ascii_lowercase();
    if [
        "error",
        "failed",
        "failure",
        "warning",
        "alert",
        "critical",
        "blocked",
        "threat",
        "expired",
        "restart required",
    ]
    .iter()
    .any(|kw| contains_word_like(&combined, kw))
    {
        return NotificationLevel::Caution;
    }

    if [
        "succeeded",
        "success",
        "completed",
        "finished",
        "installed",
        "updated",
        "connected",
        "passed",
        "verified",
    ]
    .iter()
    .any(|kw| contains_word_like(&combined, kw))
    {
        return NotificationLevel::Success;
    }

    NotificationLevel::Info
}

fn contains_word_like(haystack: &str, needle: &str) -> bool {
    let mut from = 0usize;
    while let Some(rel) = haystack[from..].find(needle) {
        let start = from + rel;
        let end = start + needle.len();
        let left_ok = start == 0 || !haystack.as_bytes()[start - 1].is_ascii_alphanumeric();
        let right_ok = end == haystack.len() || !haystack.as_bytes()[end].is_ascii_alphanumeric();
        if left_ok && right_ok {
            return true;
        }
        from = end;
    }
    false
}

fn contains_ascii_ci(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    haystack
        .as_bytes()
        .windows(needle.len())
        .any(|w| w.eq_ignore_ascii_case(needle.as_bytes()))
}

fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc::unbounded_channel;

    #[test]
    fn parse_toast_xml_extracts_title_body_and_decodes_entities() {
        let xml = r#"
            <toast hint-attribution="Slack &amp; Co" scenario="reminder" launch="id=1&amp;view=2">
                <visual>
                    <binding template="ToastGeneric">
                        <text hint-maxLines="1">Build &lt;VitaDesktop&gt; Passed &#x2014; v0.1</text>
                        <text>All &quot;Win32&quot; &amp; UIA checks &#39;green&#39;</text>
                        <text>   </text>
                        <text>0 warnings&#33;</text>
                    </binding>
                </visual>
            </toast>
        "#;

        let (notif, _) = parse_toast_xml(xml, b"", b"").expect("toast XML should parse");
        assert_eq!(notif.app_name, "Slack & Co");
        assert_eq!(notif.title, "Build <VitaDesktop> Passed — v0.1");
        assert_eq!(
            notif.body,
            "All \"Win32\" & UIA checks 'green' · 0 warnings!"
        );
        assert_eq!(notif.level, NotificationLevel::Success);
        assert!(notif.wpn_id.starts_with("wpn:"));
    }

    #[test]
    fn scan_binary_blob_with_embedded_toasts_and_deduplicates() {
        let toast_one = br#"<toast><visual><binding template="ToastGeneric"><text>Terminal Update</text><text>Session ready</text></binding></visual></toast>"#;
        let toast_two = br#"<toast scenario="urgent"><visual><binding template="ToastGeneric"><text>Security Alert</text><text>Restart required</text></binding></visual></toast>"#;

        let mut db_blob = Vec::new();
        db_blob
            .extend_from_slice(b"\x00\x01\x02\xffMicrosoft.WindowsTerminal_8wekyb3d8bbwe!App\x00");
        db_blob.extend_from_slice(toast_one);
        db_blob.extend_from_slice(b"\x00\x00\xfe\xfd<toast truncated=\"true\">broken");
        db_blob.extend_from_slice(b"\x00C:\\Program Files\\Vendor\\Code.exe\x00");
        db_blob.extend_from_slice(toast_two);
        db_blob.extend_from_slice(b"\x00\x12\x34");

        // Simulate WAL containing a duplicate of toast_one (newer position).
        let mut wal_blob = Vec::new();
        wal_blob.extend_from_slice(b"\xaa\xbbMicrosoft.WindowsTerminal_8wekyb3d8bbwe!App\x00");
        wal_blob.extend_from_slice(toast_one);
        wal_blob.extend_from_slice(b"\xcc\xdd");

        let items = scan_toasts_from_blobs(&[&db_blob, &wal_blob], 10);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].title, "Terminal Update");
        assert_eq!(items[0].app_name, "Terminal");
        assert_eq!(items[1].title, "Security Alert");
        assert_eq!(items[1].app_name, "VS Code");
        assert_eq!(items[1].level, NotificationLevel::Caution);

        // Single-blob deduplication also keeps newest-first unique items and honors limit.
        let mut single_blob = Vec::new();
        single_blob.extend_from_slice(toast_one);
        single_blob.extend_from_slice(b"\x00\x00");
        single_blob.extend_from_slice(toast_two);
        single_blob.extend_from_slice(b"\x00\x00");
        single_blob.extend_from_slice(toast_one);

        let deduped = scan_toasts_from_blobs(&[&single_blob], 1);
        assert_eq!(deduped.len(), 1);
        assert_eq!(deduped[0].title, "Terminal Update");
        assert_eq!(deduped[0].app_name, "Windows");
    }

    #[test]
    fn read_windows_notifications_never_panics() {
        assert!(read_windows_notifications(0).is_empty());
        let list = read_windows_notifications(25);
        assert!(list.len() <= 25);
        for item in &list {
            assert!(!item.wpn_id.is_empty());
            assert!(!item.app_name.is_empty());
            assert!(!item.title.is_empty());
        }
    }

    #[test]
    fn watcher_start_and_drop_never_hangs() {
        for _ in 0..5 {
            let (tx, mut rx) = unbounded_channel();
            let watcher = WindowsNotificationWatcher::start(tx);
            let _ = rx.try_recv();
            drop(watcher);
        }
    }
}
