#[cfg(all(target_os = "linux", not(feature = "gtk-tray")))]
mod linux_ksni;

#[cfg(all(target_os = "linux", feature = "gtk-tray"))]
mod linux_gtk;

#[cfg(target_os = "macos")]
mod macos;

#[cfg(all(target_os = "linux", not(feature = "gtk-tray")))]
pub use linux_ksni::{run_tray, TrayCmd};

#[cfg(all(target_os = "linux", feature = "gtk-tray"))]
pub use linux_gtk::{run_tray, TrayCmd};

#[cfg(target_os = "macos")]
pub use macos::{run_tray, TrayCmd};
