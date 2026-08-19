use anyhow::Result;
use crossbeam_channel::{bounded, Receiver, Sender};
use evdev::{Device, EventType, InputEventKind, Key};
use std::os::unix::io::AsRawFd;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::thread;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HotkeyEvent {
    Pressed,
    Released,
}

/// The push-to-talk hotkey is a MODIFIER key (Alt). The compositor merges
/// modifier state across all keyboards on the seat, so while the user holds
/// it, synthetic text typed by wtype arrives at apps as Alt+<key> — treated
/// as shortcuts and dropped (the "live partials missing" bug).
///
/// Fix: remap the hotkey's scancode to an unused NON-modifier keycode
/// (F13) via EVIOCSKEYCODE_V2 at startup, and restore on exit. F13 maps to
/// a harmless/NoSymbol keysym in the standard keymap (apps and terminals
/// ignore it entirely, unlike F23 which terminals encode as CSI sequences),
/// it is within atkbd's settable keycode range (<= 255), and nothing binds
/// it by default. The user still physically holds the same key; the system
/// just never sees a modifier being held, so wtype text is committed
/// cleanly.
const HOTKEY_KEYCODE: Key = Key::KEY_F13;

/// Records how to restore one remapped scancode.
struct RestoreEntry {
    /// Device path for reopening on normal shutdown.
    path: PathBuf,
    /// The same path as CString for the signal handler (no allocation
    /// is allowed there).
    cpath: std::ffi::CString,
    /// Serialized `struct input_keymap_entry` that maps the scancode back
    /// to its original keycode.
    entry: [u8; 40],
}

/// Populated once at startup, before the signal handler is installed, and
/// never mutated afterwards (the signal handler reads it lock-free).
static RESTORES: OnceLock<Vec<RestoreEntry>> = OnceLock::new();
/// Guards the initialization (single writer); `RESTORES` itself is frozen
/// once set.
static RESTORES_INIT: Mutex<()> = Mutex::new(());

/// EVIOCSKEYCODE_V2 = _IOW('E', 0x04, struct input_keymap_entry) on
/// Linux asm-generic ABIs (x86_64, aarch64, ...): size 40, dir WRITE.
const EVIOCSKEYCODE_V2: libc::c_ulong = 0x4028_4504;

/// Serialize `struct input_keymap_entry { u8 flags; u8 len; u16 index;
/// u32 keycode; u8 scancode[32]; }` (repr(C), natural alignment).
fn keymap_entry_bytes(keycode: u32, scancode: &[u8]) -> [u8; 40] {
    let mut e = [0u8; 40];
    e[1] = scancode.len() as u8; // len
    // index stays 0 (unused; flags = 0 means "by scancode")
    e[4..8].copy_from_slice(&keycode.to_ne_bytes());
    e[8..8 + scancode.len()].copy_from_slice(scancode);
    e
}

/// Signal-safe restore: raw syscalls only (open/ioctl/close/_exit).
extern "C" fn restore_on_signal(_sig: libc::c_int) {
    unsafe {
        if let Some(restores) = RESTORES.get() {
            for r in restores {
                let fd = libc::open(r.cpath.as_ptr(), libc::O_WRONLY | libc::O_NONBLOCK);
                if fd >= 0 {
                    let mut entry = r.entry;
                    libc::ioctl(fd, EVIOCSKEYCODE_V2, entry.as_mut_ptr() as *mut libc::c_void);
                    libc::close(fd);
                }
            }
        }
        libc::_exit(0);
    }
}

/// Restore all remapped scancodes to their original keycodes. Called on
/// normal shutdown (and from the SIGINT/SIGTERM handler above).
pub fn shutdown() {
    if let Some(restores) = RESTORES.get() {
        for r in restores {
            if let Ok(dev) = Device::open(&r.path) {
                let keycode = u32::from_ne_bytes(r.entry[4..8].try_into().unwrap());
                let len = r.entry[1] as usize;
                let sc = &r.entry[8..8 + len];
                let key = Key::new(keycode as u16);
                if let Err(e) = dev.update_scancode(key, sc) {
                    eprintln!("[vt] failed to restore {:?}: {e}", r.path);
                }
            }
        }
    }
}

/// Find the scancodes that currently produce `keycode` on `dev`.
fn scancodes_for(dev: &Device, key: Key) -> Vec<Vec<u8>> {
    // Fast path: reverse lookup.
    if let Ok(sc) = dev.get_scancode_by_keycode(key) {
        return vec![sc];
    }
    // Fallback: scan the index-based keymap table (some drivers, e.g.
    // atkbd, do not support direct reverse lookup).
    let mut found = Vec::new();
    for idx in 0..1024u16 {
        if let Ok((kc, sc)) = dev.get_scancode_by_index(idx) {
            if kc == u32::from(key.code()) {
                found.push(sc);
            }
        }
    }
    found
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
    let mut devices: Vec<(PathBuf, Device)> = Vec::new();

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
        let devs: Vec<(PathBuf, Device)> = evdev::enumerate()
            .map(|(p, d)| (p.to_path_buf(), d))
            .filter(|(_, d)| {
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
        let all_keyboards: Vec<(PathBuf, Device)> = evdev::enumerate()
            .map(|(p, d)| (p.to_path_buf(), d))
            .filter(|(_, d)| is_keyboard_device(d))
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

    // Remap the hotkey to a non-modifier keycode so holding it does not
    // pollute synthetic typing with a seat-wide modifier (see HOTKEY_KEYCODE
    // docs). Devices whose driver refuses the remap (e.g. uinput test
    // injectors) keep emitting the original keycode — we listen for both.
    let mut restores: Vec<RestoreEntry> = Vec::new();
    for (path, dev) in &devices {
        for sc in scancodes_for(dev, target_key) {
            if dev.update_scancode(HOTKEY_KEYCODE, &sc).is_ok() {
                let Ok(cpath) = std::ffi::CString::new(path.as_os_str().as_encoded_bytes())
                else {
                    continue;
                };
                restores.push(RestoreEntry {
                    path: path.clone(),
                    cpath,
                    entry: keymap_entry_bytes(u32::from(target_key.code()), &sc),
                });
            }
        }
    }
    if !restores.is_empty() {
        if std::env::var_os("VT_LOG").is_some_and(|v| !v.is_empty()) {
            for r in &restores {
                eprintln!(
                    "[vt/hotkey] remapped hotkey scancode on {:?} -> F13",
                    r.path
                );
            }
        }
        let _guard = RESTORES_INIT.lock().unwrap();
        let _ = RESTORES.set(restores);
        // Restore the key mapping even when killed, so the physical key
        // cannot stay remapped after a crash of the session.
        unsafe {
            libc::signal(libc::SIGINT, restore_on_signal as extern "C" fn(libc::c_int) as usize as libc::sighandler_t);
            libc::signal(libc::SIGTERM, restore_on_signal as extern "C" fn(libc::c_int) as usize as libc::sighandler_t);
        }
    }

    let mut keyboard_devices = devices;
    let mut is_pressed = false;

    // evdev opens devices with BLOCKING fds: a `fetch_events` on a device
    // with no pending events would park this thread until *that* device
    // fires, silently swallowing hotkey events arriving on the other
    // monitored devices (press/release went missing sporadically because of
    // this). Switch every fd to non-blocking so the polling loop below
    // actually polls.
    for (_, device) in keyboard_devices.iter_mut() {
        let fd = device.as_raw_fd();
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags >= 0 {
            unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) };
        }
    }

    loop {
        for (_, device) in keyboard_devices.iter_mut() {
            // Collect first, emit after: fetch_events holds a mutable borrow
            // of the device for the iterator's lifetime.
            let mut presses: Vec<i32> = Vec::new();
            if let Ok(events) = device.fetch_events() {
                for event in events {
                    if event.event_type() == EventType::KEY {
                        if let InputEventKind::Key(key) = event.kind() {
                            if key == HOTKEY_KEYCODE || key == target_key {
                                presses.push(event.value());
                            }
                        }
                    }
                }
            }
            for value in presses {
                if value == 1 && !is_pressed {
                    is_pressed = true;
                    let _ = tx.try_send(HotkeyEvent::Pressed);
                } else if value == 0 && is_pressed {
                    is_pressed = false;
                    let _ = tx.try_send(HotkeyEvent::Released);
                }
            }
        }

        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Probe EVIOCSKEYCODE support on real keyboards (harmless, reverts).
    /// cargo test --bin voice-type probe_scancode -- --ignored --nocapture
    #[test]
    #[ignore]
    fn probe_scancode_remap_support() {
        for (path, dev) in evdev::enumerate() {
            let Some(name) = dev.name() else { continue };
            let has_alt = dev
                .supported_keys()
                .is_some_and(|k| k.contains(Key::KEY_RIGHTALT));
            if !has_alt {
                continue;
            }
            let found = scancodes_for(&dev, Key::KEY_RIGHTALT);
            println!("{} ({}): scancodes={:02x?}", path.display(), name, found);
            for sc in found.iter() {
                match dev.update_scancode(HOTKEY_KEYCODE, sc.as_slice()) {
                    Ok(old) => {
                        println!("  remap -> F13 ok (old={:?})", old);
                        let back = dev.update_scancode(Key::KEY_RIGHTALT, sc.as_slice());
                        println!("  reverted: {}", back.is_ok());
                    }
                    Err(e) => println!("  remap FAILED: {e}"),
                }
            }
        }
    }
}
