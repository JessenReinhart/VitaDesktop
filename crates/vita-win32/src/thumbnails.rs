use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::mem::size_of;
use std::sync::Arc;
use vita_core::{PhysicalRect, RgbaIcon};
use windows::Win32::Foundation::{FALSE, HWND, RECT, TRUE};
use windows::Win32::Graphics::Dwm::{
    DWM_THUMBNAIL_PROPERTIES, DWM_TNP_OPACITY, DWM_TNP_RECTDESTINATION,
    DWM_TNP_SOURCECLIENTAREAONLY, DWM_TNP_VISIBLE, DwmQueryThumbnailSourceSize,
    DwmRegisterThumbnail, DwmUnregisterThumbnail, DwmUpdateThumbnailProperties,
};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS,
    DeleteDC, DeleteObject, GetDC, GetDIBits, HBITMAP, HDC, HGDIOBJ, ReleaseDC, SelectObject,
};
use windows::Win32::Storage::Xps::{PRINT_WINDOW_FLAGS, PrintWindow};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowRect, IsIconic, IsWindow, IsWindowVisible, PW_RENDERFULLCONTENT,
};

const MAX_CAPTURE_DIM: i32 = 16_384;

/// Fits `(src_w, src_h)` inside `slot` while preserving aspect ratio and centering along both axes.
/// Zero or non-positive inputs are guarded cleanly.
pub fn fit_aspect_rect(src_w: i32, src_h: i32, slot: PhysicalRect) -> PhysicalRect {
    if slot.width <= 0 || slot.height <= 0 {
        return PhysicalRect {
            x: slot.x,
            y: slot.y,
            width: slot.width.max(0),
            height: slot.height.max(0),
        };
    }
    if src_w <= 0 || src_h <= 0 {
        return slot;
    }

    let scale =
        (f64::from(slot.width) / f64::from(src_w)).min(f64::from(slot.height) / f64::from(src_h));
    let fitted_w = ((f64::from(src_w) * scale).round() as i32).clamp(1, slot.width);
    let fitted_h = ((f64::from(src_h) * scale).round() as i32).clamp(1, slot.height);
    let x = slot.x + (slot.width - fitted_w) / 2;
    let y = slot.y + (slot.height - fitted_h) / 2;

    PhysicalRect {
        x,
        y,
        width: fitted_w,
        height: fitted_h,
    }
}

#[derive(Debug, Clone, Copy)]
struct ThumbnailSlotEntry {
    handle: isize,
    last_dest: PhysicalRect,
    last_opacity: u8,
}

/// Manages live Win32 DWM thumbnail registrations (`HTHUMBNAIL` = `isize` in `windows 0.62`)
/// bound to a destination shell popover HWND.
pub struct DwmThumbnailHost {
    dest_hwnd: u64,
    thumbnails: HashMap<isize, ThumbnailSlotEntry>,
    opacity: u8,
}

impl Default for DwmThumbnailHost {
    fn default() -> Self {
        Self::new()
    }
}

impl DwmThumbnailHost {
    pub fn new() -> Self {
        Self {
            dest_hwnd: 0,
            thumbnails: HashMap::new(),
            opacity: 255,
        }
    }

    /// Reconciles active DWM thumbnails for `dest_hwnd` against `slots`, registering new source
    /// windows, fitting destination rectangles to source aspect ratios, and unregistering stale
    /// or mismatched-destination handles.
    pub fn sync_slots(&mut self, dest_hwnd: u64, slots: &[(isize, PhysicalRect)]) {
        if dest_hwnd == 0 || slots.is_empty() {
            self.clear();
            return;
        }

        let dest = HWND(dest_hwnd as *mut c_void);
        if !unsafe { IsWindow(Some(dest)) }.as_bool() {
            self.clear();
            return;
        }

        if self.dest_hwnd != dest_hwnd {
            self.clear();
            self.dest_hwnd = dest_hwnd;
        }

        let dest_as_isize = dest_hwnd as isize;
        let desired: HashSet<isize> = slots
            .iter()
            .filter(|&&(src_hwnd, rect)| {
                src_hwnd != 0
                    && src_hwnd != dest_as_isize
                    && rect.width > 0
                    && rect.height > 0
                    && unsafe { IsWindow(Some(HWND(src_hwnd as *mut c_void))) }.as_bool()
            })
            .map(|&(src_hwnd, _)| src_hwnd)
            .collect();

        let stale: Vec<isize> = self
            .thumbnails
            .keys()
            .copied()
            .filter(|src_hwnd| !desired.contains(src_hwnd))
            .collect();
        for src_hwnd in stale {
            if let Some(entry) = self.thumbnails.remove(&src_hwnd) {
                unsafe {
                    let _ = DwmUnregisterThumbnail(entry.handle);
                }
            }
        }

        let mut processed = HashSet::with_capacity(desired.len());
        for &(src_hwnd, slot_rect) in slots {
            if !desired.contains(&src_hwnd) || !processed.insert(src_hwnd) {
                continue;
            }
            let src = HWND(src_hwnd as *mut c_void);

            let (handle, prev_rect, prev_opacity) =
                if let Some(&entry) = self.thumbnails.get(&src_hwnd) {
                    (
                        entry.handle,
                        Some(entry.last_dest),
                        Some(entry.last_opacity),
                    )
                } else {
                    let Ok(handle) = (unsafe { DwmRegisterThumbnail(dest, src) }) else {
                        continue;
                    };
                    (handle, None, None)
                };

            let (handle, prev_rect, prev_opacity, src_size) =
                match unsafe { DwmQueryThumbnailSourceSize(handle) } {
                    Ok(sz) => (handle, prev_rect, prev_opacity, sz),
                    Err(_) if prev_rect.is_some() => {
                        unsafe {
                            let _ = DwmUnregisterThumbnail(handle);
                        }
                        self.thumbnails.remove(&src_hwnd);
                        let Ok(new_handle) = (unsafe { DwmRegisterThumbnail(dest, src) }) else {
                            continue;
                        };
                        let Ok(sz) = (unsafe { DwmQueryThumbnailSourceSize(new_handle) }) else {
                            unsafe {
                                let _ = DwmUnregisterThumbnail(new_handle);
                            }
                            continue;
                        };
                        (new_handle, None, None, sz)
                    }
                    Err(_) => {
                        unsafe {
                            let _ = DwmUnregisterThumbnail(handle);
                        }
                        self.thumbnails.remove(&src_hwnd);
                        continue;
                    }
                };

            let fitted = fit_aspect_rect(src_size.cx, src_size.cy, slot_rect);
            if prev_rect == Some(fitted) && prev_opacity == Some(self.opacity) {
                continue;
            }

            let props = DWM_THUMBNAIL_PROPERTIES {
                dwFlags: DWM_TNP_RECTDESTINATION
                    | DWM_TNP_VISIBLE
                    | DWM_TNP_OPACITY
                    | DWM_TNP_SOURCECLIENTAREAONLY,
                rcDestination: RECT {
                    left: fitted.x,
                    top: fitted.y,
                    right: fitted.right(),
                    bottom: fitted.bottom(),
                },
                rcSource: RECT::default(),
                opacity: self.opacity,
                fVisible: TRUE,
                fSourceClientAreaOnly: FALSE,
            };

            if unsafe { DwmUpdateThumbnailProperties(handle, &props) }.is_err() {
                unsafe {
                    let _ = DwmUnregisterThumbnail(handle);
                }
                self.thumbnails.remove(&src_hwnd);
                continue;
            }

            self.thumbnails.insert(
                src_hwnd,
                ThumbnailSlotEntry {
                    handle,
                    last_dest: fitted,
                    last_opacity: self.opacity,
                },
            );
        }
    }

    /// Updates DWM composition opacity (`0..=255`) for all registered thumbnails.
    pub fn set_opacity(&mut self, opacity: u8) {
        if self.opacity == opacity {
            return;
        }
        self.opacity = opacity;
        for entry in self.thumbnails.values_mut() {
            let props = DWM_THUMBNAIL_PROPERTIES {
                dwFlags: DWM_TNP_OPACITY,
                opacity,
                ..Default::default()
            };
            if unsafe { DwmUpdateThumbnailProperties(entry.handle, &props) }.is_ok() {
                entry.last_opacity = opacity;
            }
        }
    }

    /// Unregisters all active DWM thumbnail handles and clears destination tracking.
    pub fn clear(&mut self) {
        for (_, entry) in self.thumbnails.drain() {
            unsafe {
                let _ = DwmUnregisterThumbnail(entry.handle);
            }
        }
        self.dest_hwnd = 0;
    }
}

impl Drop for DwmThumbnailHost {
    fn drop(&mut self) {
        self.clear();
    }
}

pub(crate) struct ScreenDcGuard(pub(crate) HDC);

impl Drop for ScreenDcGuard {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = ReleaseDC(None, self.0);
            }
        }
    }
}

pub(crate) struct MemDcGuard(pub(crate) HDC);

impl Drop for MemDcGuard {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = DeleteDC(self.0);
            }
        }
    }
}

pub(crate) struct BitmapGuard(pub(crate) HBITMAP);

impl Drop for BitmapGuard {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = DeleteObject(self.0.into());
            }
        }
    }
}

pub(crate) struct SelectedObjectGuard {
    pub(crate) hdc: HDC,
    pub(crate) prev: HGDIOBJ,
}

impl Drop for SelectedObjectGuard {
    fn drop(&mut self) {
        if !self.hdc.is_invalid() && !self.prev.is_invalid() {
            unsafe {
                let _ = SelectObject(self.hdc, self.prev);
            }
        }
    }
}

/// Captures a static fallback RGBA8 snapshot of `hwnd_raw` via `PrintWindow` + top-down 32bpp
/// DIBSection, downscaled cleanly to at most `max_w x max_h`.
pub fn capture_window_thumbnail(hwnd_raw: isize, max_w: u32, max_h: u32) -> Option<RgbaIcon> {
    if hwnd_raw == 0 || max_w == 0 || max_h == 0 {
        return None;
    }
    let hwnd = HWND(hwnd_raw as *mut c_void);
    if !unsafe { IsWindow(Some(hwnd)) }.as_bool()
        || !unsafe { IsWindowVisible(hwnd) }.as_bool()
        || unsafe { IsIconic(hwnd) }.as_bool()
    {
        return None;
    }

    let mut rect = RECT::default();
    if unsafe { GetWindowRect(hwnd, &mut rect) }.is_err() {
        return None;
    }
    let src_w = rect.right - rect.left;
    let src_h = rect.bottom - rect.top;
    if src_w <= 0 || src_h <= 0 || src_w > MAX_CAPTURE_DIM || src_h > MAX_CAPTURE_DIM {
        return None;
    }

    let screen_dc = ScreenDcGuard(unsafe { GetDC(None) });
    if screen_dc.0.is_invalid() {
        return None;
    }

    let mem_dc = MemDcGuard(unsafe { CreateCompatibleDC(Some(screen_dc.0)) });
    if mem_dc.0.is_invalid() {
        return None;
    }

    let mut bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: src_w,
            biHeight: -src_h, // top-down DIB
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };

    let mut bits_ptr: *mut c_void = std::ptr::null_mut();
    let hbm = unsafe {
        CreateDIBSection(
            Some(screen_dc.0),
            &bmi,
            DIB_RGB_COLORS,
            &mut bits_ptr,
            None,
            0,
        )
    }
    .ok()?;
    let bitmap = BitmapGuard(hbm);
    if bitmap.0.is_invalid() || bits_ptr.is_null() {
        return None;
    }

    let prev_obj = unsafe { SelectObject(mem_dc.0, bitmap.0.into()) };
    if prev_obj.is_invalid() {
        return None;
    }
    let selected = SelectedObjectGuard {
        hdc: mem_dc.0,
        prev: prev_obj,
    };

    let mut printed =
        unsafe { PrintWindow(hwnd, mem_dc.0, PRINT_WINDOW_FLAGS(PW_RENDERFULLCONTENT)) }.as_bool();
    if !printed {
        printed = unsafe { PrintWindow(hwnd, mem_dc.0, PRINT_WINDOW_FLAGS(0)) }.as_bool();
    }
    // Unselect before calling GetDIBits as required by Win32 GDI.
    drop(selected);
    if !printed {
        return None;
    }

    let pixel_bytes = (src_w as usize) * (src_h as usize) * 4;
    let mut pixels = vec![0u8; pixel_bytes];
    let lines = unsafe {
        GetDIBits(
            screen_dc.0,
            bitmap.0,
            0,
            src_h as u32,
            Some(pixels.as_mut_ptr().cast()),
            &mut bmi,
            DIB_RGB_COLORS,
        )
    };
    drop(bitmap);
    drop(mem_dc);
    drop(screen_dc);

    if lines <= 0 {
        return None;
    }

    let mut any_alpha = false;
    let mut any_rgb = false;
    for px in pixels.as_chunks_mut::<4>().0 {
        px.swap(0, 2); // BGRA -> RGBA
        if px[3] != 0 {
            any_alpha = true;
        }
        if px[0] != 0 || px[1] != 0 || px[2] != 0 {
            any_rgb = true;
        }
    }
    if !any_alpha && !any_rgb {
        return None;
    }
    if !any_alpha {
        for px in pixels.as_chunks_mut::<4>().0 {
            px[3] = 255;
        }
    }

    let (out_w, out_h, out_pixels) =
        downscale_rgba(&pixels, src_w as u32, src_h as u32, max_w, max_h);
    Some(RgbaIcon {
        width: out_w,
        height: out_h,
        pixels: Arc::from(out_pixels),
    })
}

fn downscale_rgba(
    src: &[u8],
    src_w: u32,
    src_h: u32,
    max_w: u32,
    max_h: u32,
) -> (u32, u32, Vec<u8>) {
    let scale = (f64::from(max_w) / f64::from(src_w))
        .min(f64::from(max_h) / f64::from(src_h))
        .min(1.0);
    let dst_w = ((f64::from(src_w) * scale).round() as u32).clamp(1, max_w);
    let dst_h = ((f64::from(src_h) * scale).round() as u32).clamp(1, max_h);

    if dst_w == src_w && dst_h == src_h {
        return (src_w, src_h, src.to_vec());
    }

    let mut out = vec![0u8; (dst_w as usize) * (dst_h as usize) * 4];
    let src_w_u64 = u64::from(src_w);
    let src_h_u64 = u64::from(src_h);
    let dst_w_u64 = u64::from(dst_w);
    let dst_h_u64 = u64::from(dst_h);

    for dy in 0..dst_h {
        let dy_u64 = u64::from(dy);
        let sy0 = (dy_u64 * src_h_u64 / dst_h_u64) as usize;
        let sy1 = (((dy_u64 + 1) * src_h_u64 / dst_h_u64) as usize).clamp(sy0 + 1, src_h as usize);

        for dx in 0..dst_w {
            let dx_u64 = u64::from(dx);
            let sx0 = (dx_u64 * src_w_u64 / dst_w_u64) as usize;
            let sx1 =
                (((dx_u64 + 1) * src_w_u64 / dst_w_u64) as usize).clamp(sx0 + 1, src_w as usize);

            let mut r_sum: u64 = 0;
            let mut g_sum: u64 = 0;
            let mut b_sum: u64 = 0;
            let mut a_sum: u64 = 0;
            let mut count: u64 = 0;

            for sy in sy0..sy1 {
                let row_offset = sy * (src_w as usize) * 4;
                for sx in sx0..sx1 {
                    let idx = row_offset + sx * 4;
                    r_sum += u64::from(src[idx]);
                    g_sum += u64::from(src[idx + 1]);
                    b_sum += u64::from(src[idx + 2]);
                    a_sum += u64::from(src[idx + 3]);
                    count += 1;
                }
            }

            let half = count / 2;
            let out_idx = ((dy as usize) * (dst_w as usize) + (dx as usize)) * 4;
            out[out_idx] = ((r_sum + half) / count) as u8;
            out[out_idx + 1] = ((g_sum + half) / count) as u8;
            out[out_idx + 2] = ((b_sum + half) / count) as u8;
            out[out_idx + 3] = ((a_sum + half) / count) as u8;
        }
    }

    (dst_w, dst_h, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_aspect_rect_preserves_aspect_and_centers() {
        let slot = PhysicalRect {
            x: 100,
            y: 200,
            width: 188,
            height: 112,
        };

        // Wide 16:9 source fits horizontally (188x106) and centers vertically.
        let wide = fit_aspect_rect(1920, 1080, slot);
        assert_eq!(wide.width, 188);
        assert_eq!(wide.height, 106);
        assert_eq!(wide.x, 100);
        assert_eq!(wide.y, 203);
        assert!(wide.right() <= slot.right());
        assert!(wide.bottom() <= slot.bottom());

        // Tall 1:2 portrait source fits vertically (56x112) and centers horizontally.
        let tall = fit_aspect_rect(500, 1000, slot);
        assert_eq!(tall.width, 56);
        assert_eq!(tall.height, 112);
        assert_eq!(tall.x, 166);
        assert_eq!(tall.y, 200);
        assert!(tall.right() <= slot.right());
        assert!(tall.bottom() <= slot.bottom());

        // Zero or negative inputs are guarded cleanly.
        assert_eq!(fit_aspect_rect(0, 1080, slot), slot);
        assert_eq!(fit_aspect_rect(1920, -10, slot), slot);
        assert_eq!(
            fit_aspect_rect(
                1920,
                1080,
                PhysicalRect {
                    x: 10,
                    y: 20,
                    width: 0,
                    height: -5,
                }
            ),
            PhysicalRect {
                x: 10,
                y: 20,
                width: 0,
                height: 0,
            }
        );
    }

    #[test]
    fn dwm_thumbnail_host_clear_empty_never_panics() {
        let mut host = DwmThumbnailHost::new();
        host.clear();
        host.sync_slots(0, &[]);
        host.sync_slots(
            0,
            &[(
                1234,
                PhysicalRect {
                    x: 10,
                    y: 10,
                    width: 188,
                    height: 112,
                },
            )],
        );
        host.clear();
    }

    #[test]
    fn capture_thumbnail_invalid_hwnd_returns_none() {
        assert_eq!(capture_window_thumbnail(0, 188, 112), None);
        assert_eq!(capture_window_thumbnail(-1, 188, 112), None);
        assert_eq!(capture_window_thumbnail(0x1234_5678, 0, 112), None);
        assert_eq!(capture_window_thumbnail(0x1234_5678, 188, 0), None);
    }

    #[test]
    fn dwm_thumbnail_host_default_opacity_is_opaque_and_updates_safely() {
        let mut host = DwmThumbnailHost::new();
        assert_eq!(host.opacity, 255);
        assert_eq!(DwmThumbnailHost::default().opacity, 255);
        host.set_opacity(128);
        assert_eq!(host.opacity, 128);
        host.set_opacity(255);
        assert_eq!(host.opacity, 255);
    }
}
