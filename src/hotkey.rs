use anyhow::Result;
use crossbeam_channel::{bounded, Receiver, Sender};
use evdev::{Device, EventType, InputEventKind, Key};
use std::thread;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HotkeyEvent {
    Pressed,
    Released,
}

pub fn start_hotkey_listener() -> Result<Receiver<HotkeyEvent>> {
    let (tx, rx) = bounded::<HotkeyEvent>(16);

    let tx_clone = tx.clone();
    thread::spawn(move || {
        if let Err(e) = listen_hotkey(tx_clone) {
            eprintln!("Hotkey listener error: {}", e);
        }
    });

    Ok(rx)
}

fn listen_hotkey(tx: Sender<HotkeyEvent>) -> Result<()> {
    let target_keys = [
        (Key::KEY_RIGHTALT, "Right Alt"),
        (Key::KEY_LEFTALT, "Left Alt"),
    ];

    let mut found_key = None;
    let mut devices: Vec<Device> = Vec::new();

    // First try to find devices that explicitly support the target keys
    for (key, name) in &target_keys {
        let devs: Vec<Device> = evdev::enumerate()
            .map(|(_, d)| d)
            .filter(|d| {
                d.supported_keys()
                    .map(|keys| keys.contains(*key))
                    .unwrap_or(false)
            })
            .collect();

        if !devs.is_empty() {
            eprintln!("[Hotkey] Found {} device(s) with {} key", devs.len(), name);
            devices = devs;
            found_key = Some(*key);
            break;
        }
    }

    // Fallback: use any keyboard device if specific key detection failed
    if devices.is_empty() {
        eprintln!("[Hotkey] Specific key detection failed, trying fallback to any keyboard device");
        let all_keyboards: Vec<Device> = evdev::enumerate()
            .map(|(_, d)| d)
            .filter(|d| {
                // Check if device supports any KEY events (has at least one key)
                d.supported_keys()
                    .map_or(false, |keys| keys.iter().next().is_some())
            })
            .collect();

        if !all_keyboards.is_empty() {
            eprintln!("[Hotkey] Using {} keyboard device(s) as fallback", all_keyboards.len());
            devices = all_keyboards;
            found_key = Some(Key::KEY_RIGHTALT);
        }
    }

    if devices.is_empty() {
        anyhow::bail!(
            "No keyboard device found.\n\
             Please ensure:\n\
             1. You are in the 'input' group: sudo usermod -a -G input $USER\n\
             2. Log out and back in for group changes to take effect\n\
             3. Your keyboard is connected"
        );
    }

    let target_key = found_key.unwrap();
    let mut keyboard_devices = devices;
    let mut is_pressed = false;

    loop {
        for device in &mut keyboard_devices {
            if let Ok(events) = device.fetch_events() {
                for event in events {
                    if event.event_type() == EventType::KEY {
                        if let InputEventKind::Key(key) = event.kind() {
                            if key == target_key {
                                let value = event.value();
                                if value == 1 && !is_pressed {
                                    is_pressed = true;
                                    let _ = tx.try_send(HotkeyEvent::Pressed);
                                } else if value == 0 && is_pressed {
                                    is_pressed = false;
                                    let _ = tx.try_send(HotkeyEvent::Released);
                                }
                            }
                        }
                    }
                }
            }
        }

        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}
