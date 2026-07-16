# Voice Type

A cross-platform voice-to-text application that types what you speak. Hold a hotkey to record, release to transcribe and type.

## Features

- **Cross-platform**: Linux (X11/Wayland) and macOS
- **Native UI**: State-aware system tray icon + recording popup near cursor
- **Local ASR**: On-device speech recognition with [faster-whisper](https://github.com/SYSTRAN/faster-whisper) (no cloud, fully offline)
- **Smart correction**: Optional post-processing of the transcript with a local LLM (qwen2.5 via [Ollama](https://ollama.com)) to fix homophones, punctuation and filler words
- **Fast feedback**: Raw transcript is typed immediately; the corrected version replaces it in the background once ready
- **Global hotkey**: Hold Alt/Option to record, release to type

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

## Dependencies (ASR + Correction)

Voice Type runs two local services. Install them once:

### 1. Ollama + qwen2.5 (transcript correction)

```bash
curl -fsSL https://ollama.com/install.sh | sh
ollama pull qwen2.5:3b-instruct   # fast on CPU; use qwen2.5:7b-instruct for quality
```

The installer usually registers an `ollama` systemd service that auto-starts.
Verify it is listening on port 11434 (`curl http://127.0.0.1:11434/api/tags`);
if not, start it manually with `ollama serve` (or `systemctl start ollama`).

> Prefer **non-thinking** instruct models here (e.g. qwen2.5). Thinking models
> like qwen3 ignore the "output only the corrected text" instruction and emit
> long chains of thought, which is both slow on CPU and pollutes the output.

### 2. faster-whisper ASR server

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

1. Start the ASR server (see *Dependencies*) and ensure Ollama is running.
2. Run the application:
   ```bash
   ./target/release/voice-type
   ```

3. Hold **Alt** (Linux) or **Option** (macOS) to start recording
4. Speak your text
5. Release the key: the raw transcript is typed immediately, then replaced
   seconds later by the corrected version

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
| `VT_ASR_URL` | `http://127.0.0.1:8000` | faster-whisper ASR server base URL |
| `VT_ASR_MODEL` | `medium` | faster-whisper model (`tiny`/`base`/`small`/`medium`/`large-v3`) |
| `VT_ASR_DEVICE` | `cpu` | CTranslate2 device (`cpu`/`cuda`) |
| `VT_ASR_COMPUTE_TYPE` | `int8` | CTranslate2 compute type (use `float16` on CUDA) |
| `VT_CORRECT` | `1` | Enable (`1`) / disable (`0`) LLM correction |
| `VT_CORRECTOR_URL` | `http://127.0.0.1:11434` | Ollama base URL |
| `VT_CORRECTOR_MODEL` | `qwen2.5:3b-instruct` | Correction model (e.g. `qwen2.5:7b-instruct` for quality). Avoid thinking models like qwen3 — they ignore "output only" and are slow on CPU |

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
| "Corrector request failed" | Ensure Ollama is running and the model is pulled (`ollama pull qwen2.5:3b-instruct`); or set `VT_CORRECT=0` |
| Correction is too slow | Use a smaller model or `VT_CORRECT=0` |
| Text gets replaced wrongly | An IME can break backspace-based replace; disable correction (`VT_CORRECT=0`) or the IME while typing |
| No tray icon | Ensure your bar supports StatusNotifierItem (waybar, etc.) |
| macOS: Typing doesn't work | Grant Accessibility permissions in System Settings |

## License

MIT
