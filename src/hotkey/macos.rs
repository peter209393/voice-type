use anyhow::Result;
use crossbeam_channel::{bounded, Receiver, Sender};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HotkeyEvent {
    Pressed,
    Released,
}

static ALT_PRESSED: AtomicBool = AtomicBool::new(false);
static CHANNEL_SENDER: std::sync::OnceLock<Sender<HotkeyEvent>> = std::sync::OnceLock::new();

pub fn start_hotkey_listener() -> Result<Receiver<HotkeyEvent>> {
    let (tx, rx) = bounded::<HotkeyEvent>(16);

    let _ = CHANNEL_SENDER.set(tx.clone());

    thread::spawn(move || {
        if let Err(e) = listen_hotkey() {
            eprintln!("Hotkey listener error: {}", e);
        }
    });

    Ok(rx)
}

fn listen_hotkey() -> Result<()> {
    use rdev::{listen, Event, EventType, Key};

    let callback = |event: Event| {
        let tx = match CHANNEL_SENDER.get() {
            Some(tx) => tx,
            None => return,
        };

        match event.event_type {
            EventType::KeyPress(Key::Alt) | EventType::KeyPress(Key::AltGr) => {
                if !ALT_PRESSED.load(Ordering::SeqCst) {
                    ALT_PRESSED.store(true, Ordering::SeqCst);
                    let _ = tx.try_send(HotkeyEvent::Pressed);
                }
            }
            EventType::KeyRelease(Key::Alt) | EventType::KeyRelease(Key::AltGr) => {
                if ALT_PRESSED.load(Ordering::SeqCst) {
                    ALT_PRESSED.store(false, Ordering::SeqCst);
                    let _ = tx.try_send(HotkeyEvent::Released);
                }
            }
            _ => {}
        }
    };

    if let Err(e) = listen(callback) {
        anyhow::bail!("Failed to listen to keyboard events: {:?}", e);
    }

    Ok(())
}
