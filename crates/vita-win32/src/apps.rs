use crate::windows::hbitmap_to_rgba;
use directories::BaseDirs;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use vita_core::{InstalledApp, RgbaIcon, classify_app_category};
use windows::Win32::Foundation::SIZE;
use windows::Win32::Graphics::Gdi::DeleteObject;
use windows::Win32::System::Com::{
    COINIT_APARTMENTTHREADED, CoInitializeEx, CoTaskMemFree, CoUninitialize,
};
use windows::Win32::UI::Shell::{
    BHID_EnumItems, FOLDERID_AppsFolder, IEnumShellItems, IShellItem, IShellItemImageFactory,
    KF_FLAG_DEFAULT, SHGetKnownFolderItem, SHCreateItemFromParsingName,
    SIGDN_DESKTOPABSOLUTEPARSING, SIGDN_NORMALDISPLAY, SIIGBF_BIGGERSIZEOK, SIIGBF_RESIZETOFIT,
    ShellExecuteW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{Interface, PCWSTR, PWSTR, w};

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

fn icon_cache_dir() -> PathBuf {
    BaseDirs::new()
        .map(|b| b.cache_dir().join("VitaDesktop").join("icons"))
        .unwrap_or_else(|| std::env::temp_dir().join("VitaDesktop-icons"))
}

fn fnv1a_64(input: &str) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in input.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn pwstr_to_string_and_free(pwstr: PWSTR) -> String {
    if pwstr.is_null() {
        return String::new();
    }
    let text = unsafe { pwstr.to_string() }.unwrap_or_default();
    unsafe {
        CoTaskMemFree(Some(pwstr.0.cast()));
    }
    text
}

fn is_clutter_entry(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.contains("uninstall")
        || lower.starts_with("readme")
        || lower.ends_with(" help")
        || lower.contains("release notes")
        || lower.contains("online documentation")
}

fn load_cached_png(path: &Path) -> Option<RgbaIcon> {
    let img = image::open(path).ok()?.to_rgba8();
    let (width, height) = img.dimensions();
    Some(RgbaIcon {
        width,
        height,
        pixels: Arc::from(img.into_raw()),
    })
}

fn extract_and_cache_item_icon(item: &IShellItem, cache_path: &Path) -> Option<RgbaIcon> {
    if cache_path.exists()
        && let Some(cached) = load_cached_png(cache_path)
    {
        return Some(cached);
    }

    let factory: IShellItemImageFactory = item.cast().ok()?;
    let hbitmap = unsafe {
        factory.GetImage(
            SIZE { cx: 64, cy: 64 },
            SIIGBF_RESIZETOFIT | SIIGBF_BIGGERSIZEOK,
        )
    }
    .ok()?;
    let rgba = hbitmap_to_rgba(hbitmap, None);
    unsafe {
        let _ = DeleteObject(hbitmap.into());
    }

    if let Some(icon) = &rgba {
        if let Some(parent) = cache_path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = image::save_buffer(
            cache_path,
            &icon.pixels,
            icon.width,
            icon.height,
            image::ColorType::Rgba8,
        );
    }
    rgba
}

/// Discovers all installed Win32 and UWP/MSIX applications from `shell:AppsFolder`
/// with a persistent 64x64 RGBA PNG cache in `%LOCALAPPDATA%\VitaDesktop\icons`.
pub fn enumerate_installed_apps() -> Vec<InstalledApp> {
    let _com = ComGuard::init_sta();
    let cache_dir = icon_cache_dir();
    let _ = fs::create_dir_all(&cache_dir);

    let apps_folder: IShellItem = match unsafe {
        SHGetKnownFolderItem(&FOLDERID_AppsFolder, KF_FLAG_DEFAULT, None)
    } {
        Ok(folder) => folder,
        Err(err) => {
            tracing::warn!(?err, "Failed to open FOLDERID_AppsFolder");
            return Vec::new();
        }
    };

    let enum_items: IEnumShellItems = match unsafe { apps_folder.BindToHandler(None, &BHID_EnumItems) }
    {
        Ok(e) => e,
        Err(err) => {
            tracing::warn!(?err, "Failed to bind BHID_EnumItems");
            return Vec::new();
        }
    };

    let mut apps: Vec<InstalledApp> = Vec::new();
    let mut name_index: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut seen_aumids = HashSet::new();
    let mut batch: [Option<IShellItem>; 16] = Default::default();

    loop {
        let mut fetched: u32 = 0;
        let hr = unsafe { enum_items.Next(&mut batch, Some(&mut fetched)) };
        if hr.is_err() || fetched == 0 {
            break;
        }

        for slot in batch.iter_mut().take(fetched as usize) {
            let Some(item) = slot.take() else { continue };
            let name = unsafe { item.GetDisplayName(SIGDN_NORMALDISPLAY) }
                .map(pwstr_to_string_and_free)
                .unwrap_or_default();
            let aumid = unsafe { item.GetDisplayName(SIGDN_DESKTOPABSOLUTEPARSING) }
                .map(pwstr_to_string_and_free)
                .unwrap_or_default();

            let name_trim = name.trim();
            let aumid_trim = aumid.trim();
            if name_trim.is_empty() || aumid_trim.is_empty() || is_clutter_entry(name_trim) {
                continue;
            }

            if !seen_aumids.insert(aumid_trim.to_ascii_lowercase()) {
                continue;
            }

            let cache_file = cache_dir.join(format!("{:016x}.png", fnv1a_64(aumid_trim)));
            let icon = extract_and_cache_item_icon(&item, &cache_file);
            let category = classify_app_category(name_trim, aumid_trim);

            let entry = InstalledApp {
                name: name_trim.to_string(),
                aumid: aumid_trim.to_string(),
                category,
                icon_cache_path: cache_file.to_str().map(str::to_string),
                icon,
            };

            let name_key = name_trim.to_ascii_lowercase();
            if let Some(&existing_idx) = name_index.get(&name_key) {
                if apps[existing_idx].icon.is_none() && entry.icon.is_some() {
                    apps[existing_idx] = entry;
                }
            } else {
                name_index.insert(name_key, apps.len());
                apps.push(entry);
            }
        }
    }

    apps.sort_by(|a, b| {
        a.name
            .to_ascii_lowercase()
            .cmp(&b.name.to_ascii_lowercase())
    });
    apps
}

/// Launches either a `shell:AppsFolder\<AUMID>` identifier or a plain target/executable.
pub fn launch_app_aumid(aumid_or_target: &str) {
    let trimmed = aumid_or_target.trim();
    if trimmed.is_empty() {
        return;
    }

    let launch_str = if trimmed.starts_with("shell:")
        || trimmed.contains(":\\")
        || trimmed.ends_with(':')
    {
        trimmed.to_string()
    } else if trimmed.contains('!')
        || trimmed.starts_with('{')
        || trimmed.eq_ignore_ascii_case("MSEdge")
    {
        format!("shell:AppsFolder\\{trimmed}")
    } else {
        trimmed.to_string()
    };

    let wide: Vec<u16> = launch_str.encode_utf16().chain(Some(0)).collect();
    unsafe {
        let _ = ShellExecuteW(
            None,
            w!("open"),
            PCWSTR(wide.as_ptr()),
            None,
            None,
            SW_SHOWNORMAL,
        );
    }
}

/// Resolves a crisp icon from an AUMID string directly (used when pinning arbitrary apps).
pub fn resolve_aumid_icon(aumid: &str) -> Option<RgbaIcon> {
    let _com = ComGuard::init_sta();
    let cache_file = icon_cache_dir().join(format!("{:016x}.png", fnv1a_64(aumid)));
    if cache_file.exists()
        && let Some(cached) = load_cached_png(&cache_file)
    {
        return Some(cached);
    }
    let parsing = format!("shell:AppsFolder\\{aumid}");
    let wide: Vec<u16> = parsing.encode_utf16().chain(Some(0)).collect();
    let item: IShellItem =
        unsafe { SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None) }.ok()?;
    extract_and_cache_item_icon(&item, &cache_file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enumerate_installed_apps_returns_real_apps() {
        let apps = enumerate_installed_apps();
        assert!(!apps.is_empty(), "Expected FOLDERID_AppsFolder to list installed apps");
        assert!(apps.iter().all(|a| !a.name.is_empty() && !a.aumid.is_empty()));
        assert!(apps.iter().any(|a| a.icon.is_some()), "Expected at least one extracted app icon");
    }
}
