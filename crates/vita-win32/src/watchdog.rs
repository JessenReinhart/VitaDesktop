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
    const DROP_WITH_VALUE: [&str; 3] = ["--screenshot-dir", "--restart-wait", "--hot-reload-wait"];
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

/// Runs `cargo check --release --workspace` so compile errors surface while the shell still runs.
/// Safe against the running exe because checking never writes the final binary.
pub fn validate_release_build(workspace: &std::path::Path) -> Result<(), String> {
    let output = Command::new(cargo_program())
        .args(["check", "--release", "--workspace"])
        .current_dir(workspace)
        .creation_flags(CREATE_NO_WINDOW.0)
        .output()
        .map_err(|err| format!("could not run cargo: {err}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let summary: Vec<&str> = stderr
        .lines()
        .filter(|line| line.starts_with("error"))
        .take(3)
        .collect();
    if summary.is_empty() {
        Err("release check failed".to_string())
    } else {
        Err(summary.join(" | "))
    }
}

fn cargo_program() -> std::ffi::OsString {
    std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into())
}

fn hot_reload_log(message: &str) {
    let path = std::env::temp_dir().join("vita-hot-reload.log");
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut file| {
            use std::io::Write;
            writeln!(file, "[{stamp}] {message}")
        });
}

/// Launches the detached hot-reload helper for this process.
pub fn spawn_hot_reload_helper() -> io::Result<()> {
    let exe = std::env::current_exe()?;
    let pid = std::process::id();
    let inherited = restartable_args(std::env::args().skip(1));
    Command::new(exe)
        .args(["--hot-reload-wait", &pid.to_string()])
        .args(&inherited)
        .creation_flags(CREATE_NO_WINDOW.0)
        .spawn()?;
    Ok(())
}

/// Waits for the shell to exit, rebuilds release, and always relaunches something runnable.
///
/// The successful `cargo check` performed before exit makes the rebuild here a link-only step in
/// the common case. If the build still fails, the previously built binary is relaunched so the
/// desktop never stays down.
pub fn run_hot_reload_wait(parent_pid: u32) {
    let exe = match std::env::current_exe() {
        Ok(path) => path,
        Err(err) => {
            hot_reload_log(&format!("cannot resolve own exe: {err}"));
            return;
        }
    };
    let workspace = exe
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf());

    let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, parent_pid) };
    if let Ok(handle) = handle {
        unsafe {
            let _ = WaitForSingleObject(handle, INFINITE);
            let _ = CloseHandle(handle);
        }
    }
    std::thread::sleep(std::time::Duration::from_millis(700));

    if let Some(workspace) = workspace {
        let output = Command::new(cargo_program())
            .args(["build", "--release", "--workspace"])
            .current_dir(&workspace)
            .creation_flags(CREATE_NO_WINDOW.0)
            .output();
        match output {
            Ok(out) if out.status.success() => hot_reload_log("rebuild succeeded"),
            Ok(out) => hot_reload_log(&format!(
                "rebuild failed, relaunching previous binary: {}",
                String::from_utf8_lossy(&out.stderr)
                    .lines()
                    .filter(|line| line.starts_with("error"))
                    .take(3)
                    .collect::<Vec<_>>()
                    .join(" | ")
            )),
            Err(err) => hot_reload_log(&format!("cargo unavailable: {err}")),
        }
    }

    let inherited = restartable_args(std::env::args().skip(3));
    let _ = Command::new(exe)
        .args(&inherited)
        .creation_flags(CREATE_NO_WINDOW.0)
        .spawn();
}
