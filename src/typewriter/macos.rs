use anyhow::{bail, Result};

pub fn type_text_auto(text: &str) -> Result<()> {
    if text.trim().is_empty() {
        bail!("No text to type (transcription empty)");
    }

    type_with_enigo(text)
}

/// Press Backspace `n` times.
pub fn backspace(n: usize) -> Result<()> {
    if n == 0 {
        return Ok(());
    }

    use enigo::{Enigo, Key, Keyboard, Settings};

    let mut enigo = Enigo::new(&Settings::default())?;
    for _ in 0..n {
        enigo.key(Key::Backspace, enigo::Direction::Press)?;
        enigo.key(Key::Backspace, enigo::Direction::Release)?;
    }

    Ok(())
}

fn type_with_enigo(text: &str) -> Result<()> {
    use enigo::{Enigo, Keyboard, Settings};

    let mut enigo = Enigo::new(&Settings::default())?;

    enigo.text(text)?;

    Ok(())
}
