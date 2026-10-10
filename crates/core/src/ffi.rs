//! Version 1 C ABI. Input strings are borrowed for the call; event JSON is
//! borrowed only during callback. free joins the worker before dropping context.
use crate::{
    engine::{Handle, Mode},
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
            buaa_stop(std::ptr::null_mut());
            buaa_free(std::ptr::null_mut());
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
            buaa_stop(client);
            buaa_free(client);
            let events: Vec<_> = rx.try_iter().collect();
            assert_eq!(events.last().unwrap().state, "stopped");
            drop(Box::from_raw(context));
            assert!(rx.recv().is_err());
        }
    }
}
