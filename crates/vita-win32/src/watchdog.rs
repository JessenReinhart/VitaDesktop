use crate::appbar::restore_saved_workareas;
use crate::taskbar::restore_all_taskbars;
use std::io;
use std::os::windows::process::CommandExt;
use std::process::Command;
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Threading::{
    CREATE_NO_WINDOW, INFINITE, OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject,
};

/// Spawns an out-of-process watchdog that restores the Windows Taskbar if this shell crashes or is force-killed.
pub fn spawn_watchdog() -> io::Result<()> {
    let exe = std::env::current_exe()?;
    let pid = std::process::id();
    Command::new(exe)
        .args(["--watchdog", &pid.to_string()])
        .creation_flags(CREATE_NO_WINDOW.0)
        .spawn()?;
    Ok(())
}

/// Blocks on `parent_pid` exiting and immediately restores all native Windows taskbars.
pub fn run_watchdog(parent_pid: u32) {
    let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, parent_pid) };
    if let Ok(handle) = handle {
        unsafe {
            let _ = WaitForSingleObject(handle, INFINITE);
            let _ = CloseHandle(handle);
        }
    }
    restore_saved_workareas();
    restore_all_taskbars();
}
