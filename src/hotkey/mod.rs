#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "linux")]
pub use linux::{HotkeyEvent, start_hotkey_listener};

#[cfg(target_os = "macos")]
pub use macos::{HotkeyEvent, start_hotkey_listener};
