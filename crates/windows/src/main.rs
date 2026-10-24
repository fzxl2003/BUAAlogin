#![cfg_attr(windows, windows_subsystem = "windows")]
#[cfg(windows)]
mod app;
#[cfg(windows)]
mod appearance;
#[cfg(windows)]
mod autostart;
#[cfg(any(windows, test))]
mod credentials;
#[cfg(windows)]
mod wifi;
fn main() {
    #[cfg(windows)]
    app::run();
    #[cfg(not(windows))]
    eprintln!("This entry point requires Windows.");
}
