use anyhow::{Context, Result};
use reqwest::multipart::{Form, Part};
use std::io::Cursor;

pub struct StreamTranscriber {
    api_token: String,
    client: reqwest::Client,
}

impl StreamTranscriber {
    pub fn new(_whisper_bin: &str, _model_path: &std::path::Path) -> Self {
        let api_token = std::env::var("ZAI_API_TOKEN")
            .expect("ZAI_API_TOKEN environment variable must be set");

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("Failed to build HTTP client");

        Self {
            api_token,
            client,
        }
    }

    pub fn transcribe_chunk(&self, samples: &[f32], sample_rate: u32) -> Result<String> {
        if samples.is_empty() {
            return Ok(String::new());
        }

        // Run async code in a blocking manner
        let rt = tokio::runtime::Handle::try_current();
        match rt {
            Ok(handle) => handle.block_on(self.transcribe_chunk_async(samples, sample_rate)),
            Err(_) => {
                let rt = tokio::runtime::Runtime::new()
                    .context("Failed to create tokio runtime")?;
                rt.block_on(self.transcribe_chunk_async(samples, sample_rate))
            }
        }
    }

    async fn transcribe_chunk_async(&self, samples: &[f32], sample_rate: u32) -> Result<String> {
        // Resample to 16kHz if needed (z.ai ASR model expects 16kHz)
        let resampled = if sample_rate != 16000 {
            resample_to_16k(samples, sample_rate)
        } else {
            samples.to_vec()
        };

        // Encode to WAV format
        let wav_bytes = encode_wav(&resampled, 16000)?;

        // Create multipart form
        let file_part = Part::bytes(wav_bytes)
            .file_name("audio.wav")
            .mime_str("audio/wav")
            .context("Failed to create file part")?;

        let form = Form::new()
            .text("model", "glm-asr-2512")
            .text("stream", "true")
            .part("file", file_part);

        // Send request to z.ai API
        let response = self
            .client
            .post("https://api.z.ai/api/paas/v4/audio/transcriptions")
            .header("Authorization", format!("Bearer {}", self.api_token))
            .multipart(form)
            .send()
            .await
            .context("Failed to send transcription request")?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            anyhow::bail!("API request failed: {} - {}", status, error_text);
        }

        // Handle streaming response
        let text = self.handle_stream_async(response).await?;

        Ok(clean_text(&text))
    }

    async fn handle_stream_async(&self, response: reqwest::Response) -> Result<String> {
        use futures_util::StreamExt;

        let mut text_parts = Vec::new();
        let mut stream = response.bytes_stream();

        while let Some(chunk_result) = stream.next().await {
            let chunk = chunk_result.context("Failed to read stream chunk")?;
            let chunk_str = String::from_utf8_lossy(&chunk);

            // Parse SSE format: "data: {...}\n\n"
            for line in chunk_str.lines() {
                let line = line.trim();
                if let Some(data) = line.strip_prefix("data: ") {
                    if data == "[DONE]" {
                        break;
                    }

                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(data) {
                        if let Some(text) = json.get("text").and_then(|t| t.as_str()) {
                            text_parts.push(text.to_string());
                        }
                    }
                }
            }
        }

        Ok(text_parts.join(""))
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

fn resample_to_16k(samples: &[f32], from_rate: u32) -> Vec<f32> {
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