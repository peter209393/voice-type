# sway-voice-type

**TLDR: Built because there's no good open-source, hands-free voice typing tool for Sway/Wayland — no dictation daemon, no Dragon, no Google Docs. Just hold Alt, speak, and your words appear.**

A lightweight, open-source push-to-talk voice dictation tool for Sway and Wayland. Hold Alt to record, release to transcribe via the [z.ai](https://z.ai) ASR API, and have the result typed at your cursor — no GUI, no clipboard, no paste.

## Features

- Push-to-talk with Right/Left Alt key — no always-on microphone
- Streaming transcription every 0.5s while recording for low latency
- Auto-types via `wtype` (preferred) or `ydotool` fallback — works in any app
- System tray icon showing live state (idle / recording / transcribing / error)
- Tiny binary, zero configuration files

## Requirements

- Sway or any Wayland compositor
- `wtype` or `ydotool` installed and in `PATH`
- A [z.ai](https://z.ai) API key (`ZAI_API_TOKEN`)
- Membership in the `input` group (for keyboard event access via evdev)

## Setup

### 1. Add yourself to the `input` group

```sh
sudo usermod -a -G input $USER
# Log out and back in for this to take effect
```

### 2. Install a typing tool

```sh
# Arch Linux
sudo pacman -S wtype
# or
sudo pacman -S ydotool
```

### 3. Set your API key

```sh
export ZAI_API_TOKEN=your_key_here
```

Add to your shell profile (`~/.bashrc`, `~/.zshrc`, `~/.config/fish/config.fish`) to persist across sessions.

## Build & Run

```sh
cargo build --release
./target/release/sway-voice-type
```

Or run directly:

```sh
ZAI_API_TOKEN=your_key cargo run --release
```

## Usage

1. Run the binary — a tray icon appears (muted mic = idle)
2. **Hold Alt** to start recording (icon changes to record indicator)
3. **Speak** — partial transcriptions are typed in real time every 0.5s
4. **Release Alt** — final chunk is transcribed, typed, and the tool returns to idle

The typed text appears wherever your cursor is focused — terminal, browser, editor, chat app, anything.

## Autostart with Sway

Add to `~/.config/sway/config`:

```
exec env ZAI_API_TOKEN=your_key $HOME/.cargo/bin/sway-voice-type
```

## Troubleshooting

| Problem | Fix |
|---|---|
| "No keyboard device found" | Add yourself to the `input` group and re-login |
| "Neither wtype nor ydotool found" | Install `wtype` or `ydotool` and ensure they are in `PATH` |
| API errors / auth failure | Check that `ZAI_API_TOKEN` is set and valid |
| No tray icon | Ensure your bar supports StatusNotifierItem (e.g. waybar with `tray` module enabled) |
| Text typed twice | Only run one instance of `sway-voice-type` at a time |

## License

MIT
