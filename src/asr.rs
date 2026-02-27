use anyhow::Result;
use std::path::Path;
use std::sync::{Arc, Mutex};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

pub struct StreamTranscriber {
    ctx: Arc<Mutex<WhisperContext>>,
}

impl StreamTranscriber {
    pub fn new(_whisper_bin: &str, model_path: &Path) -> Self {
        whisper_rs::install_logging_hooks();

        let ctx_params = WhisperContextParameters::default();
        let ctx =
            WhisperContext::new_with_params(model_path.to_string_lossy().as_ref(), ctx_params)
                .expect("Failed to load whisper model");

        Self {
            ctx: Arc::new(Mutex::new(ctx)),
        }
    }

    pub fn transcribe_chunk(&self, samples: &[f32], sample_rate: u32) -> Result<String> {
        if samples.is_empty() {
            return Ok(String::new());
        }

        let ctx = self
            .ctx
            .lock()
            .map_err(|_| anyhow::anyhow!("Failed to lock whisper context"))?;

        let resample = if sample_rate != 16000 {
            Some(resample_to_16k(samples, sample_rate))
        } else {
            None
        };

        let audio_data = resample.as_ref().map(|r| r.as_slice()).unwrap_or(samples);

        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(Some("auto"));
        params.set_translate(false);
        params.set_no_context(true);
        params.set_single_segment(true);
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_suppress_blank(true);

        let mut state = ctx.create_state()?;
        state.full(params, audio_data)?;

        let num_segments = state.full_n_segments();
        let mut result = String::new();

        for i in 0..num_segments {
            if let Some(segment) = state.get_segment(i) {
                if let Ok(text) = segment.to_str_lossy() {
                    result.push_str(&text);
                }
            }
        }

        Ok(clean_text(&result))
    }
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
