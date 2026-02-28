use anyhow::{bail, Result};

pub fn type_text_auto(text: &str) -> Result<()> {
    if text.trim().is_empty() {
        bail!("No text to type (transcription empty)");
    }

    type_with_enigo(text)
}

fn type_with_enigo(text: &str) -> Result<()> {
    use enigo::{Enigo, Key, Keyboard, Settings};

    let mut enigo = Enigo::new(&Settings::default())?;

    enigo.text(text)?;

    Ok(())
}
