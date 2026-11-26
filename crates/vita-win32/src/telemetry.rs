use crate::system::{read_ddc_brightness, read_wmi_brightness};
use std::mem::size_of;
use std::ptr::null_mut;
use vita_core::SystemSummary;
use windows::Win32::Devices::Bluetooth::{
    BLUETOOTH_FIND_RADIO_PARAMS, BluetoothFindFirstRadio, BluetoothFindRadioClose,
};
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{IMMDeviceEnumerator, MMDeviceEnumerator, eMultimedia, eRender};
use windows::Win32::NetworkManagement::WiFi::{
    WLAN_CONNECTION_ATTRIBUTES, WLAN_INTERFACE_INFO_LIST, WlanCloseHandle, WlanEnumInterfaces,
    WlanFreeMemory, WlanOpenHandle, WlanQueryInterface, wlan_interface_state_connected,
    wlan_intf_opcode_current_connection,
};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};

/// Reads live Battery, Master Audio Volume, Brightness, Bluetooth, and Wi-Fi SSID status.
pub fn read_system_summary() -> SystemSummary {
    let mut summary = SystemSummary::default();
    read_battery(&mut summary);
    read_audio(&mut summary);
    read_wifi(&mut summary);
    read_bluetooth(&mut summary);
    summary.brightness_pct = read_wmi_brightness().or_else(read_ddc_brightness);
    summary
}

fn read_battery(summary: &mut SystemSummary) {
    let mut status = SYSTEM_POWER_STATUS::default();
    if unsafe { GetSystemPowerStatus(&mut status) }.is_ok() {
        // BatteryFlag == 128 indicates "no system battery" (desktop PC).
        if status.BatteryFlag != 128 && status.BatteryLifePercent <= 100 {
            summary.battery_pct = Some(status.BatteryLifePercent);
            summary.is_charging = status.ACLineStatus == 1;
            if !summary.is_charging && status.BatteryLifeTime != u32::MAX {
                summary.battery_remaining_secs = Some(status.BatteryLifeTime);
            }
        }
    }
}

fn read_audio(summary: &mut SystemSummary) {
    let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    let should_uninit = hr.is_ok();

    let result = (|| -> windows::core::Result<(u8, bool)> {
        let enumerator: IMMDeviceEnumerator =
            unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
        let device = unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eMultimedia)? };
        let endpoint: IAudioEndpointVolume = unsafe { device.Activate(CLSCTX_ALL, None)? };
        let scalar = unsafe { endpoint.GetMasterVolumeLevelScalar()? };
        let muted = unsafe { endpoint.GetMute()? }.as_bool();
        Ok(((scalar * 100.0).round().clamp(0.0, 100.0) as u8, muted))
    })();

    if let Ok((pct, muted)) = result {
        summary.volume_pct = pct;
        summary.is_muted = muted;
    }

    if should_uninit {
        unsafe { CoUninitialize() };
    }
}

fn read_bluetooth(summary: &mut SystemSummary) {
    let params = BLUETOOTH_FIND_RADIO_PARAMS {
        dwSize: size_of::<BLUETOOTH_FIND_RADIO_PARAMS>() as u32,
    };
    let mut radio = HANDLE::default();
    let res = unsafe { BluetoothFindFirstRadio(&params, &mut radio) };
    match res {
        Ok(find_handle) => {
            summary.bluetooth_on = Some(true);
            if !radio.is_invalid() {
                unsafe {
                    let _ = CloseHandle(radio);
                }
            }
            unsafe {
                let _ = BluetoothFindRadioClose(find_handle);
            }
        }
        Err(_) => {
            summary.bluetooth_on = Some(false);
        }
    }
}

fn read_wifi(summary: &mut SystemSummary) {
    let mut negotiated_version: u32 = 0;
    let mut client_handle = HANDLE::default();
    let err = unsafe { WlanOpenHandle(2, None, &mut negotiated_version, &mut client_handle) };
    if err != 0 || client_handle.is_invalid() {
        return;
    }

    let mut list_ptr: *mut WLAN_INTERFACE_INFO_LIST = null_mut();
    let enum_err = unsafe { WlanEnumInterfaces(client_handle, None, &mut list_ptr) };
    if enum_err == 0 && !list_ptr.is_null() {
        let count = unsafe { (*list_ptr).dwNumberOfItems } as usize;
        let items =
            unsafe { std::slice::from_raw_parts((*list_ptr).InterfaceInfo.as_ptr(), count) };
        for iface in items {
            if iface.isState == wlan_interface_state_connected {
                summary.wifi_connected = true;
                let mut data_size: u32 = 0;
                let mut data_ptr: *mut std::ffi::c_void = null_mut();
                let qerr = unsafe {
                    WlanQueryInterface(
                        client_handle,
                        &iface.InterfaceGuid,
                        wlan_intf_opcode_current_connection,
                        None,
                        &mut data_size,
                        &mut data_ptr,
                        None,
                    )
                };
                if qerr == 0
                    && !data_ptr.is_null()
                    && (data_size as usize) >= size_of::<WLAN_CONNECTION_ATTRIBUTES>()
                {
                    let attrs = unsafe { &*(data_ptr as *const WLAN_CONNECTION_ATTRIBUTES) };
                    let ssid_struct = &attrs.wlanAssociationAttributes.dot11Ssid;
                    let len = (ssid_struct.uSSIDLength as usize).min(ssid_struct.ucSSID.len());
                    if len > 0 {
                        let ssid = String::from_utf8_lossy(&ssid_struct.ucSSID[..len]).to_string();
                        summary.wifi_ssid = Some(ssid);
                    }
                    unsafe {
                        WlanFreeMemory(data_ptr);
                    }
                }
                break;
            }
        }
        unsafe {
            WlanFreeMemory(list_ptr.cast());
        }
    }

    unsafe {
        let _ = WlanCloseHandle(client_handle, None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn telemetry_read_succeeds_safely() {
        let summary = read_system_summary();
        assert!(summary.volume_pct <= 100);
        if let Some(batt) = summary.battery_pct {
            assert!(batt <= 100);
        }
        if let Some(bright) = summary.brightness_pct {
            assert!(bright <= 100);
        }
        assert!(summary.bluetooth_on.is_some());
    }
}
