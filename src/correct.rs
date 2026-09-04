//! Post-ASR transcript correction via VolcEngine Ark (ByteDance LLM).
//!
//! After the final transcript is typed, a fast *non-thinking* chat model
//! fixes homophones, typos, punctuation and filler words, then the typed
//! text is replaced with the corrected version (backspace + retype).
//!
//! Configuration (env):
//!   VT_ARK_API_KEY     Ark API key — correction is enabled only when set
//!   VT_CORRECT_MODEL   model id or `ep-` endpoint (default: doubao-seed-2-1-turbo-260628,
//!                       a fast non-thinking model)
//!   VT_CORRECT         set to 0/false/no to disable even when a key exists
//!
//! Ark API is OpenAI-compatible:
//!   POST https://ark.cn-beijing.volces.com/api/v3/chat/completions

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

const ARK_URL: &str = "https://ark.cn-beijing.volces.com/api/v3/chat/completions";

const SYSTEM_PROMPT: &str = "You fix speech-recognition transcripts. Correct homophones, \
typos and punctuation, remove filler words (嗯/呃/那个/um/uh), and keep the original \
meaning, language and wording otherwise unchanged. NEVER add explanations, translations \
or new content. Reply with the corrected text ONLY. If the input is already correct, \
return it unchanged.";

pub struct Corrector {
    enabled: bool,
    api_key: String,
    model: String,
    client: reqwest::Client,
    runtime: tokio::runtime::Runtime,
}

impl Corrector {
    pub fn new() -> Self {
        let api_key = std::env::var("VT_ARK_API_KEY")
            .unwrap_or_default()
            .trim()
            .to_string();
        let enabled = !api_key.is_empty()
            && !matches!(
                std::env::var("VT_CORRECT")
                    .unwrap_or_default()
                    .to_lowercase()
                    .as_str(),
                "0" | "false" | "no" | "off"
            );

        let model = std::env::var("VT_CORRECT_MODEL")
            .unwrap_or_else(|_| "doubao-seed-2-1-turbo-260628".to_string());

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("Failed to build corrector tokio runtime");

        // Fast fix only: bail out early rather than hang the replacement.
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("Failed to build corrector HTTP client");

        Self {
            enabled,
            api_key,
            model,
            client,
            runtime,
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Returns the corrected text (or the original on trivial input).
    pub fn correct(&self, raw: &str) -> Result<String> {
        if !self.enabled || raw.trim().is_empty() {
            return Ok(raw.to_string());
        }
        self.runtime.handle().block_on(self.correct_async(raw))
    }

    async fn correct_async(&self, raw: &str) -> Result<String> {
        let mut body = json!({
            "model": self.model,
            "messages": [
                { "role": "system", "content": SYSTEM_PROMPT },
                { "role": "user", "content": raw },
            ],
            "temperature": 0.1,
            "max_tokens": 1024,
        });
        // Doubao "seed" models default to thinking mode; disable it so the
        // correction stays a fast single pass.
        let use_thinking_off = self.model.contains("seed") || self.model.contains("1.6");
        if use_thinking_off {
            body["thinking"] = json!({ "type": "disabled" });
        }

        let send = |body: Value| {
            self.client
                .post(ARK_URL)
                .bearer_auth(&self.api_key)
                .json(&body)
                .send()
        };

        let mut resp = send(body.clone()).await;
        // Some models reject the thinking parameter; retry without it.
        let status = resp.as_ref().map(|r| r.status()).unwrap_or_default();
        if use_thinking_off && (status.as_u16() == 400 || status.as_u16() == 404) {
            let mut retry = body.clone();
            retry.as_object_mut().unwrap().remove("thinking");
            resp = send(retry).await;
        }

        let resp = resp.context("failed to send Ark correction request")?;
        let status = resp.status();
        let payload: Value = resp
            .json()
            .await
            .context("failed to parse Ark correction response")?;
        if !status.is_success() {
            bail!(
                "Ark correction failed ({}): {}",
                status,
                serde_json::to_string(&payload).unwrap_or_default()
            );
        }

        let text = payload
            .pointer("/choices/0/message/content")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or("")
            .trim()
            .to_string();

        if text.is_empty() {
            bail!("Ark correction returned empty content");
        }
        Ok(text)
    }
}
