use anyhow::{Context, Result};
use reqwest::multipart::{Form, Part};
use std::io::Cursor;

/// Transcribes PCM audio via the local faster-whisper HTTP server.
///
/// The server is expected to expose an OpenAI-compatible endpoint:
///   POST {asr_url}/v1/audio/transcriptions  (multipart `file`)
/// returning `{"text": "..."}`.
pub struct StreamTranscriber {
    asr_url: String,
    client: reqwest::Client,
    /// Persistent runtime so the connection pool bound to it stays valid
    /// across transcription calls. Recreating a runtime per call would
    /// orphan the pooled TLS connections (bound to the dropped runtime's
    /// reactor) and break every request after the first.
    runtime: tokio::runtime::Runtime,
}

impl StreamTranscriber {
    pub fn new(_whisper_bin: &str, _model_path: &std::path::Path) -> Self {
        let asr_url = std::env::var("VT_ASR_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:8000".to_string());

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("Failed to build tokio runtime");

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .expect("Failed to build HTTP client");

        Self {
            asr_url,
            client,
            runtime,
        }
    }

    pub fn transcribe_chunk(&self, samples: &[f32], sample_rate: u32) -> Result<String> {
        if samples.is_empty() {
            return Ok(String::new());
        }

        // Always drive the request from the same persistent runtime so the
        // client's pooled connections keep working on every call.
        self.runtime
            .handle()
            .block_on(self.transcribe_chunk_async(samples, sample_rate))
    }

    async fn transcribe_chunk_async(&self, samples: &[f32], sample_rate: u32) -> Result<String> {
        // Resample to 16kHz if needed (Whisper expects 16kHz)
        let resampled = if sample_rate != 16000 {
            resample_to_16k(samples, sample_rate)
        } else {
            samples.to_vec()
        };

        // Encode to WAV format
        let wav_bytes = encode_wav(&resampled, 16000)?;

        let file_part = Part::bytes(wav_bytes)
            .file_name("audio.wav")
            .mime_str("audio/wav")
            .context("Failed to create file part")?;

        let form = Form::new().part("file", file_part);

        let url = format!("{}/v1/audio/transcriptions", self.asr_url.trim_end_matches('/'));
        let response = self
            .client
            .post(&url)
            .multipart(form)
            .send()
            .await
            .with_context(|| format!("Failed to send transcription request to {}", url))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            anyhow::bail!("ASR request failed: {} - {}", status, error_text);
        }

        // Plain JSON response: {"text": "..."}
        let json: serde_json::Value = response
            .json()
            .await
            .context("Failed to parse ASR JSON response")?;

        let text = json
            .get("text")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string();

        Ok(clean_text(&text))
    }
}

fn encode_wav(samples: &[f32], sample_rate: u32) -> Result<Vec<u8>> {
    let mut cursor = Cursor::new(Vec::new());

    let spec = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };

    let mut writer = hound::WavWriter::new(&mut cursor, spec)
        .context("Failed to create WAV writer")?;

    // Convert f32 (-1.0 to 1.0) to i16
    for &sample in samples {
        let int_sample = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        writer
            .write_sample(int_sample)
            .context("Failed to write sample")?;
    }

    writer.finalize().context("Failed to finalize WAV")?;

    Ok(cursor.into_inner())
}

fn clean_text(text: &str) -> String {
    let special_tokens = [
        "[BLANK_AUDIO]",
        "[NOISE]",
        "[MUSIC]",
        "[SPEECH]",
        "[UNKNOWN]",
        "[LAUGHTER]",
        "[APPLAUSE]",
        "[COUGH]",
        "[THROAT_CLEARING]",
        "[BREATH]",
    ];

    let mut cleaned = text.to_string();
    for token in special_tokens {
        cleaned = cleaned.replace(token, "");
    }

    let words: Vec<&str> = cleaned.split_whitespace().collect();
    words.join(" ")
}

pub(crate) fn resample_to_16k(samples: &[f32], from_rate: u32) -> Vec<f32> {
    if from_rate == 16000 {
        return samples.to_vec();
    }

    let ratio = 16000.0 / from_rate as f64;
    let new_len = (samples.len() as f64 * ratio) as usize;
    let mut result = Vec::with_capacity(new_len);

    for i in 0..new_len {
        let src_idx = i as f64 / ratio;
        let idx = src_idx as usize;
        let frac = src_idx - idx as f64;

        let s0 = samples.get(idx).copied().unwrap_or(0.0);
        let s1 = samples.get(idx + 1).copied().unwrap_or(s0);
        result.push(s0 * (1.0 - frac as f32) + s1 * frac as f32);
    }

    result
}
