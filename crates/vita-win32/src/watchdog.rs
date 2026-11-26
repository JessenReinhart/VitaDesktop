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

/// Shell arguments that must not be replayed on restart: `--screenshot-dir` ends the process
/// early, and restart flags would nest helper processes.
fn restartable_args(args: impl Iterator<Item = String>) -> Vec<String> {
    const DROP_WITH_VALUE: [&str; 2] = ["--screenshot-dir", "--restart-wait"];
    let mut out = Vec::new();
    let mut skip_next = false;
    for arg in args {
        if skip_next {
            skip_next = false;
            continue;
        }
        let key = arg.split('=').next().unwrap_or(arg.as_str());
        if DROP_WITH_VALUE.contains(&key) {
            skip_next = !arg.contains('=');
            continue;
        }
        out.push(arg);
    }
    out
}

/// Launches a detached helper that waits for this process to exit, then relaunches the shell.
pub fn spawn_restart_helper() -> io::Result<()> {
    let exe = std::env::current_exe()?;
    let pid = std::process::id();
    let inherited = restartable_args(std::env::args().skip(1));
    Command::new(exe)
        .args(["--restart-wait", &pid.to_string()])
        .args(&inherited)
        .creation_flags(CREATE_NO_WINDOW.0)
        .spawn()?;
    Ok(())
}

/// Blocks on `parent_pid` exiting, then relaunches the shell binary detached.
pub fn run_restart_wait(parent_pid: u32) {
    let inherited = restartable_args(std::env::args().skip(3));
    let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, parent_pid) };
    if let Ok(handle) = handle {
        unsafe {
            let _ = WaitForSingleObject(handle, INFINITE);
            let _ = CloseHandle(handle);
        }
    }
    // Brief grace so the dying shell finishes releasing AppBar and taskbar state.
    std::thread::sleep(std::time::Duration::from_millis(500));
    if let Ok(exe) = std::env::current_exe() {
        let _ = Command::new(exe)
            .args(&inherited)
            .creation_flags(CREATE_NO_WINDOW.0)
            .spawn();
    }
}
