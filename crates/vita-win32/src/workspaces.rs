use std::collections::HashMap;
use std::ffi::c_void;
use std::mem::size_of;
use std::thread;
use std::time::Duration;
use vita_core::{RunningWindow, SpaceInfo};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, RRF_RT_REG_BINARY, RRF_RT_REG_SZ, RegGetValueW,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP,
    SendInput, VIRTUAL_KEY, VK_D, VK_LCONTROL, VK_LEFT, VK_LWIN, VK_RIGHT,
};
use windows::Win32::UI::Shell::{IVirtualDesktopManager, VirtualDesktopManager};
use windows::core::{GUID, PCWSTR};

const VD_ROOT: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\VirtualDesktops";

fn read_registry_binary(subkey: &str, value_name: &str) -> Option<Vec<u8>> {
    let subkey_w: Vec<u16> = subkey.encode_utf16().chain(Some(0)).collect();
    let val_w: Vec<u16> = value_name.encode_utf16().chain(Some(0)).collect();
    let mut size: u32 = 0;

    let probe = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey_w.as_ptr()),
            PCWSTR(val_w.as_ptr()),
            RRF_RT_REG_BINARY,
            None,
            None,
            Some(&mut size),
        )
    };
    if probe.is_err() || size == 0 {
        return None;
    }

    let mut buf = vec![0u8; size as usize];
    let read = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey_w.as_ptr()),
            PCWSTR(val_w.as_ptr()),
            RRF_RT_REG_BINARY,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    if read.is_ok() {
        buf.truncate(size as usize);
        Some(buf)
    } else {
        None
    }
}

fn read_registry_string(subkey: &str, value_name: &str) -> Option<String> {
    let subkey_w: Vec<u16> = subkey.encode_utf16().chain(Some(0)).collect();
    let val_w: Vec<u16> = value_name.encode_utf16().chain(Some(0)).collect();
    let mut buf = [0u16; 256];
    let mut bytes = (buf.len() * 2) as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey_w.as_ptr()),
            PCWSTR(val_w.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut bytes),
        )
    };
    if status.is_ok() && bytes >= 2 {
        let len = (bytes as usize / 2).saturating_sub(1);
        let text = String::from_utf16_lossy(&buf[..len]).trim().to_string();
        (!text.is_empty()).then_some(text)
    } else {
        None
    }
}

fn bytes_to_guid(bytes: &[u8]) -> Option<GUID> {
    if bytes.len() < 16 {
        return None;
    }
    let data1 = u32::from_le_bytes(bytes[0..4].try_into().ok()?);
    let data2 = u16::from_le_bytes(bytes[4..6].try_into().ok()?);
    let data3 = u16::from_le_bytes(bytes[6..8].try_into().ok()?);
    let data4: [u8; 8] = bytes[8..16].try_into().ok()?;
    Some(GUID::from_values(data1, data2, data3, data4))
}

fn format_guid(g: GUID) -> String {
    format!(
        "{{{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}}}",
        g.data1,
        g.data2,
        g.data3,
        g.data4[0],
        g.data4[1],
        g.data4[2],
        g.data4[3],
        g.data4[4],
        g.data4[5],
        g.data4[6],
        g.data4[7],
    )
}

fn default_dux_space_meta(index: usize) -> (String, String) {
    match index {
        0 => (
            "Personal".to_string(),
            "Notes, photos & the everyday".to_string(),
        ),
        1 => ("Studio".to_string(), "Ideas in progress".to_string()),
        2 => (
            "Unwind".to_string(),
            "A little room to play".to_string(),
        ),
        n => (format!("Space {}", n + 1), "Focused workspace".to_string()),
    }
}

/// Safely enumerates Windows 11 Virtual Desktops using public Registry state and documented
/// `IVirtualDesktopManager` without relying on fragile undocumented COM vtables.
pub fn enumerate_spaces(running_windows: &[RunningWindow]) -> Vec<SpaceInfo> {
    let ids_raw = read_registry_binary(VD_ROOT, "VirtualDesktopIDs").unwrap_or_default();
    let cur_raw = read_registry_binary(VD_ROOT, "CurrentVirtualDesktop").unwrap_or_default();
    let current_guid = bytes_to_guid(&cur_raw);

    let mut guids: Vec<GUID> = ids_raw
        .as_chunks::<16>()
        .0
        .iter()
        .filter_map(|chunk| bytes_to_guid(chunk))
        .collect();

    if guids.is_empty() {
        let fallback_guid = current_guid.unwrap_or(GUID::zeroed());
        guids.push(fallback_guid);
    }

    // Count running windows per Virtual Desktop GUID via documented IVirtualDesktopManager.
    let mut counts: HashMap<String, usize> = HashMap::new();
    let mut mapped_any = false;

    let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    let should_uninit = hr.is_ok();
    if let Ok(vdm) =
        unsafe { CoCreateInstance::<_, IVirtualDesktopManager>(&VirtualDesktopManager, None, CLSCTX_ALL) }
    {
        for win in running_windows {
            let hwnd = HWND(win.hwnd as *mut c_void);
            if let Ok(desk_guid) = unsafe { vdm.GetWindowDesktopId(hwnd) }
                && desk_guid != GUID::zeroed()
            {
                *counts.entry(format_guid(desk_guid)).or_insert(0) += 1;
                mapped_any = true;
            }
        }
    }
    if should_uninit {
        unsafe { CoUninitialize() };
    }

    let mut any_current = false;
    let mut spaces = Vec::with_capacity(guids.len());

    for (index, guid) in guids.iter().enumerate() {
        let id = format_guid(*guid);
        let is_current = current_guid.is_some_and(|c| c == *guid);
        if is_current {
            any_current = true;
        }
        let custom_name = read_registry_string(&format!("{VD_ROOT}\\Desktops\\{id}"), "Name");
        let (default_name, default_tagline) = default_dux_space_meta(index);
        let name = custom_name.unwrap_or(default_name);
        let window_count = if mapped_any {
            counts.get(&id).copied().unwrap_or(0)
        } else if is_current || (current_guid.is_none() && index == 0) {
            running_windows.len()
        } else {
            0
        };

        spaces.push(SpaceInfo {
            id,
            index,
            name,
            tagline: default_tagline,
            is_current,
            window_count,
        });
    }

    if !any_current && let Some(first) = spaces.first_mut() {
        first.is_current = true;
    }

    spaces
}

fn key_input(vk: VIRTUAL_KEY, key_up: bool, extended: bool) -> INPUT {
    let mut flags = Default::default();
    if key_up {
        flags |= KEYEVENTF_KEYUP;
    }
    if extended {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn send_win_ctrl_combo(action_key: VIRTUAL_KEY, extended: bool) {
    let seq = [
        key_input(VK_LWIN, false, false),
        key_input(VK_LCONTROL, false, false),
        key_input(action_key, false, extended),
        key_input(action_key, true, extended),
        key_input(VK_LCONTROL, true, false),
        key_input(VK_LWIN, true, false),
    ];
    unsafe {
        let _ = SendInput(&seq, size_of::<INPUT>() as i32);
    }
}

/// Switches to `target_index` cleanly via OS `Win + Ctrl + Left/Right` shortcuts.
pub fn switch_to_space(current_index: usize, target_index: usize) {
    if current_index == target_index {
        return;
    }
    let steps = current_index.abs_diff(target_index);
    let key = if target_index > current_index {
        VK_RIGHT
    } else {
        VK_LEFT
    };
    for i in 0..steps {
        send_win_ctrl_combo(key, true);
        if i + 1 < steps {
            thread::sleep(Duration::from_millis(65));
        }
    }
}

/// Creates a new Windows 11 Virtual Desktop via `Win + Ctrl + D`.
pub fn create_new_space() {
    send_win_ctrl_combo(VK_D, false);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enumerate_spaces_returns_at_least_one_current_space() {
        let spaces = enumerate_spaces(&[]);
        assert!(!spaces.is_empty());
        assert_eq!(spaces.iter().filter(|s| s.is_current).count(), 1);
    }
}
