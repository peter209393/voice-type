use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use crossbeam_channel::{Receiver, TryRecvError};
use gtk::prelude::*;

use crate::tray::icons::{icon_for_state, IconData, ICON_SIZE};
use crate::UiState;

pub enum TrayCmd {
    UpdateState(UiState),
    Quit,
}

/// Icon name registered in our private theme path for each UI state.
fn state_icon_name(state: &UiState) -> &'static str {
    match state {
        UiState::Idle => "voice-type-idle",
        UiState::Recording { .. } => "voice-type-recording",
        UiState::Transcribing { .. } => "voice-type-transcribing",
        UiState::Done { .. } => "voice-type-done",
        UiState::Error { .. } => "voice-type-error",
    }
}

/// Write an RGBA buffer to `<dir>/<W x H>/apps/<name>.png` (hicolor layout).
fn write_icon_png(dir: &Path, name: &str, icon: &IconData) -> anyhow::Result<()> {
    let sub = dir
        .join(format!("{}x{}", ICON_SIZE, ICON_SIZE))
        .join("apps");
    std::fs::create_dir_all(&sub)?;
    let path = sub.join(format!("{}.png", name));

    let bytes = gtk::glib::Bytes::from_owned(icon.rgba.clone());
    let pixbuf = gtk::gdk_pixbuf::Pixbuf::from_bytes(
        &bytes,
        gtk::gdk_pixbuf::Colorspace::Rgb,
        true,
        8,
        icon.width,
        icon.height,
        icon.width * 4,
    );
    pixbuf
        .savev(&path, "png", &[])
        .map_err(|e| anyhow::anyhow!("failed to write {}: {}", path.display(), e))?;
    Ok(())
}

/// Render every state's icon to PNG under a per-process private theme path,
/// so the tray icon does not depend on the active system icon theme.
fn install_icon_theme() -> anyhow::Result<PathBuf> {
    let dir = std::env::temp_dir().join(format!("voice-type-icons-{}", std::process::id()));
    // Fresh dir each launch: remove any stale content from a crashed prior run.
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;

    let states: [(UiState, &str); 5] = [
        (UiState::Idle, "voice-type-idle"),
        (
            UiState::Recording {
                started_at: Instant::now(),
            },
            "voice-type-recording",
        ),
        (
            UiState::Transcribing {
                started_at: Instant::now(),
            },
            "voice-type-transcribing",
        ),
        (
            UiState::Done {
                text: String::new(),
            },
            "voice-type-done",
        ),
        (UiState::Error { msg: String::new() }, "voice-type-error"),
    ];
    for (state, name) in &states {
        let icon = icon_for_state(state);
        write_icon_png(&dir, name, &icon)?;
    }
    Ok(dir)
}

pub fn run_tray(running: Arc<AtomicBool>, cmd_rx: Receiver<TrayCmd>) -> anyhow::Result<()> {
    use libappindicator::AppIndicator;

    // The popup module is gone; this feature now owns GTK initialization.
    gtk::init().map_err(|e| anyhow::anyhow!("Failed to init GTK: {e}"))?;

    let theme_path = install_icon_theme()?;
    let theme_path_str = theme_path
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("non-UTF-8 icon temp path"))?
        .to_string();

    let indicator = RefCell::new(AppIndicator::with_path(
        "Voice Type",
        "voice-type-idle",
        &theme_path_str,
    ));

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
                indicator
                    .borrow_mut()
                    .set_icon_full(state_icon_name(&state), "Voice Type");

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

    let _ = std::fs::remove_dir_all(&theme_path);
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
