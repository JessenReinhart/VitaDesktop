use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use tokio::sync::mpsc::UnboundedSender;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, RegisterHotKey, UnregisterHotKey, VK_OEM_3,
    VK_SPACE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetMessageW, MSG, PM_NOREMOVE, PeekMessageW, PostThreadMessageW, WM_HOTKEY, WM_QUIT, WM_USER,
};

const HOTKEY_EMERGENCY_RESTORE: i32 = 1;
const HOTKEY_TOGGLE_LAUNCHER: i32 = 2;
const HOTKEY_TOGGLE_SPACES: i32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyEvent {
    EmergencyRestoreTaskbar,
    ToggleLauncher,
    ToggleSpaces,
}

pub struct HotkeyGuard {
    stop: Arc<AtomicBool>,
    thread_id: u32,
    thread: Option<JoinHandle<()>>,
}

impl HotkeyGuard {
    pub fn start(tx: UnboundedSender<HotkeyEvent>) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = Arc::clone(&stop);
        let (tid_tx, tid_rx) = std::sync::mpsc::channel::<u32>();

        let thread = std::thread::Builder::new()
            .name("vita-hotkeys".into())
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

                let _ = unsafe {
                    RegisterHotKey(
                        None,
                        HOTKEY_EMERGENCY_RESTORE,
                        MOD_CONTROL | MOD_ALT | MOD_SHIFT | MOD_NOREPEAT,
                        0x52, // 'R'
                    )
                };
                let _ = unsafe {
                    RegisterHotKey(
                        None,
                        HOTKEY_TOGGLE_LAUNCHER,
                        MOD_ALT | MOD_NOREPEAT,
                        u32::from(VK_SPACE.0),
                    )
                };
                let _ = unsafe {
                    RegisterHotKey(
                        None,
                        HOTKEY_TOGGLE_SPACES,
                        MOD_ALT | MOD_NOREPEAT,
                        u32::from(VK_OEM_3.0),
                    )
                };

                if !stop_thread.load(Ordering::Acquire) {
                    let mut msg = MSG::default();
                    while !stop_thread.load(Ordering::Acquire)
                        && unsafe { GetMessageW(&mut msg, None, 0, 0) }.0 > 0
                    {
                        if msg.message == WM_HOTKEY {
                            let ev = match msg.wParam.0 as i32 {
                                HOTKEY_EMERGENCY_RESTORE => Some(HotkeyEvent::EmergencyRestoreTaskbar),
                                HOTKEY_TOGGLE_LAUNCHER => Some(HotkeyEvent::ToggleLauncher),
                                HOTKEY_TOGGLE_SPACES => Some(HotkeyEvent::ToggleSpaces),
                                _ => None,
                            };
                            if let Some(ev) = ev {
                                let _ = tx.send(ev);
                            }
                        }
                    }
                }

                unsafe {
                    let _ = UnregisterHotKey(None, HOTKEY_EMERGENCY_RESTORE);
                    let _ = UnregisterHotKey(None, HOTKEY_TOGGLE_LAUNCHER);
                    let _ = UnregisterHotKey(None, HOTKEY_TOGGLE_SPACES);
                }
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

impl Drop for HotkeyGuard {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if self.thread_id != 0 {
            unsafe {
                let _ = PostThreadMessageW(self.thread_id, WM_QUIT, Default::default(), Default::default());
            }
        }
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}
