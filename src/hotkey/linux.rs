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

    fn is_keyboard_device(d: &Device) -> bool {
        let has_keys = d
            .supported_keys()
            .is_some_and(|keys| keys.iter().next().is_some());
        let has_relative = d
            .supported_relative_axes()
            .is_some_and(|axes| axes.iter().next().is_some());
        has_keys && !has_relative
    }

    for (key, _name) in &target_keys {
        let devs: Vec<Device> = evdev::enumerate()
            .map(|(_, d)| d)
            .filter(|d| {
                is_keyboard_device(d)
                    && d.supported_keys()
                        .map(|keys| keys.contains(*key))
                        .unwrap_or(false)
            })
            .collect();

        if !devs.is_empty() {
            devices = devs;
            found_key = Some(*key);
            break;
        }
    }

    if devices.is_empty() {
        let all_keyboards: Vec<Device> = evdev::enumerate()
            .map(|(_, d)| d)
            .filter(is_keyboard_device)
            .collect();

        if !all_keyboards.is_empty() {
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
        for device in keyboard_devices.iter_mut() {
            if let Ok(events) = device.fetch_events() {
                for event in events {
                    if event.event_type() == EventType::KEY {
                        if let InputEventKind::Key(key) = event.kind() {
                            let value = event.value();

                            if key == target_key {
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
