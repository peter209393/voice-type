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
