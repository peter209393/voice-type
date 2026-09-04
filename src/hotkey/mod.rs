#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "linux")]
pub use linux::{shutdown, start_hotkey_listener, HotkeyEvent};

#[cfg(target_os = "macos")]
pub use macos::{start_hotkey_listener, HotkeyEvent};

#[cfg(target_os = "macos")]
#[allow(dead_code)]
pub fn shutdown() {}
