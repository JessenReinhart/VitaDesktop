use std::io;
use std::path::PathBuf;
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE, REG_SAM_FLAGS, REG_SZ, REG_VALUE_TYPE,
    RRF_RT_REG_SZ, RegCloseKey, RegDeleteValueW, RegGetValueW, RegOpenKeyExW, RegSetValueExW,
};
use windows::core::PCWSTR;

/// Value name used for the per-user logon entry.
const AUTOSTART_VALUE: &str = "VitaDesktop";
/// Relative path under `HKEY_CURRENT_USER` that Windows processes at logon.
const RUN_KEY_PATH: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Resolves the executable path used for the Run entry, preferring the
/// `vita-shell.exe` sibling of the current process.
pub fn preferred_autostart_exe() -> io::Result<PathBuf> {
    let exe = std::env::current_exe()?;
    if let Some(parent) = exe.parent() {
        let candidate = parent.join("vita-shell.exe");
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Ok(exe)
}

/// Builds the `HKCU\...\Run` command string for the given executable.
pub fn build_run_command(exe: &std::path::Path) -> String {
    format!("\"{}\"", exe.display())
}

fn open_run_key(access: REG_SAM_FLAGS) -> io::Result<HKEY> {
    let path = wide(RUN_KEY_PATH);
    let mut key = HKEY::default();
    let status = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(path.as_ptr()),
            None,
            access,
            &mut key,
        )
    };
    if status.is_err() {
        return Err(io::Error::from_raw_os_error(status.0 as i32));
    }
    Ok(key)
}

/// Registers or removes the VitaDesktop logon entry. Idempotent.
pub fn set_launch_at_login(enabled: bool) -> io::Result<()> {
    let key = open_run_key(KEY_SET_VALUE)?;
    let value_name = wide(AUTOSTART_VALUE);

    let result = if enabled {
        let exe = preferred_autostart_exe()?;
        let command = build_run_command(&exe);
        let mut bytes: Vec<u8> = command
            .encode_utf16()
            .flat_map(|unit| unit.to_le_bytes())
            .collect();
        // REG_SZ must be null-terminated.
        bytes.extend_from_slice(&[0, 0]);
        let status =
            unsafe { RegSetValueExW(key, PCWSTR(value_name.as_ptr()), None, REG_SZ, Some(&bytes)) };
        if status.is_err() {
            Err(io::Error::from_raw_os_error(status.0 as i32))
        } else {
            Ok(())
        }
    } else {
        let status = unsafe { RegDeleteValueW(key, PCWSTR(value_name.as_ptr())) };
        // ERROR_FILE_NOT_FOUND (2) means the entry is already absent.
        if status.is_err() && status.0 != 2 {
            Err(io::Error::from_raw_os_error(status.0 as i32))
        } else {
            Ok(())
        }
    };

    unsafe {
        let _ = RegCloseKey(key);
    }
    result
}

/// Reads the current logon entry. Returns `Some(command)` when present.
pub fn read_launch_entry() -> io::Result<Option<String>> {
    let key = open_run_key(KEY_READ)?;
    let value_name = wide(AUTOSTART_VALUE);

    let mut value_type = REG_VALUE_TYPE::default();
    let mut len = 0u32;
    let probe = unsafe {
        RegGetValueW(
            key,
            PCWSTR::null(),
            PCWSTR(value_name.as_ptr()),
            RRF_RT_REG_SZ,
            Some(&mut value_type),
            None,
            Some(&mut len),
        )
    };
    if probe.is_err() {
        unsafe {
            let _ = RegCloseKey(key);
        }
        return Ok(None);
    }

    let mut buf = vec![0u16; (len as usize / 2).max(1)];
    let status = unsafe {
        RegGetValueW(
            key,
            PCWSTR::null(),
            PCWSTR(value_name.as_ptr()),
            RRF_RT_REG_SZ,
            Some(&mut value_type),
            Some(buf.as_mut_ptr() as *mut std::ffi::c_void),
            Some(&mut len),
        )
    };
    unsafe {
        let _ = RegCloseKey(key);
    }
    if status.is_err() {
        return Ok(None);
    }

    let units = (len as usize / 2).min(buf.len());
    let text: String = String::from_utf16_lossy(&buf[..units])
        .trim_end_matches('\0')
        .to_string();
    if text.is_empty() {
        Ok(None)
    } else {
        Ok(Some(text))
    }
}
