# Voice Type

A cross-platform voice-to-text application that types what you speak. Hold a hotkey to record, release to transcribe and type.

## Features

- **Cross-platform**: Linux (X11/Wayland) and macOS
- **Native UI**: System tray icon + recording popup near cursor
- **Real-time transcription**: Uses Z.AI API for accurate speech recognition
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

## Installation

### From Source

```bash
git clone https://github.com/yourname/voice-type.git
cd voice-type

# Linux (default, ksni tray)
cargo build --release

# Linux (GTK tray - for XFCE, MATE, etc.)
cargo build --release --features gtk-tray

# macOS
cargo build --release
```

## Usage

1. Set your API token:
   ```bash
   export ZAI_API_TOKEN=your_token_here
   ```

2. Run the application:
   ```bash
   ./target/release/voice-type
   ```

3. Hold **Alt** (Linux) or **Option** (macOS) to start recording
4. Speak your text
5. Release the key to transcribe and type

## Autostart

### Linux (Sway)
Add to `~/.config/sway/config`:
```
exec env ZAI_API_TOKEN=your_key /path/to/voice-type
```

### Linux (systemd)
Create `~/.config/systemd/user/voice-type.service`:
```ini
[Unit]
Description=Voice Type
After=graphical-session.target

[Service]
Environment=ZAI_API_TOKEN=your_token_here
ExecStart=/path/to/voice-type
Restart=on-failure

[Install]
WantedBy=default.target
```

Then:
```bash
systemctl --user enable --now voice-type
```

### macOS
Add to Login Items in System Settings → General → Login Items.

## Configuration

### Environment Variables

| Variable | Description | Required |
|----------|-------------|----------|
| `ZAI_API_TOKEN` | API token from z.ai | Yes |

## Building Features

| Feature | Description | Default |
|---------|-------------|---------|
| `gtk-tray` | Use GTK/libappindicator for tray (XFCE, MATE) | No |

## Troubleshooting

| Problem | Fix |
|---------|-----|
| "No keyboard device found" | Add yourself to the `input` group and re-login |
| "No typing tool found" | Install `wtype`, `ydotool`, or `xdotool` |
| API errors / auth failure | Check that `ZAI_API_TOKEN` is set and valid |
| No tray icon | Ensure your bar supports StatusNotifierItem (waybar, etc.) |
| macOS: Typing doesn't work | Grant Accessibility permissions in System Settings |

## License

MIT
