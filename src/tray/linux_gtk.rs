use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crossbeam_channel::{Receiver, TryRecvError};

use crate::UiState;

pub enum TrayCmd {
    UpdateState(UiState),
    Quit,
}

pub fn run_tray(running: Arc<AtomicBool>, cmd_rx: Receiver<TrayCmd>) -> anyhow::Result<()> {
    use gtk::prelude::*;
    use libappindicator::AppIndicator;

    let indicator = RefCell::new(AppIndicator::new("Voice Type", "audio-input-microphone"));

    let mut menu = gtk::Menu::new();

    let status_label = gtk::MenuItem::with_label("Voice Type - Idle");
    status_label.set_sensitive(false);
    menu.append(&status_label);

    let separator = gtk::SeparatorMenuItem::new();
    menu.append(&separator);

    let quit_item = gtk::MenuItem::with_label("Quit");
    let running_clone = running.clone();
    quit_item.connect_activate(move |_| {
        running_clone.store(false, Ordering::SeqCst);
        gtk::main_quit();
    });
    menu.append(&quit_item);

    menu.show_all();
    indicator.borrow_mut().set_menu(&mut menu);

    let (tx, rx) = std::sync::mpsc::channel::<Option<UiState>>();

    let running_clone = running.clone();
    std::thread::spawn(move || loop {
        match cmd_rx.try_recv() {
            Ok(TrayCmd::UpdateState(state)) => {
                let _ = tx.send(Some(state));
            }
            Ok(TrayCmd::Quit) => {
                let _ = tx.send(None);
                break;
            }
            Err(TryRecvError::Empty) => {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(TryRecvError::Disconnected) => {
                running_clone.store(false, Ordering::SeqCst);
                break;
            }
        }
    });

    let status_label_clone = status_label.clone();

    gtk::glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
        match rx.try_recv() {
            Ok(Some(state)) => {
                let icon_name = match &state {
                    UiState::Idle => "microphone-sensitivity-muted",
                    UiState::Recording { .. } => "media-record",
                    UiState::Transcribing { .. } => "audio-input-microphone",
                    UiState::Done { .. } => "dialog-ok",
                    UiState::Error { .. } => "dialog-error",
                };

                indicator
                    .borrow_mut()
                    .set_icon_full(icon_name, "Voice Type");

                let status_text = match &state {
                    UiState::Idle => "Voice Type - Idle".to_string(),
                    UiState::Recording { .. } => "Recording...".to_string(),
                    UiState::Transcribing { .. } => "Transcribing...".to_string(),
                    UiState::Done { text } => format!("Done: {}", text),
                    UiState::Error { msg } => format!("Error: {}", msg),
                };
                status_label_clone.set_label(&status_text);
            }
            Ok(None) => {
                gtk::main_quit();
                return gtk::glib::ControlFlow::Break;
            }
            Err(_) => {}
        }
        gtk::glib::ControlFlow::Continue
    });

    gtk::main();

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
