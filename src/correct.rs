use anyhow::{Context, Result};

/// Corrects ASR output via a local LLM (qwen2.5 through Ollama by default).
///
/// POST {base_url}/api/chat  with `stream: false`, returning the model's
/// `message.content`. The prompt asks for a faithful, punctuation-fixed,
/// homophone-fixed rewrite with no explanations.
///
/// Note: prefer non-thinking instruct models here. Reasoning models (e.g.
/// qwen3) tend to ignore "output only the text" and emit long chains of
/// thought, which is both slow on CPU and pollutes the output.
pub struct Corrector {
    enabled: bool,
    base_url: String,
    model: String,
    client: reqwest::Client,
    runtime: tokio::runtime::Runtime,
}

impl Corrector {
    pub fn new() -> Self {
        let enabled = std::env::var("VT_CORRECT")
            .map(|v| !(v == "0" || v.eq_ignore_ascii_case("false") || v == "no"))
            .unwrap_or(true);

        let base_url = std::env::var("VT_CORRECTOR_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:11434".to_string());

        let model = std::env::var("VT_CORRECTOR_MODEL")
            .unwrap_or_else(|_| "qwen2.5:3b-instruct".to_string());

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("Failed to build corrector tokio runtime");

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .expect("Failed to build corrector HTTP client");

        Self {
            enabled,
            base_url,
            model,
            client,
            runtime,
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Returns the corrected text. If correction is disabled or the input is
    /// trivially empty, the original text is returned unchanged.
    pub fn correct(&self, raw: &str) -> Result<String> {
        if !self.enabled || raw.trim().is_empty() {
            return Ok(raw.to_string());
        }
        self.runtime
            .handle()
            .block_on(self.correct_async(raw))
    }

    async fn correct_async(&self, raw: &str) -> Result<String> {
        let body = serde_json::json!({
            "model": self.model,
            "stream": false,
            "options": { "temperature": 0.0 },
            "messages": [
                { "role": "system", "content": SYSTEM_PROMPT },
                { "role": "user", "content": raw },
            ],
        });

        let url = format!("{}/api/chat", self.base_url.trim_end_matches('/'));
        let response = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .with_context(|| format!("Failed to call corrector at {}", url))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_else(|_| "Unknown error".to_string());
            anyhow::bail!("Corrector request failed: {} - {}", status, error_text);
        }

        let json: serde_json::Value = response
            .json()
            .await
            .context("Failed to parse corrector JSON response")?;

        let content = json
            .pointer("/message/content")
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .trim()
            .to_string();

        // The model occasionally wraps its answer in quotes / markdown.
        let content = strip_fences(&content);

        // The transcript we typed is single-line; force the correction to be
        // single-line too, otherwise the typed correction would land on a new
        // line in the target field. Collapse any internal newlines/CRs.
        let content = content.split_whitespace().collect::<Vec<_>>().join(" ");

        if content.is_empty() {
            Ok(raw.to_string())
        } else {
            Ok(content)
        }
    }
}

/// Removes a leading/trailing pair of code fences or surrounding quotes that
/// small models sometimes add despite the "no explanation" instruction.
fn strip_fences(s: &str) -> String {
    let s = s.trim();
    let s = if s.starts_with("```") {
        let inner = s.trim_start_matches('`');
        // drop a possible language tag on the first line
        let inner = inner.trim_start_matches(|c: char| c.is_alphanumeric());
        inner.trim().trim_end_matches("```").trim().to_string()
    } else {
        s.to_string()
    };
    s.trim_matches(|c: char| c == '"' || c == '\'' || c == '\u{201c}' || c == '\u{201d}')
        .to_string()
}

const SYSTEM_PROMPT: &str = "你是一个语音转写(ASR)结果的后处理助手。任务:对用户给你的转写文本进行修正。\n\
规则:\n\
1. 修正同音字、错别字与口语化导致的明显错误;\n\
2. 补充/调整中文标点与英文标点,使句子通顺;\n\
3. 去除\"嗯、啊、那个\"等无意义口水词(但保留说话人原意);\n\
4. 保持原语种(中文保持中文,英文保持英文),不要翻译;\n\
5. 忠于原意,不要增删信息、不要扩写;\n\
6. 只输出修正后的文本,不要任何解释、前缀、引号或代码块。";
