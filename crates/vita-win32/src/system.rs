use std::os::windows::process::CommandExt;
use std::process::Command;
use std::ptr::null_mut;
use windows::Win32::Devices::Display::{
    DestroyPhysicalMonitors, GetMonitorBrightness, GetNumberOfPhysicalMonitorsFromHMONITOR,
    GetPhysicalMonitorsFromHMONITOR, PHYSICAL_MONITOR, SetMonitorBrightness,
};
use windows::Win32::Foundation::{POINT, RPC_E_TOO_LATE};
use windows::Win32::Graphics::Gdi::{MONITOR_DEFAULTTOPRIMARY, MonitorFromPoint};
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{IMMDeviceEnumerator, MMDeviceEnumerator, eMultimedia, eRender};
use windows::Win32::System::Com::{
    CLSCTX_ALL, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
    CoInitializeSecurity, CoSetProxyBlanket, CoUninitialize, EOAC_NONE, RPC_C_AUTHN_LEVEL_CALL,
    RPC_C_AUTHN_LEVEL_DEFAULT, RPC_C_IMP_LEVEL_IMPERSONATE,
};
use windows::Win32::System::Power::SetSuspendState;
use windows::Win32::System::Shutdown::LockWorkStation;
use windows::Win32::System::Threading::CREATE_NO_WINDOW;
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::System::Wmi::{
    IWbemClassObject, IWbemLocator, IWbemServices, WBEM_FLAG_FORWARD_ONLY,
    WBEM_FLAG_RETURN_WBEM_COMPLETE, WbemLocator,
};
use windows::core::{BSTR, w};

const RPC_C_AUTHN_WINNT: u32 = 10;
const RPC_C_AUTHZ_NONE: u32 = 0;

struct MtaComGuard {
    should_uninit: bool,
}

impl MtaComGuard {
    fn init() -> Self {
        let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        Self {
            should_uninit: hr.is_ok(),
        }
    }
}

impl Drop for MtaComGuard {
    fn drop(&mut self) {
        if self.should_uninit {
            unsafe { CoUninitialize() };
        }
    }
}

/// Sets the default audio render device volume (`0..=100`).
pub fn set_master_volume(pct: u8) -> bool {
    let _com = MtaComGuard::init();
    let scalar = f32::from(pct.min(100)) / 100.0;
    let res: windows::core::Result<()> = (|| {
        let enumerator: IMMDeviceEnumerator =
            unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
        let device = unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eMultimedia)? };
        let endpoint: IAudioEndpointVolume = unsafe { device.Activate(CLSCTX_ALL, None)? };
        unsafe { endpoint.SetMasterVolumeLevelScalar(scalar, null_mut())? };
        if pct > 0 {
            let _ = unsafe { endpoint.SetMute(false, null_mut()) };
        }
        Ok(())
    })();
    res.is_ok()
}

/// Mutes or unmutes the default audio render endpoint.
pub fn set_master_mute(muted: bool) -> bool {
    let _com = MtaComGuard::init();
    let res: windows::core::Result<()> = (|| {
        let enumerator: IMMDeviceEnumerator =
            unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
        let device = unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eMultimedia)? };
        let endpoint: IAudioEndpointVolume = unsafe { device.Activate(CLSCTX_ALL, None)? };
        unsafe { endpoint.SetMute(muted, null_mut())? };
        Ok(())
    })();
    res.is_ok()
}

fn connect_wmi_root_wmi() -> Option<(MtaComGuard, IWbemServices)> {
    let com = MtaComGuard::init();
    let sec_hr = unsafe {
        CoInitializeSecurity(
            None,
            -1,
            None,
            None,
            RPC_C_AUTHN_LEVEL_DEFAULT,
            RPC_C_IMP_LEVEL_IMPERSONATE,
            None,
            EOAC_NONE,
            None,
        )
    };
    if let Err(err) = sec_hr
        && err.code() != RPC_E_TOO_LATE
    {
        return None;
    }

    let locator: IWbemLocator =
        unsafe { CoCreateInstance(&WbemLocator, None, CLSCTX_INPROC_SERVER) }.ok()?;
    let ns = BSTR::from(r"ROOT\WMI");
    let services = unsafe {
        locator.ConnectServer(
            &ns,
            &BSTR::default(),
            &BSTR::default(),
            &BSTR::default(),
            0,
            &BSTR::default(),
            None,
        )
    }
    .ok()?;

    let _ = unsafe {
        CoSetProxyBlanket(
            &services,
            RPC_C_AUTHN_WINNT,
            RPC_C_AUTHZ_NONE,
            None,
            RPC_C_AUTHN_LEVEL_CALL,
            RPC_C_IMP_LEVEL_IMPERSONATE,
            None,
            EOAC_NONE,
        )
    };

    Some((com, services))
}

pub(crate) fn read_wmi_brightness() -> Option<u8> {
    let (_com, services) = connect_wmi_root_wmi()?;
    let enumerator = unsafe {
        services.ExecQuery(
            &BSTR::from("WQL"),
            &BSTR::from("SELECT CurrentBrightness FROM WmiMonitorBrightness WHERE Active = TRUE"),
            WBEM_FLAG_RETURN_WBEM_COMPLETE | WBEM_FLAG_FORWARD_ONLY,
            None,
        )
    }
    .ok()?;

    let mut objs = [None; 1];
    let mut returned = 0u32;
    let _ = unsafe { enumerator.Next(2000, &mut objs, &mut returned) };
    if returned == 0 {
        return None;
    }
    let obj = objs[0].as_ref()?;
    let mut var = VARIANT::default();
    unsafe { obj.Get(w!("CurrentBrightness"), 0, &mut var, None, None) }.ok()?;
    let val = i32::try_from(&var)
        .ok()
        .or_else(|| u32::try_from(&var).ok().map(|v| v as i32))
        .map(|v| v.clamp(0, 100) as u8)?;
    Some(val)
}

pub(crate) fn read_ddc_brightness() -> Option<u8> {
    let hmon = unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) };
    let mut count: u32 = 0;
    if unsafe { GetNumberOfPhysicalMonitorsFromHMONITOR(hmon, &mut count) }.is_err() || count == 0 {
        return None;
    }
    let mut monitors = vec![PHYSICAL_MONITOR::default(); count as usize];
    if unsafe { GetPhysicalMonitorsFromHMONITOR(hmon, &mut monitors) }.is_err() {
        return None;
    }
    let mut min_b = 0u32;
    let mut cur_b = 0u32;
    let mut max_b = 0u32;
    let ok = unsafe {
        GetMonitorBrightness(
            monitors[0].hPhysicalMonitor,
            &mut min_b,
            &mut cur_b,
            &mut max_b,
        )
    } != 0;
    unsafe {
        let _ = DestroyPhysicalMonitors(&monitors);
    }
    if ok && max_b > min_b {
        let pct = ((cur_b - min_b) as f32 / (max_b - min_b) as f32 * 100.0)
            .round()
            .clamp(0.0, 100.0) as u8;
        Some(pct)
    } else {
        None
    }
}

/// Sets internal laptop display brightness via WMI `WmiSetBrightness`, falling back to DDC/CI.
pub fn set_display_brightness(pct: u8) -> bool {
    let clamped = pct.min(100);
    if set_wmi_brightness(clamped) {
        return true;
    }
    set_ddc_brightness(clamped)
}

fn set_wmi_brightness(pct: u8) -> bool {
    let Some((_com, services)) = connect_wmi_root_wmi() else {
        return false;
    };

    let class_name = BSTR::from("WmiMonitorBrightnessMethods");
    let method_name = BSTR::from("WmiSetBrightness");
    let mut class_obj: Option<IWbemClassObject> = None;
    if unsafe {
        services.GetObject(
            &class_name,
            Default::default(),
            None,
            Some(&mut class_obj),
            None,
        )
    }
    .is_err()
    {
        return false;
    }
    let Some(class_obj) = class_obj else {
        return false;
    };

    let mut in_sig: Option<IWbemClassObject> = None;
    if unsafe { class_obj.GetMethod(&method_name, 0, &raw mut in_sig, null_mut()) }.is_err() {
        return false;
    }
    let Some(in_sig) = in_sig else {
        return false;
    };
    let Ok(in_params) = (unsafe { in_sig.SpawnInstance(0) }) else {
        return false;
    };

    let timeout_var = VARIANT::from(0u32);
    let bright_var = VARIANT::from(pct);
    if unsafe { in_params.Put(w!("Timeout"), 0, &timeout_var, 0) }.is_err() {
        return false;
    }
    if unsafe { in_params.Put(w!("Brightness"), 0, &bright_var, 0) }.is_err() {
        return false;
    }

    let Ok(enumerator) = (unsafe {
        services.ExecQuery(
            &BSTR::from("WQL"),
            &BSTR::from("SELECT * FROM WmiMonitorBrightnessMethods WHERE Active = TRUE"),
            WBEM_FLAG_RETURN_WBEM_COMPLETE | WBEM_FLAG_FORWARD_ONLY,
            None,
        )
    }) else {
        return false;
    };

    let mut any_set = false;
    loop {
        let mut objs = [None; 1];
        let mut returned = 0u32;
        if unsafe { enumerator.Next(2000, &mut objs, &mut returned) }.is_err() || returned == 0 {
            break;
        }
        let Some(instance) = &objs[0] else {
            continue;
        };
        let mut relpath_var = VARIANT::default();
        if unsafe { instance.Get(w!("__RELPATH"), 0, &mut relpath_var, None, None) }.is_err() {
            continue;
        }
        let Ok(relpath_bstr) = BSTR::try_from(&relpath_var) else {
            continue;
        };
        if unsafe {
            services.ExecMethod(
                &relpath_bstr,
                &method_name,
                Default::default(),
                None,
                Some(&in_params),
                None,
                None,
            )
        }
        .is_ok()
        {
            any_set = true;
        }
    }

    any_set
}

fn set_ddc_brightness(pct: u8) -> bool {
    let hmon = unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) };
    let mut count: u32 = 0;
    if unsafe { GetNumberOfPhysicalMonitorsFromHMONITOR(hmon, &mut count) }.is_err() || count == 0 {
        return false;
    }
    let mut monitors = vec![PHYSICAL_MONITOR::default(); count as usize];
    if unsafe { GetPhysicalMonitorsFromHMONITOR(hmon, &mut monitors) }.is_err() {
        return false;
    }
    let mut any_ok = false;
    for mon in &monitors {
        let mut min_b = 0u32;
        let mut cur_b = 0u32;
        let mut max_b = 0u32;
        if unsafe { GetMonitorBrightness(mon.hPhysicalMonitor, &mut min_b, &mut cur_b, &mut max_b) }
            != 0
            && max_b > min_b
        {
            let target = min_b + ((max_b - min_b) as f32 * f32::from(pct) / 100.0).round() as u32;
            if unsafe { SetMonitorBrightness(mon.hPhysicalMonitor, target.clamp(min_b, max_b)) }
                != 0
            {
                any_ok = true;
            }
        }
    }
    unsafe {
        let _ = DestroyPhysicalMonitors(&monitors);
    }
    any_ok
}

pub fn lock_workstation() {
    unsafe {
        let _ = LockWorkStation();
    }
}

pub fn sleep_system() {
    unsafe {
        let _ = SetSuspendState(false, false, false);
    }
}

pub fn restart_system() {
    let _ = Command::new("shutdown.exe")
        .args(["/r", "/t", "0"])
        .creation_flags(CREATE_NO_WINDOW.0)
        .spawn();
}

pub fn shutdown_system() {
    let _ = Command::new("shutdown.exe")
        .args(["/s", "/t", "0"])
        .creation_flags(CREATE_NO_WINDOW.0)
        .spawn();
}
