# Voice Type

> 🎙️ **Hold-to-talk voice typing for Linux & macOS** — hold a key, speak, and the text appears **live at your cursor** as you talk. Real-time streaming speech-to-text in a tiny Rust tray daemon.

No dictation windows, no clicking — Voice Type turns any focused input box (browser, chat, editor, terminal) into a voice input box. Works on **Wayland** (sway, Hyprland, GNOME, KDE), **X11**, and **macOS**.

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](#license)
![Platforms](https://img.shields.io/badge/platform-Linux%20%7C%20macOS-informational)
![Rust](https://img.shields.io/badge/rust-%F0%9F%A6%80-orange)
![ASR](https://img.shields.io/badge/ASR-VolcEngine%20streaming%20%7C%20faster--whisper%20local-green)

## Features

- 🗣️ **Real-time voice typing** — streaming ASR partials are typed live while you speak; no waiting for release
- ✨ **Fast LLM correction** — after release, a non-thinking Doubao model (VolcEngine Ark) fixes homophones, punctuation and filler words, then quietly replaces the text
- ⌨️ **Push-to-talk hotkey** — hold **Alt** / **Option** to record, release to finish
- 🔁 **Self-correcting** — when the recognizer rewrites earlier words, the divergence is backspaced and retyped
- ☁️ **Cloud ASR by default** — VolcEngine streaming (same engine & key as the [`pi-voice-input`](https://github.com/tr-nc/pi-voice-input) extension), with punctuation & ITN
- 🏠 **Offline fallback** — local [faster-whisper](https://github.com/SYSTRAN/faster-whisper) server when the cloud is unavailable
- 🎨 **State-aware tray icon** — procedurally rendered, independent of the icon theme
- 🖥️ **Cross-platform, single binary** — Linux (Wayland/X11) & macOS, no Electron

## Install

### One-click via AI agent

Paste [`agent-install.md`](agent-install.md) into your coding agent (pi, Claude Code, Cursor, …) on the target machine — it handles prerequisites, build, API key, autostart and verification:

```text
Install and set up Voice Type (https://github.com/peter209393/voice-type) …
→ full prompt: agent-install.md in the repo root
```

### Manual

```bash
sudo usermod -a -G input $USER && re-login   # Linux: hotkey access
sudo pacman -S wtype                         # or: sudo apt install wtype

git clone https://github.com/peter209393/voice-type.git
cd voice-type && cargo build --release
export VT_VOLC_API_KEY="your-key"            # or see Configuration
./target/release/voice-type
```

Get a VolcEngine key at <https://console.volcengine.com/speech/new/setting/apikeys?projectName=default>, or go fully offline with the [faster-whisper server](#local-offline-fallback-faster-whisper).

## Usage

Focus any input box, **hold Alt and speak** — text appears live at the cursor (Chinese/English mixed input works well). Release to finalize. The tray icon shows: idle → recording → transcribing → done/error.

## How It Works

Audio is streamed to the ASR engine in 100 ms packets while you hold the hotkey. Each partial transcript is diffed against what is already on screen; only the delta is typed (with backspaces on rewrites). On release, the final transcript completes the utterance.

<details>
<summary><b>Hotkey remapping (Linux)</b> — why Alt becomes F13</summary>

The hotkey is a **modifier** (Alt). Wayland compositors merge modifier state across all keyboards on the seat, so while Alt is held, synthetic keys from `wtype` reach apps as `Alt+<key>` — treated as shortcuts and dropped, which broke live typing. On startup Voice Type remaps the hotkey's scancode to `F13` via `EVIOCSKEYCODE_V2` (no root; `input` group suffices): F13 is a non-modifier with an inert keysym that apps and terminals ignore entirely. The original mapping is restored on exit, including `SIGINT`/`SIGTERM`.

While running: the hotkey key temporarily loses its Alt/AltGr function (restored on exit). After a `kill -9`, restore by terminating the app gracefully on next run, replugging the keyboard, or rebooting. Devices whose driver refuses the remap fall back to the original keycode.
</details>

<details>
<summary><b>Local offline fallback (faster-whisper)</b></summary>

Run an isolated Python 3.12 server via [`uv`](https://docs.astral.sh/uv/), then set `VT_ASR_PROVIDER=whisper`:

```bash
uv run --python 3.12 \
    --with fastapi --with "uvicorn[standard]" \
    --with "faster-whisper" --with python-multipart \
    uvicorn server.asr_server:app --host 127.0.0.1 --port 8000
```

First run downloads the model (~1.5 GB for `medium`, cached in `~/.cache/huggingface`). Use a systemd user unit to keep it running (see Autostart).
</details>

| Platform | Hotkey | Typing | Tray |
|----------|--------|--------|------|
| Linux (Wayland) | Alt | wtype, ydotool | ksni (waybar etc.) |
| Linux (X11) | Alt | xdotool, ydotool | ksni / GTK (`gtk-tray` feature) |
| macOS | Option | enigo | NSStatusItem |

## Configuration

| Variable | Default | Description |
|----------|---------|-------------|
| `VT_ASR_PROVIDER` | `auto` | `auto` (VolcEngine when keyed, else whisper), `volc`, `whisper` |
| `VT_VOLC_API_KEY` | unset | VolcEngine key via env var — takes precedence over any config file |
| `VT_VOLC_BOOSTING_TABLE_ID` | unset | VolcEngine hotwords table id |
| `VT_ARK_API_KEY` | unset | VolcEngine Ark (ByteDance LLM) key — enables fast post-release transcript correction (homophones, punctuation, fillers) |
| `VT_CORRECT_MODEL` | `doubao-seed-2-1-turbo-260628` | Ark model for correction — use a fast *non-thinking* model (thinking is auto-disabled for seed/1.6 models) |
| `VT_CORRECT` | `1` | Set `0`/`off` to disable correction even when a key exists |
| `VT_VOLC_CONFIG` | `~/.pi/agent/voice-input.config.json` | Shared pi-voice-input config file |
| `VT_ASR_URL` | `http://127.0.0.1:8000` | faster-whisper server URL |
| `VT_ASR_MODEL` | `medium` | faster-whisper model (`tiny`…`large-v3`) |
| `VT_ASR_DEVICE` / `VT_ASR_COMPUTE_TYPE` | `cpu` / `int8` | CTranslate2 device / compute type |
| `VT_LOG` | unset | Any non-empty value = verbose debug logging |

## Autostart

**sway** — `~/.config/sway/config`:

```
exec VT_VOLC_API_KEY=your-key ~/.local/bin/voice-type &
```

**systemd** — `~/.config/systemd/user/voice-type.service`:

```ini
[Unit]
Description=Voice Type — hold-to-talk voice typing
After=graphical-session.target
[Service]
Environment=VT_VOLC_API_KEY=your-key
ExecStart=%h/.local/bin/voice-type
Restart=on-failure
[Install]
WantedBy=default.target
```

```bash
systemctl --user enable --now voice-type
```

**macOS** — add to Login Items (System Settings → General).

## Troubleshooting

| Problem | Fix |
|---------|-----|
| "No keyboard device found" | Add yourself to the `input` group and re-login |
| "No typing tool found" | Install `wtype`, `ydotool`, or `xdotool` |
| ASR errors / connection refused | Start the faster-whisper server; check `VT_ASR_URL` |
| Live typing garbles with an IME | Switch the IME to direct/English mode while dictating |
| Right Alt doesn't work as Alt/AltGr | Expected while running (remapped to F13); restored on exit |
| Key still remapped after a crash | Graceful-kill the app on next run, replug keyboard, or reboot |
| Live partials missing | Check the VolcEngine key; debug with `VT_LOG=1` |
| No tray icon | Bar must support StatusNotifierItem (waybar etc.) |
| macOS: typing doesn't work | Grant Accessibility permissions |

## License

MIT
