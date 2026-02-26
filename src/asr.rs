use anyhow::{bail, Context, Result};
use std::path::Path;
use std::process::Command;

pub fn transcribe_with_whisper_cli(
    whisper_bin: &str,
    model_path: &Path,
    wav_path: &Path,
) -> Result<String> {
    let out = Command::new(whisper_bin)
        .args([
            "-m",
            model_path.to_string_lossy().as_ref(),
            "-f",
            wav_path.to_string_lossy().as_ref(),
            "-l",
            "auto",
            "--no-timestamps",
        ])
        .output()
        .with_context(|| format!("Failed to run {}", whisper_bin))?;

    if !out.status.success() {
        bail!(
            "whisper-cli failed (code={:?}). stderr:\n{}",
            out.status.code(),
            String::from_utf8_lossy(&out.stderr)
        );
    }

    // Some builds print progress to stderr; take stdout primarily
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    if text.trim().is_empty() {
        // fallback: some versions output to stderr
        text = String::from_utf8_lossy(&out.stderr).to_string();
    }

    // Basic cleanup: remove empty lines and extra whitespace
    let cleaned = text
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n");

    Ok(cleaned)
}
