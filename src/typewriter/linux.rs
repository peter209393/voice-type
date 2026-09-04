use anyhow::{bail, Context, Result};
use std::process::Command;
use which::which;

pub fn type_text_auto(text: &str) -> Result<()> {
    if text.trim().is_empty() {
        bail!("No text to type (transcription empty)");
    }

    if which("wtype").is_ok() {
        return type_with_wtype(text);
    }

    if which("ydotool").is_ok() {
        return type_with_ydotool(text);
    }

    if which("xdotool").is_ok() {
        return type_with_xdotool(text);
    }

    bail!("No typing tool found. Install one of: wtype, ydotool, or xdotool");
}

/// Press Backspace `n` times. Each backspace deletes one committed character
/// (works for CJK text typed as a unit by these tools).
pub fn backspace(n: usize) -> Result<()> {
    if n == 0 {
        return Ok(());
    }

    if which("wtype").is_ok() {
        eprintln!("[vt] backspace via wtype n={}", n);
        return backspace_with_wtype(n);
    }

    if which("ydotool").is_ok() {
        eprintln!("[vt] backspace via ydotool n={}", n);
        return backspace_with_ydotool(n);
    }

    if which("xdotool").is_ok() {
        eprintln!("[vt] backspace via xdotool n={}", n);
        return backspace_with_xdotool(n);
    }

    bail!("No typing tool found. Install one of: wtype, ydotool, or xdotool");
}

fn type_with_wtype(text: &str) -> Result<()> {
    let status = Command::new("wtype")
        .arg(text)
        .status()
        .context("Failed to execute wtype")?;

    if !status.success() {
        bail!("wtype exited with {:?}", status.code());
    }
    Ok(())
}

fn type_with_ydotool(text: &str) -> Result<()> {
    let status = Command::new("ydotool")
        .args(["type", "--", text])
        .status()
        .context("Failed to execute ydotool")?;

    if !status.success() {
        bail!("ydotool exited with {:?}", status.code());
    }
    Ok(())
}

fn type_with_xdotool(text: &str) -> Result<()> {
    let status = Command::new("xdotool")
        .args(["type", "--clearmodifiers", "--", text])
        .status()
        .context("Failed to execute xdotool")?;

    if !status.success() {
        bail!("xdotool exited with {:?}", status.code());
    }
    Ok(())
}

fn backspace_with_wtype(n: usize) -> Result<()> {
    // wtype has no --repeat; send each backspace as a key event. Use the
    // canonical XKB keysym "BackSpace" and a small inter-key delay so the
    // compositor reliably processes every event.
    let mut args: Vec<String> = Vec::with_capacity(n * 2 + 2);
    args.push("-d".to_string());
    args.push("20".to_string());
    for _ in 0..n {
        args.push("-k".to_string());
        args.push("BackSpace".to_string());
    }
    let status = Command::new("wtype")
        .args(&args)
        .status()
        .context("Failed to execute wtype (backspace)")?;
    if !status.success() {
        bail!("wtype backspace exited with {:?}", status.code());
    }
    Ok(())
}

fn backspace_with_ydotool(n: usize) -> Result<()> {
    // ydotool key uses Linux input-event keycodes; Backspace = 14.
    // `--repeat N` repeats the whole key sequence N times, so we pass one key.
    let status = Command::new("ydotool")
        .args(["key", "--repeat", &n.to_string(), "14:1", "14:0"])
        .status()
        .context("Failed to execute ydotool (backspace)")?;
    if !status.success() {
        bail!("ydotool backspace exited with {:?}", status.code());
    }
    Ok(())
}

fn backspace_with_xdotool(n: usize) -> Result<()> {
    let status = Command::new("xdotool")
        .args([
            "key",
            "--repeat",
            &n.to_string(),
            "--clearmodifiers",
            "BackSpace",
        ])
        .status()
        .context("Failed to execute xdotool (backspace)")?;
    if !status.success() {
        bail!("xdotool backspace exited with {:?}", status.code());
    }
    Ok(())
}
