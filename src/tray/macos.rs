use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crossbeam_channel::{Receiver, TryRecvError};

use crate::UiState;

pub enum TrayCmd {
    UpdateState(UiState),
    Quit,
}

pub fn run_tray(running: Arc<AtomicBool>, cmd_rx: Receiver<TrayCmd>) -> anyhow::Result<()> {
    use objc::runtime::{Class, Object, BOOL, NO, YES};
    use objc::{class, msg_send, sel, sel_impl};
    use std::ffi::c_void;

    fn get_status_icon(state: &UiState) -> &'static str {
        match state {
            UiState::Idle => "🎤",
            UiState::Recording { .. } => "🔴",
            UiState::Transcribing { .. } => "⏳",
            UiState::Done { .. } => "✅",
            UiState::Error { .. } => "❌",
        }
    }

    fn create_status_item() -> *mut Object {
        unsafe {
            let ns_status_bar = class!(NSStatusBar);
            let status_bar: *mut Object = msg_send![ns_status_bar, systemStatusBar];
            let status_item: *mut Object = msg_send![status_bar, statusItemWithLength:-1.0];
            status_item
        }
    }

    fn set_status_title(status_item: *mut Object, title: &str) {
        unsafe {
            let ns_string = class!(NSString);
            let title_str: *mut Object = msg_send![ns_string, alloc];
            let title_bytes = title.as_ptr() as *const i8;
            let title_len = title.len() as u64;
            let title_obj: *mut Object =
                msg_send![title_str, initWithBytes:title_bytes length:title_len encoding:4];
            let _: () = msg_send![status_item, setTitle:title_obj];
        }
    }

    let status_item = create_status_item();
    set_status_title(status_item, "🎤");

    loop {
        if !running.load(Ordering::SeqCst) {
            break;
        }

        match cmd_rx.try_recv() {
            Ok(TrayCmd::UpdateState(state)) => {
                let icon = get_status_icon(&state);
                set_status_title(status_item, icon);
            }
            Ok(TrayCmd::Quit) => {
                running.store(false, Ordering::SeqCst);
                break;
            }
            Err(TryRecvError::Empty) => {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(TryRecvError::Disconnected) => {
                running.store(false, Ordering::SeqCst);
                break;
            }
        }
    }

    Ok(())
}

impl Clone for TrayCmd {
    fn clone(&self) -> Self {
        match self {
            TrayCmd::UpdateState(state) => TrayCmd::UpdateState(state.clone()),
            TrayCmd::Quit => TrayCmd::Quit,
        }
    }
}
