use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;
use uuid::Uuid;

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

    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    if text.trim().is_empty() {
        text = String::from_utf8_lossy(&out.stderr).to_string();
    }

    let cleaned = text
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n");

    Ok(cleaned)
}

pub struct StreamTranscriber {
    whisper_bin: String,
    model_path: PathBuf,
    temp_dir: PathBuf,
}

impl StreamTranscriber {
    pub fn new(whisper_bin: &str, model_path: &Path) -> Self {
        Self {
            whisper_bin: whisper_bin.to_string(),
            model_path: model_path.to_path_buf(),
            temp_dir: std::env::temp_dir(),
        }
    }

    pub fn transcribe_chunk(&self, samples: &[f32], sample_rate: u32) -> Result<String> {
        if samples.is_empty() {
            return Ok(String::new());
        }

        let chunk_id = Uuid::new_v4();
        let wav_path = self.temp_dir.join(format!("voice-chunk-{}.wav", chunk_id));

        crate::audio::write_wav_f32_mono(&wav_path, sample_rate, samples)
            .with_context(|| format!("Failed to write wav to {}", wav_path.display()))?;

        let result = transcribe_with_whisper_cli(&self.whisper_bin, &self.model_path, &wav_path);

        let _ = std::fs::remove_file(&wav_path);

        result
    }
}
