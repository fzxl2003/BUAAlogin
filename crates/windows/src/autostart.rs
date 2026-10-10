use std::{
    os::windows::ffi::OsStrExt,
    ptr::{null, null_mut},
};
use windows_sys::Win32::{Foundation::*, System::Registry::*};
const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const VALUE: &str = "BUAALoginNativeV1";
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
struct Key(HKEY);
impl Drop for Key {
    fn drop(&mut self) {
        unsafe {
            RegCloseKey(self.0);
        }
    }
}
pub fn enabled() -> Result<bool, String> {
    unsafe {
        let mut bytes = 0u32;
        match RegGetValueW(
            HKEY_CURRENT_USER,
            wide(RUN_KEY).as_ptr(),
            wide(VALUE).as_ptr(),
            RRF_RT_REG_SZ,
            null_mut(),
            null_mut(),
            &mut bytes,
        ) {
            ERROR_SUCCESS => Ok(bytes > 2),
            ERROR_FILE_NOT_FOUND => Ok(false),
            _ => Err("无法读取开机自启设置".into()),
        }
    }
}
pub fn set_enabled(enabled: bool) -> Result<(), String> {
    unsafe {
        let mut handle = null_mut();
        let result = if enabled {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                wide(RUN_KEY).as_ptr(),
                0,
                null(),
                REG_OPTION_NON_VOLATILE,
                KEY_SET_VALUE,
                null(),
                &mut handle,
                null_mut(),
            )
        } else {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                wide(RUN_KEY).as_ptr(),
                0,
                KEY_SET_VALUE,
                &mut handle,
            )
        };
        if !enabled && result == ERROR_FILE_NOT_FOUND {
            return Ok(());
        }
        if result != ERROR_SUCCESS {
            return Err("无法修改开机自启设置".into());
        }
        let key = Key(handle);
        let result = if enabled {
            let path = std::env::current_exe().map_err(|_| "无法确定程序位置")?;
            // Quote the executable path directly; no shell and no credentials in arguments.
            let mut command = vec![b'"' as u16];
            command.extend(path.as_os_str().encode_wide());
            command.extend("\" --autostart".encode_utf16());
            if command.len() > 260 {
                return Err("程序路径过长，请移到较短的固定路径后开启开机自启".into());
            }
            command.push(0);
            RegSetValueExW(
                key.0,
                wide(VALUE).as_ptr(),
                0,
                REG_SZ,
                command.as_ptr() as *const u8,
                (command.len() * 2) as u32,
            )
        } else {
            RegDeleteValueW(key.0, wide(VALUE).as_ptr())
        };
        if result == ERROR_SUCCESS || (!enabled && result == ERROR_FILE_NOT_FOUND) {
            Ok(())
        } else {
            Err("无法保存开机自启设置".into())
        }
    }
}
