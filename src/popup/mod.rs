#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "linux")]
pub use linux::{get_cursor_position, hide_popup, init_gtk, process_events, show_popup};

#[cfg(target_os = "macos")]
pub use macos::{get_cursor_position, hide_popup, init_popup, process_events, show_popup};
