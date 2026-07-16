#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "linux")]
pub use linux::{backspace, type_text_auto};

#[cfg(target_os = "macos")]
pub use macos::{backspace, type_text_auto};
