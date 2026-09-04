# agent-install.md — one-click install prompt for AI agents

Copy the block below and paste it into your coding agent (pi, Claude Code,
Cursor, Codex CLI, …) on the target machine. It installs Voice Type end-to-end:
prerequisites → build → API key → autostart → live verification.

---

Install and set up **Voice Type** (https://github.com/peter209393/voice-type) on this machine so it is built, running, and auto-started at login. It is a Rust tray daemon for voice typing: hold **Alt** (Option on macOS), speak, and the recognized text is typed live at the cursor. Target OS: Linux (Wayland preferred) or macOS.

Do the following:

1. **Detect** the OS and package manager (pacman / apt / dnf / brew).
2. **Prerequisites**:
   - Rust toolchain via rustup if `cargo` is missing.
   - Linux only: install a typing tool — `wtype` on Wayland (`pacman -S wtype` / `apt install wtype`), `xdotool` or `ydotool` on X11.
   - Linux only: ensure my user is in the `input` group (`sudo usermod -aG input $USER`); if you just added it, remind me that a re-login is needed for the hotkey to work.
3. **Clone & build**: clone https://github.com/peter209393/voice-type.git (keep an existing checkout if one is present) and run `cargo build --release`. Install the binary to `~/.local/bin/voice-type`.
4. **ASR backend** — ask me which one, then configure it:
   - a. *VolcEngine cloud (recommended, live streaming text)*: I will give you an API key; wire it as the `VT_VOLC_API_KEY` environment variable wherever you configure autostart.
   - b. *Fully offline*: run the local faster-whisper server with `uv` exactly as described in the repo README section "Local offline fallback (faster-whisper)", set `VT_ASR_PROVIDER=whisper`, and set up a systemd user unit for the server too.
5. **Autostart**: if sway is in use, add `exec VT_VOLC_API_KEY=<key> ~/.local/bin/voice-type &` to `~/.config/sway/config`; otherwise create a systemd **user** unit `~/.config/systemd/user/voice-type.service` (with `Environment=VT_VOLC_API_KEY=...`) and enable it.
6. **Start it now** (no re-login needed if the `input` group membership was already active).
7. **Verify**:
   - the process is running and a tray icon is visible;
   - then ask me to focus any text field, hold **right Alt**, and speak a short sentence — the text must appear live at the cursor while I speak, and complete after I release;
   - if nothing types, debug with `VT_LOG=1 voice-type` and report the relevant log lines.

Notes:
- No root needed except the `usermod` (and package installs).
- Do not git commit or push anything; only modify this machine's local config.
