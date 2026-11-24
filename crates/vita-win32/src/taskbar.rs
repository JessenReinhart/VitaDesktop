use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::Threading::{GetCurrentProcessId, GetCurrentThreadId};
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, EnumWindows, GetClassNameW, GetMessageW, GetWindowThreadProcessId,
    IsWindow, IsWindowVisible, KillTimer, MSG, PM_NOREMOVE, PeekMessageW,
    PostThreadMessageW, SW_HIDE, SW_SHOW, SWP_ASYNCWINDOWPOS, SWP_FRAMECHANGED, SWP_HIDEWINDOW,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SWP_SHOWWINDOW, SetTimer, SetWindowPos,
    ShowWindow, ShowWindowAsync, TranslateMessage, WM_QUIT, WM_TIMER, WM_USER,
};
use windows::core::BOOL;

const EVENT_OBJECT_SHOW: u32 = 0x8002;
const WINEVENT_OUTOFCONTEXT: u32 = 0x0000;
const WINEVENT_SKIPOWNPROCESS: u32 = 0x0002;
const OBJID_WINDOW: i32 = 0;

static ENFORCE_HIDE: AtomicBool = AtomicBool::new(false);

fn is_taskbar_class(hwnd: HWND) -> bool {
    let mut pid: u32 = 0;
    unsafe {
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
    }
    if pid == 0 || pid == unsafe { GetCurrentProcessId() } {
        return false;
    }

    let mut buf = [0u16; 64];
    let len = unsafe { GetClassNameW(hwnd, &mut buf) };
    if len <= 0 {
        return false;
    }
    let class_name = String::from_utf16_lossy(&buf[..len as usize]);
    matches!(
        class_name.as_str(),
        "Shell_TrayWnd" | "Shell_SecondaryTrayWnd"
    )
}

unsafe extern "system" fn collect_taskbars(hwnd: HWND, lparam: LPARAM) -> BOOL {
    if is_taskbar_class(hwnd) {
        let set = unsafe { &mut *(lparam.0 as *mut Vec<isize>) };
        set.push(hwnd.0 as isize);
    }
    BOOL(1)
}

/// Finds all primary (`Shell_TrayWnd`) and secondary (`Shell_SecondaryTrayWnd`) taskbar HWNDs.
pub fn find_taskbar_hwnds() -> Vec<HWND> {
    let mut raw: Vec<isize> = Vec::new();
    let ptr = &mut raw as *mut Vec<isize> as isize;
    unsafe {
        let _ = EnumWindows(Some(collect_taskbars), LPARAM(ptr));
    }

    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for val in raw {
        if val != 0 && seen.insert(val) {
            out.push(HWND(val as *mut std::ffi::c_void));
        }
    }
    out
}

fn hide_hwnd_async(hwnd: HWND) {
    if unsafe { IsWindow(Some(hwnd)) }.as_bool() {
        unsafe {
            let _ = ShowWindowAsync(hwnd, SW_HIDE);
            let _ = SetWindowPos(
                hwnd,
                None,
                0,
                0,
                0,
                0,
                SWP_HIDEWINDOW | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE
                    | SWP_ASYNCWINDOWPOS,
            );
        }
    }
}

/// Hides all native Windows taskbars non-destructively across primary and secondary monitors.
pub fn hide_all_taskbars() {
    for hwnd in find_taskbar_hwnds() {
        hide_hwnd_async(hwnd);
    }
}

/// Unconditionally restores all native Windows taskbars and refreshes layout.
pub fn restore_all_taskbars() {
    for hwnd in find_taskbar_hwnds() {
        if unsafe { IsWindow(Some(hwnd)) }.as_bool() {
            unsafe {
                let _ = ShowWindow(hwnd, SW_SHOW);
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    0,
                    0,
                    0,
                    0,
                    SWP_SHOWWINDOW
                        | SWP_NOMOVE
                        | SWP_NOSIZE
                        | SWP_NOZORDER
                        | SWP_NOACTIVATE
                        | SWP_FRAMECHANGED,
                );
            }
        }
    }
}

unsafe extern "system" fn taskbar_show_proc(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    id_object: i32,
    id_child: i32,
    _thread_id: u32,
    _time: u32,
) {
    if event == EVENT_OBJECT_SHOW
        && id_object == OBJID_WINDOW
        && id_child == 0
        && ENFORCE_HIDE.load(Ordering::Relaxed)
        && is_taskbar_class(hwnd)
        && unsafe { IsWindowVisible(hwnd) }.as_bool()
    {
        hide_hwnd_async(hwnd);
    }
}

/// RAII guard that hides Windows taskbars, actively suppresses re-show events while running,
/// and unconditionally restores native taskbars on drop.
pub struct TaskbarGuard {
    hidden: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    thread_id: u32,
    thread: Option<JoinHandle<()>>,
}

impl TaskbarGuard {
    pub fn new(hide: bool) -> Self {
        ENFORCE_HIDE.store(hide, Ordering::SeqCst);
        if hide {
            hide_all_taskbars();
        }

        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = Arc::clone(&stop);
        let (tid_tx, tid_rx) = std::sync::mpsc::channel::<u32>();
        let thread = std::thread::Builder::new()
            .name("vita-taskbar-guard".into())
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

                let hook: HWINEVENTHOOK = unsafe {
                    SetWinEventHook(
                        EVENT_OBJECT_SHOW,
                        EVENT_OBJECT_SHOW,
                        None,
                        Some(taskbar_show_proc),
                        0,
                        0,
                        WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
                    )
                };
                let timer_id = unsafe { SetTimer(None, 0, 400, None) };

                if !stop_thread.load(Ordering::Acquire) {
                    let mut msg = MSG::default();
                    while !stop_thread.load(Ordering::Acquire)
                        && unsafe { GetMessageW(&mut msg, None, 0, 0) }.0 > 0
                    {
                        if msg.message == WM_TIMER && ENFORCE_HIDE.load(Ordering::Relaxed) {
                            for hwnd in find_taskbar_hwnds() {
                                if unsafe { IsWindowVisible(hwnd) }.as_bool() {
                                    hide_hwnd_async(hwnd);
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
                if !hook.is_invalid() {
                    unsafe {
                        let _ = UnhookWinEvent(hook);
                    }
                }
            })
            .ok();

        let thread_id = tid_rx.recv().unwrap_or(0);
        Self {
            hidden: Arc::new(AtomicBool::new(hide)),
            stop,
            thread_id,
            thread,
        }
    }

    pub fn set_hidden(&mut self, hide: bool) {
        let prev = self.hidden.swap(hide, Ordering::SeqCst);
        ENFORCE_HIDE.store(hide, Ordering::SeqCst);
        if hide {
            hide_all_taskbars();
        } else if prev {
            restore_all_taskbars();
        }
    }

    pub fn is_hidden(&self) -> bool {
        self.hidden.load(Ordering::Relaxed)
    }
}

impl Drop for TaskbarGuard {
    fn drop(&mut self) {
        let was_hidden = self.hidden.swap(false, Ordering::SeqCst);
        ENFORCE_HIDE.store(false, Ordering::SeqCst);
        self.stop.store(true, Ordering::Release);
        if self.thread_id != 0 {
            unsafe {
                let _ = PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
            }
        }
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
        if was_hidden {
            restore_all_taskbars();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::c_void;
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, RegisterClassExW, UnregisterClassW,
        WINDOW_EX_STYLE, WNDCLASSEXW, WS_POPUP,
    };
    use windows::core::w;

    unsafe extern "system" fn dummy_wnd_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> windows::Win32::Foundation::LRESULT {
        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }

    #[test]
    fn taskbar_lookup_never_panics() {
        let _ = find_taskbar_hwnds();
        for _ in 0..5 {
            let guard = TaskbarGuard::new(false);
            assert!(!guard.is_hidden());
        }
    }

    #[test]
    fn find_taskbar_hwnds_excludes_own_process_shell_tray_wnd() {
        let _lock = crate::tray::SHELL_TRAY_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let hinst = unsafe { GetModuleHandleW(None) }.unwrap_or_default();
        let class_name = w!("Shell_TrayWnd");
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(dummy_wnd_proc),
            hInstance: hinst.into(),
            lpszClassName: class_name,
            ..Default::default()
        };
        let atom = unsafe { RegisterClassExW(&wc) };
        let own_hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
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
                Some(std::ptr::null::<c_void>()),
            )
        }
        .expect("create test own-process Shell_TrayWnd");

        assert!(!is_taskbar_class(own_hwnd));
        let discovered = find_taskbar_hwnds();
        assert!(!discovered.contains(&own_hwnd));

        unsafe {
            let _ = DestroyWindow(own_hwnd);
            if atom != 0 {
                let _ = UnregisterClassW(class_name, Some(hinst.into()));
            }
        }
    }
}
