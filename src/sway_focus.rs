use anyhow::Result;
use std::process::Command;

pub fn focus_prev() -> Result<()> {
    // Works in sway: focuses previously focused container
    let _ = Command::new("swaymsg").args(["focus", "prev"]).status()?;
    Ok(())
}
