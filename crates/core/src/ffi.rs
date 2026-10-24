//! Version 1 C ABI. Input strings are borrowed for the call; event JSON is
//! borrowed only during callback. free joins the worker before dropping context.
use crate::{
    engine::{Handle, Mode, TestKind},
    Config,
};
use std::{
    ffi::{c_char, c_void, CStr, CString},
    panic::{catch_unwind, AssertUnwindSafe},
};
type Callback = unsafe extern "C" fn(*mut c_void, *const c_char);
pub struct Client {
    config: Config,
    callback: Callback,
    context: usize,
    handle: Option<Handle>,
    mode: Option<Mode>,
}
#[no_mangle]
pub extern "C" fn buaa_abi_version() -> u32 {
    1
}
/// # Safety
/// json must point to a readable NUL-terminated UTF-8 configuration for this call.
/// Callback context must remain valid until free returns; callbacks cannot unwind.
#[no_mangle]
pub unsafe extern "C" fn buaa_create(
    json: *const c_char,
    callback: Option<Callback>,
    context: *mut c_void,
) -> *mut Client {
    catch_unwind(AssertUnwindSafe(|| {
        if json.is_null() || callback.is_none() {
            return std::ptr::null_mut();
        }
        let Ok(config) = serde_json::from_slice::<Config>(CStr::from_ptr(json).to_bytes()) else {
            return std::ptr::null_mut();
        };
        if config.validate(false).is_err() {
            return std::ptr::null_mut();
        }
        Box::into_raw(Box::new(Client {
            config,
            callback: callback.unwrap(),
            context: context as usize,
            handle: None,
            mode: None,
        }))
    }))
    .unwrap_or(std::ptr::null_mut())
}
unsafe fn start(client: *mut Client, mode: Mode) -> i32 {
    catch_unwind(AssertUnwindSafe(|| {
        let Some(client) = client.as_mut() else {
            return -1;
        };
        if client.handle.as_ref().is_some_and(|h| !h.is_finished()) {
            return -2;
        }
        client.handle.take();
        let callback = client.callback;
        let context = client.context;
        match Handle::spawn(client.config.clone(), mode, move |event| {
            if let Ok(text) = serde_json::to_string(&event) {
                if let Ok(text) = CString::new(text) {
                    callback(context as *mut c_void, text.as_ptr());
                }
            }
        }) {
            Ok(handle) => {
                client.handle = Some(handle);
                client.mode = Some(mode);
                0
            }
            Err(_) => -3,
        }
    }))
    .unwrap_or(-4)
}
/// # Safety
/// The client must be live (or null), used serially by its owning thread.
/// Callback context must remain valid until free returns; callbacks cannot unwind.
#[no_mangle]
pub unsafe extern "C" fn buaa_start(client: *mut Client) -> i32 {
    start(client, Mode::Monitor)
}
/// # Safety
/// The client must be live (or null), used serially by its owning thread.
/// Callback context must remain valid until free returns; callbacks cannot unwind.
#[no_mangle]
pub unsafe extern "C" fn buaa_scan(client: *mut Client) -> i32 {
    start(client, Mode::Inspect)
}
unsafe fn signal(client: *mut Client, action: u32) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if let Some(handle) = client.as_ref().and_then(|c| c.handle.as_ref()) {
            match action {
                0 => handle.signal.stop(),
                1 => handle.signal.network_changed(),
                2 => handle.signal.suspend(),
                3 => handle.signal.resume(),
                _ => handle.signal.tick(),
            }
        }
    }));
}
/// # Safety
/// The client must be live (or null), used serially by its owning thread.
/// Callback context must remain valid until free returns; callbacks cannot unwind.
#[no_mangle]
pub unsafe extern "C" fn buaa_stop(client: *mut Client) {
    signal(client, 0);
}
/// # Safety
/// The client must be live (or null), used serially by its owning thread.
#[no_mangle]
pub unsafe extern "C" fn buaa_recover(client: *mut Client, mode: u32) -> i32 {
    catch_unwind(AssertUnwindSafe(|| {
        let Some(client) = client.as_ref() else {
            return -1;
        };
        let Some(handle) = client
            .handle
            .as_ref()
            .filter(|handle| !handle.is_finished())
        else {
            return -2;
        };
        let mode = match mode {
            0 => crate::engine::RecoveryMode::Protocol,
            1 => crate::engine::RecoveryMode::Browser,
            _ => return -3,
        };
        handle.signal.recover(mode);
        0
    }))
    .unwrap_or(-4)
}
/// Run one real diagnostic. A live monitor serializes it with scheduled work;
/// an idle client starts a one-shot worker.
#[no_mangle]
pub unsafe extern "C" fn buaa_test(client: *mut Client, kind: u32) -> i32 {
    catch_unwind(AssertUnwindSafe(|| {
        let Some(client) = client.as_mut() else {
            return -1;
        };
        let kind = match kind {
            0 => TestKind::PortalAuth,
            1 => TestKind::Connectivity,
            2 => TestKind::Browser,
            _ => return -3,
        };
        if let Some(handle) = client.handle.as_ref().filter(|h| !h.is_finished()) {
            if matches!(client.mode, Some(Mode::Monitor)) {
                handle.signal.test(kind);
                return 0;
            }
            return -2;
        }
        start(client, Mode::Test(kind))
    }))
    .unwrap_or(-4)
}
/// # Safety
/// The client must be live (or null), used serially by its owning thread.
/// A live monitor applies the new interval after its current operation completes.
#[no_mangle]
pub unsafe extern "C" fn buaa_set_interval(client: *mut Client, seconds: u64) -> i32 {
    catch_unwind(AssertUnwindSafe(|| {
        let Some(client) = client.as_mut() else {
            return -1;
        };
        if !(10..=86400).contains(&seconds) {
            return -3;
        }
        if let Some(handle) = client
            .handle
            .as_ref()
            .filter(|handle| !handle.is_finished())
        {
            if matches!(client.mode, Some(Mode::Monitor))
                && handle.signal.set_interval(seconds).is_err()
            {
                return -2;
            }
        }
        client.config.interval = seconds;
        0
    }))
    .unwrap_or(-4)
}
/// # Safety
/// The client must be live (or null), used serially by its owning thread.
/// Callback context must remain valid until free returns; callbacks cannot unwind.
#[no_mangle]
pub unsafe extern "C" fn buaa_network_changed(client: *mut Client) {
    signal(client, 1);
}
/// # Safety
/// client follows the same ownership rules as buaa_start; name is a borrowed NUL-terminated UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn buaa_network_changed_interface(client: *mut Client, name: *const c_char) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if let Some(handle) = client.as_ref().and_then(|c| c.handle.as_ref()) {
            if !name.is_null() {
                handle
                    .signal
                    .network_changed_for(vec![CStr::from_ptr(name).to_string_lossy().into_owned()]);
            }
        }
    }));
}
/// # Safety
/// The client must be live (or null), used serially by its owning thread.
/// Callback context must remain valid until free returns; callbacks cannot unwind.
#[no_mangle]
pub unsafe extern "C" fn buaa_suspend(client: *mut Client) {
    signal(client, 2);
}
/// # Safety
/// The client must be live (or null), used serially by its owning thread.
/// Callback context must remain valid until free returns; callbacks cannot unwind.
#[no_mangle]
pub unsafe extern "C" fn buaa_resume(client: *mut Client) {
    signal(client, 3);
}
/// # Safety
/// The client must be live (or null), used serially by its owning thread.
/// Callback context must remain valid until free returns; callbacks cannot unwind.
#[no_mangle]
pub unsafe extern "C" fn buaa_tick(client: *mut Client) {
    signal(client, 4);
}
/// # Safety
/// The client must be live (or null), used serially by its owning thread.
/// Callback context must remain valid until free returns; callbacks cannot unwind.
#[no_mangle]
pub unsafe extern "C" fn buaa_free(client: *mut Client) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if !client.is_null() {
            drop(Box::from_raw(client));
        }
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Event;
    unsafe extern "C" fn receive(context: *mut c_void, json: *const c_char) {
        let tx = &*(context as *const std::sync::mpsc::Sender<Event>);
        let event =
            serde_json::from_slice::<EventForTest>(CStr::from_ptr(json).to_bytes()).unwrap();
        let _ = tx.send(Event::new(&event.state, ""));
    }
    #[derive(serde::Deserialize)]
    struct EventForTest {
        state: String,
    }
    #[test]
    fn invalid_config_and_null_notifications_are_safe() {
        unsafe {
            assert_eq!(buaa_abi_version(), 1);
            assert!(buaa_create(c"{}".as_ptr(), None, std::ptr::null_mut()).is_null());
            assert!(buaa_create(
                c"{\"interval\":0}".as_ptr(),
                Some(receive),
                std::ptr::null_mut()
            )
            .is_null());
            assert_eq!(buaa_start(std::ptr::null_mut()), -1);
            assert_eq!(buaa_recover(std::ptr::null_mut(), 0), -1);
            assert_eq!(buaa_test(std::ptr::null_mut(), 0), -1);
            assert_eq!(buaa_set_interval(std::ptr::null_mut(), 10), -1);
            buaa_stop(std::ptr::null_mut());
            buaa_free(std::ptr::null_mut());
        }
    }
    #[test]
    fn interval_validation_and_live_updates_preserve_the_new_value_on_stop() {
        unsafe extern "C" fn receive_json(context: *mut c_void, json: *const c_char) {
            let tx = &*(context as *const std::sync::mpsc::Sender<serde_json::Value>);
            let event = serde_json::from_slice(CStr::from_ptr(json).to_bytes()).unwrap();
            let _ = tx.send(event);
        }
        unsafe {
            let (tx, rx) = std::sync::mpsc::channel::<serde_json::Value>();
            let context = Box::into_raw(Box::new(tx));
            let json = c"{\"username\":\"test\",\"password\":\"test\",\"interface\":\"nonexistent-test-interface\",\"external_timer\":true}";
            let client = buaa_create(json.as_ptr(), Some(receive_json), context as _);
            assert!(!client.is_null());
            for invalid in [0, 9, 86401, u64::MAX] {
                assert_eq!(buaa_set_interval(client, invalid), -3);
                assert_eq!((*client).config.interval, 300);
            }
            assert_eq!(buaa_set_interval(client, 10), 0);
            assert_eq!(buaa_start(client), 0);
            assert_eq!(buaa_set_interval(client, 37), 0);
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            let mut updated = false;
            loop {
                let event = rx
                    .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
                    .unwrap();
                if event["state"] == "dashboard" && event["dashboard"]["interval"] == 37 {
                    updated = true;
                }
                if event["state"] == "interval_changed" {
                    break;
                }
            }
            buaa_free(client);
            let events: Vec<_> = rx.try_iter().collect();
            drop(Box::from_raw(context));
            assert!(updated);
            let stopped = events
                .iter()
                .find(|event| event["state"] == "dashboard")
                .unwrap();
            assert_eq!(stopped["dashboard"]["interval"], 37);
            assert!(rx.recv().is_err());
        }
    }
    #[test]
    fn free_joins_worker_and_no_callbacks_outlive_context() {
        unsafe {
            let (tx, rx) = std::sync::mpsc::channel::<Event>();
            let context = Box::into_raw(Box::new(tx));
            let json = c"{\"username\":\"test\",\"password\":\"test\",\"interface\":\"nonexistent-test-interface\",\"external_timer\":true}";
            let client = buaa_create(json.as_ptr(), Some(receive), context as _);
            assert!(!client.is_null());
            assert_eq!(buaa_recover(client, 0), -2);
            assert_eq!(buaa_start(client), 0);
            // UI snapshots may precede the lifecycle event. Keep the same bounded wait.
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            let first = loop {
                let event = rx
                    .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
                    .unwrap();
                if event.state != "dashboard" {
                    break event;
                }
            };
            assert_eq!(first.state, "waiting_network");
            assert_eq!(buaa_start(client), -2);
            assert_eq!(buaa_recover(client, 2), -3);
            assert_eq!(buaa_test(client, 3), -3);
            buaa_stop(client);
            buaa_free(client);
            let events: Vec<_> = rx.try_iter().collect();
            assert_eq!(events.last().unwrap().state, "stopped");
            drop(Box::from_raw(context));
            assert!(rx.recv().is_err());
        }
    }
}
