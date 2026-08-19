# Voice Type

A cross-platform voice-to-text application that types what you speak. Hold a hotkey to record, release to transcribe and type.

## Features

- **Cross-platform**: Linux (X11/Wayland) and macOS
- **Native UI**: State-aware system tray icon + recording popup near cursor
- **Cloud ASR (default)**: VolcEngine streaming speech recognition — the same engine and API key as the [`pi-voice-input`](https://github.com/tr-nc/pi-voice-input) pi extension
- **Real-time preview**: the recording popup shows the live transcript as you speak (streaming ASR partials), no more waiting until release
- **Local ASR fallback**: on-device recognition with [faster-whisper](https://github.com/SYSTRAN/faster-whisper) when no cloud key is configured or the cloud fails
- **Smart correction**: Optional post-processing of the transcript with a local LLM (qwen2.5 via [Ollama](https://ollama.com)) to fix homophones, punctuation and filler words
- **Fast feedback**: Raw transcript is typed immediately; the corrected version replaces it in the background once ready
- **Global hotkey**: Hold Alt/Option to record, release to type
- **Hotkey remapping (Linux)**: while running, the hotkey scancode is transparently remapped to `F13` so holding it never pollutes synthetic typing (see [Hotkey remapping](#hotkey-remapping-linux))

## Supported Platforms

| Platform | Hotkey | Typing | Tray | Popup |
|----------|--------|--------|------|-------|
| Linux (Wayland) | Alt | wtype, ydotool | ksni | GTK |
| Linux (X11) | Alt | xdotool, ydotool | ksni | GTK |
| macOS | Option | enigo | NSStatusItem | NSWindow |

## Requirements

### Linux
- `input` group membership for hotkey detection:
  ```bash
  sudo usermod -a -G input $USER
  # Log out and back in for changes to take effect
  ```

- Typing tool (install one):
  ```bash
  # Wayland (recommended)
  sudo pacman -S wtype      # Arch
  sudo apt install wtype    # Debian/Ubuntu

  # X11
  sudo apt install xdotool

  # Universal (works on both)
  sudo pacman -S ydotool    # Arch
  yay -S ydotool
  ```

### macOS
- Accessibility permissions:
  - System Settings → Privacy & Security → Accessibility
  - Add the app to the list

## Dependencies (ASR)

Voice Type supports two ASR backends:

| Provider | Mode | Requirement |
|----------|--------|-------------|
| **VolcEngine** (default) | Cloud streaming, live typing | API key in `~/.pi/agent/voice-input.config.json` |
| faster-whisper | Local, offline | Local ASR server (below) |

### VolcEngine streaming ASR (default, live output)

Voice Type shares its credentials with the [`pi-voice-input`](https://github.com/tr-nc/pi-voice-input) pi extension. If you already use that extension in pi (configured via `/voice key`), there is **nothing to set up** — Voice Type reads the same file:

```text
~/.pi/agent/voice-input.config.json
```

```json
{
  "volcApiKey": "your-key",
  "boostingTableId": ""
}
```

Get a key at <https://console.volcengine.com/speech/new/setting/apikeys?projectName=default>. When the key is present, Voice Type automatically uses the VolcEngine *streaming* endpoint: while you hold the hotkey, the recognized text is typed **live into the focused input box at your cursor** as you speak. When the ASR rewrites earlier words, the divergence is backspaced and retyped automatically. If the cloud call fails, Voice Type falls back to the local whisper server for that utterance.

### faster-whisper ASR server (optional local fallback)

Only needed if you want fully offline recognition (`VT_ASR_PROVIDER=whisper`) or as an automatic fallback when the VolcEngine key is absent/fails.

The server runs in its own isolated Python (3.12) via [`uv`](https://docs.astral.sh/uv/),
so your system Python is not affected. Install `uv`, then from the repo root:

```bash
uv run --python 3.12 \
    --with fastapi --with "uvicorn[standard]" \
    --with "faster-whisper" --with python-multipart \
    uvicorn server.asr_server:app --host 127.0.0.1 --port 8000
```

The first run downloads the ASR model (~1.5 GB for `medium`, cached under
`~/.cache/huggingface`). On a CPU-only machine the default `medium`/`int8`
model is a good speed/accuracy trade-off; see `VT_ASR_MODEL` below to change it.

> The ASR server must be running whenever Voice Type is. Consider wrapping the
> command above in a systemd unit alongside Voice Type (see Autostart).

## Installation

### From Source

```bash
git clone https://github.com/peter209393/voice-type.git
cd voice-type

# Linux (default, ksni tray)
cargo build --release

# Linux (GTK tray - for XFCE, MATE, etc.)
cargo build --release --features gtk-tray

# macOS
cargo build --release
```

## Usage

1. Optionally start the local whisper server (see *Dependencies*) if you want an offline fallback.
2. Run the application:
   ```bash
   ./target/release/voice-type
   ```

3. Focus any input box, hold **Alt** (Linux) or **Option** (macOS) and speak —
   the recognized text is typed **live at your cursor** as you speak
   (VolcEngine streaming)
4. Release the key: the final transcript is completed at the cursor

The tray icon shows the current state: idle → recording → transcribing → done/error.

## Hotkey remapping (Linux)

The push-to-talk hotkey is a **modifier** (Alt). On Wayland, compositors merge
modifier state across *all* keyboards on the seat, so while the hotkey is held,
synthetic keys from `wtype` reach the focused app as `Alt+<key>` — apps treat
those as shortcuts and silently drop the characters. This broke live typing:
partial transcripts "disappeared" until the key was released.

To fix this, on startup Voice Type remaps the hotkey's scancode to `F13`
(via the `EVIOCSKEYCODE_V2` ioctl, no root needed — membership in the `input`
group suffices):

- `F13` is not a modifier, so held-hotkey state never leaks into synthetic typing.
- In standard keymaps `F13` maps to an inert keysym; apps and terminals ignore
  it entirely (unlike e.g. `F23`, which terminals encode as escape sequences).
- The original mapping is **restored automatically** when the app exits,
  including on `SIGINT`/`SIGTERM`.

Practical implications while Voice Type is running:

- The hotkey key (right/left Alt) no longer works as Alt/AltGr. If you need
  AltGr characters, terminate Voice Type (`kill $(pgrep -x voice-type)`)
  and restart it afterwards — the remap is re-applied on launch.
- After a hard kill (`kill -9`) the scancode may stay remapped; terminate the
  app gracefully on next run, replug the keyboard, or reboot to restore.
- Devices whose driver refuses the remap (e.g. some external keyboards) keep
  the original keycode — the listener accepts both, but live typing on those
  setups may still be affected by the held modifier.

## Autostart

### Linux (Sway)
Add to `~/.config/sway/config`:
```
exec /path/to/voice-type
```

### Linux (systemd)
Create `~/.config/systemd/user/voice-type.service`:
```ini
[Unit]
Description=Voice Type
After=graphical-session.target

[Service]
ExecStart=/path/to/voice-type
Restart=on-failure

[Install]
WantedBy=default.target
```

A companion unit for the ASR server (`voice-type-asr.service`) is recommended,
running the `uv run ... uvicorn ...` command from *Dependencies*.

Then:
```bash
systemctl --user enable --now voice-type
```

### macOS
Add to Login Items in System Settings → General → Login Items.

## Configuration

### Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `VT_ASR_PROVIDER` | `auto` | ASR backend: `auto` (VolcEngine when its key is configured, else whisper), `volc` (force VolcEngine), `whisper` (force local) |
| `VT_VOLC_CONFIG` | `~/.pi/agent/voice-input.config.json` | Path to the shared VolcEngine/pi-voice-input config |
| `VT_ASR_URL` | `http://127.0.0.1:8000` | faster-whisper ASR server base URL |
| `VT_ASR_MODEL` | `medium` | faster-whisper model (`tiny`/`base`/`small`/`medium`/`large-v3`) |
| `VT_ASR_DEVICE` | `cpu` | CTranslate2 device (`cpu`/`cuda`) |
| `VT_ASR_COMPUTE_TYPE` | `int8` | CTranslate2 compute type (use `float16` on CUDA) |
| `VT_LOG` | unset | Set to any non-empty value (e.g. `1`) for verbose debug logging: main loop, streaming ASR worker, hotkey remapping, audio device selection |

## Building Features

| Feature | Description | Default |
|---------|-------------|---------|
| `gtk-tray` | Use GTK/libappindicator for tray (XFCE, MATE) | No |

## Troubleshooting

| Problem | Fix |
|---------|-----|
| "No keyboard device found" | Add yourself to the `input` group and re-login |
| "No typing tool found" | Install `wtype`, `ydotool`, or `xdotool` |
| ASR errors / connection refused | Start the faster-whisper server (see *Dependencies*); check `VT_ASR_URL` |
| Live typing garbles with an IME | An IME can interfere with synthetic typing; switch the IME to plain English/direct mode while dictating |
| Right Alt doesn't work as Alt/AltGr | Expected while Voice Type runs (scancode remapped to `F13`); restored on exit — see [Hotkey remapping](#hotkey-remapping-linux) |
| Key still remapped after a crash | A hard kill skips the restore; terminate the app gracefully on next run, replug the keyboard, or reboot |
| Live partials missing in an app | Ensure the VolcEngine key is configured (streaming mode); check `VT_LOG=1` output for `volc: partial` lines |
| No tray icon | Ensure your bar supports StatusNotifierItem (waybar, etc.) |
| macOS: Typing doesn't work | Grant Accessibility permissions in System Settings |

## License

MIT
