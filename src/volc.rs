//! VolcEngine streaming ASR — the same speech engine that powers the
//! `pi-voice-input` pi extension.
//!
//! Credentials are read from the extension's config file
//! (`~/.pi/agent/voice-input.config.json` by default, override with
//! `VT_VOLC_CONFIG`) so both tools share a single API key.
//!
//! Unlike the extension (which uses the one-shot endpoint after recording),
//! this module talks to the *streaming* endpoint and emits partial
//! transcripts while you speak, which are typed live at the cursor.

use anyhow::{bail, Context, Result};
use crossbeam_channel::{bounded, Receiver, Sender};
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::Message;

use crate::asr::resample_to_16k;

const WS_URL: &str = "wss://openspeech.bytedance.com/api/v3/sauc/bigmodel";
const RESOURCE_ID: &str = "volc.bigasr.sauc.duration";
/// Audio is streamed in 100 ms packets for low-latency live preview.
const SEGMENT_MS: usize = 100;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const IDLE_TIMEOUT: Duration = Duration::from_secs(120);
const FINAL_TIMEOUT: Duration = Duration::from_secs(30);

// VolcEngine v3 binary framing (same wire protocol as pi-voice-input).
const MSG_CLIENT_FULL_REQUEST: u8 = 0b0001;
const MSG_CLIENT_AUDIO_ONLY: u8 = 0b0010;
const MSG_SERVER_FULL_RESPONSE: u8 = 0b1001;
const MSG_SERVER_ERROR: u8 = 0b1111;
const FLAG_POS_SEQUENCE: u8 = 0b0001;
const FLAG_NEG_WITH_SEQUENCE: u8 = 0b0011;
const FLAG_SERVER_LAST: u8 = 0b0010;
const SERIALIZATION_NONE: u8 = 0b0000;
const SERIALIZATION_JSON: u8 = 0b0001;
const COMPRESSION_GZIP: u8 = 0b0001;

pub struct VolcConfig {
    pub api_key: String,
    pub boosting_table_id: String,
    pub config_path: PathBuf,
}

/// Loads the shared pi-voice-input config. Returns an error if the file
/// cannot be read/parsed (callers treat a missing/empty key as "unavailable").
/// Loads the VolcEngine credentials.
///
/// Priority:
///   1. `VT_VOLC_API_KEY` env var (optionally `VT_VOLC_BOOSTING_TABLE_ID`)
///      — no file needed at all
///   2. `VT_VOLC_CONFIG` env var — path to a JSON config file
///   3. `~/.pi/agent/voice-input.config.json` (shared with the pi
///      voice-input extension)
pub fn load_config() -> Result<VolcConfig> {
    if let Ok(key) = std::env::var("VT_VOLC_API_KEY") {
        let key = key.trim().to_string();
        if !key.is_empty() {
            return Ok(VolcConfig {
                api_key: key,
                boosting_table_id: std::env::var("VT_VOLC_BOOSTING_TABLE_ID")
                    .unwrap_or_default()
                    .trim()
                    .to_string(),
                config_path: PathBuf::from("<VT_VOLC_API_KEY>"),
            });
        }
    }

    let path = std::env::var("VT_VOLC_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|_| default_config_path());
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let v: Value = serde_json::from_str(&raw).context("invalid voice input config JSON")?;
    Ok(VolcConfig {
        api_key: v
            .get("volcApiKey")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string(),
        boosting_table_id: v
            .get("boostingTableId")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string(),
        config_path: path,
    })
}

fn default_config_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".pi")
        .join("agent")
        .join("voice-input.config.json")
}

/// True when the VolcEngine provider can be used (key present).
pub fn available() -> bool {
    load_config()
        .map(|c| !c.api_key.is_empty())
        .unwrap_or(false)
}

pub enum VolcCmd {
    Audio { samples: Vec<f32>, sample_rate: u32 },
    Finish,
}

pub enum VolcEvent {
    /// WebSocket is open and the request config was accepted.
    Ready,
    /// Incremental recognized text (cumulative) — drives the live preview.
    Partial(String),
    /// Final transcript after the last audio packet was acknowledged.
    Final(String),
    Failed(String),
}

pub struct VolcSession {
    cmd_tx: mpsc::Sender<VolcCmd>,
    pub events: Receiver<VolcEvent>,
}

impl VolcSession {
    /// Spawns a worker thread that owns its own single-threaded tokio
    /// runtime. `connect_async` happens in the background; audio queued on
    /// the command channel is flushed once the socket is up.
    pub fn start() -> Result<Self> {
        let cfg = load_config()?;
        if cfg.api_key.is_empty() {
            bail!(
                "VolcEngine API key missing: set VT_VOLC_API_KEY, or configure \
                 volcApiKey in {} (e.g. via /voice key inside pi)",
                cfg.config_path.display()
            );
        }

        let (cmd_tx, mut cmd_rx) = mpsc::channel::<VolcCmd>(512);
        let (ev_tx, ev_rx) = bounded::<VolcEvent>(256);

        std::thread::Builder::new()
            .name("volc-asr".into())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        let _ = ev_tx.try_send(VolcEvent::Failed(format!(
                            "failed to build tokio runtime: {e}"
                        )));
                        return;
                    }
                };
                let ev_report = ev_tx.clone();
                if let Err(e) = rt.block_on(run(&mut cmd_rx, ev_tx, cfg)) {
                    let _ = ev_report.try_send(VolcEvent::Failed(format!("{e:#}")));
                }
            })
            .context("failed to spawn volc-asr worker thread")?;

        Ok(Self {
            cmd_tx,
            events: ev_rx,
        })
    }

    /// Queues an audio chunk (any sample rate, mono f32); drops silently if
    /// the worker is backed up so the audio callback never blocks.
    pub fn send_audio(&self, samples: &[f32], sample_rate: u32) {
        let _ = self.cmd_tx.try_send(VolcCmd::Audio {
            samples: samples.to_vec(),
            sample_rate,
        });
    }

    /// Signals end-of-audio: sends the last packet and makes the worker
    /// wait for the final transcript.
    pub fn finish(&self) {
        let _ = self.cmd_tx.try_send(VolcCmd::Finish);
    }
}

macro_rules! vlog {
    ($($arg:tt)*) => {
        if std::env::var_os("VT_LOG").is_some_and(|v| !v.is_empty()) {
            eprintln!("[vt/volc] {}", format!($($arg)*));
        }
    };
}

async fn run(
    cmd_rx: &mut mpsc::Receiver<VolcCmd>,
    ev: Sender<VolcEvent>,
    cfg: VolcConfig,
) -> Result<()> {
    let connect_id = uuid::Uuid::new_v4().to_string();
    let mut req = WS_URL
        .into_client_request()
        .context("invalid ASR websocket URL")?;
    let headers = req.headers_mut();
    headers.insert("X-Api-Key", HeaderValue::from_str(&cfg.api_key)?);
    headers.insert("X-Api-Resource-Id", HeaderValue::from_static(RESOURCE_ID));
    headers.insert("X-Api-Connect-Id", HeaderValue::from_str(&connect_id)?);
    headers.insert("X-Api-Request-Id", HeaderValue::from_str(&connect_id)?);
    headers.insert("X-Api-Sequence", HeaderValue::from_static("-1"));

    let (ws, _resp) = tokio::time::timeout(CONNECT_TIMEOUT, tokio_tungstenite::connect_async(req))
        .await
        .map_err(|_| anyhow::anyhow!("timed out connecting to VolcEngine ASR"))?
        .context("failed to connect to VolcEngine ASR")?;
    let _ = ev.try_send(VolcEvent::Ready);
    vlog!("connected");

    let (mut write, mut read) = ws.split();

    // 1) full request (session config) with sequence 1
    write
        .send(Message::Binary(
            full_request(1, &request_payload(&cfg))?.into(),
        ))
        .await
        .context("failed to send ASR full request")?;
    let mut seq: i32 = 2;
    vlog!("full request sent");

    // Pending 16 kHz mono s16le PCM bytes waiting to be segmented.
    let mut pcm: Vec<u8> = Vec::new();
    let seg_bytes = 16000 * 2 * SEGMENT_MS / 1000;
    let mut last_text = String::new();
    let mut finished = false;

    loop {
        let read_timeout = if finished {
            FINAL_TIMEOUT
        } else {
            IDLE_TIMEOUT
        };
        tokio::select! {
            cmd = cmd_rx.recv(), if !finished => {
                match cmd {
                    Some(VolcCmd::Audio { samples, sample_rate }) => {
                        vlog!("audio cmd: {} samples @ {}", samples.len(), sample_rate);
                        for s in resample_to_16k(&samples, sample_rate) {
                            let b = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
                            pcm.extend_from_slice(&b.to_le_bytes());
                        }
                        while pcm.len() >= seg_bytes {
                            let seg: Vec<u8> = pcm.drain(..seg_bytes).collect();
                            vlog!("sending audio packet seq={}", seq);
                            write
                                .send(Message::Binary(audio_request(seq, &seg, false)?.into()))
                                .await
                                .context("failed to send ASR audio packet")?;
                            seq += 1;
                        }
                    }
                    Some(VolcCmd::Finish) | None => {
                        vlog!("finish cmd; sending last packet seq={}", seq);
                        let tail = std::mem::take(&mut pcm);
                        write
                            .send(Message::Binary(audio_request(seq, &tail, true)?.into()))
                            .await
                            .context("failed to send final ASR audio packet")?;
                        finished = true;
                    }
                }
            }

            msg = tokio::time::timeout(read_timeout, read.next()) => {
                let msg = match msg {
                    Ok(m) => m,
                    Err(_) => bail!("timed out waiting for ASR response"),
                };
                match msg {
                    Some(Ok(Message::Binary(data))) => {
                        let frame = parse_frame(&data)?;
                        let text = extract_text(&frame.payload);
                        vlog!("server frame: is_last={} text={:?}", frame.is_last, text);
                        if !text.is_empty() {
                            last_text = text.clone();
                        }
                        if frame.is_last {
                            let _ = ev.try_send(VolcEvent::Final(last_text.clone()));
                            return Ok(());
                        }
                        if !text.is_empty() {
                            let _ = ev.try_send(VolcEvent::Partial(text));
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        if finished {
                            let _ = ev.try_send(VolcEvent::Final(last_text.clone()));
                            return Ok(());
                        }
                        bail!("ASR socket closed before the final result");
                    }
                    Some(Ok(_)) => {} // ping/pong handled by tungstenite
                    Some(Err(e)) => bail!("ASR socket error: {e}"),
                }
            }
        }
    }
}

fn request_payload(cfg: &VolcConfig) -> Value {
    let mut request = json!({
        "model_name": "bigmodel",
        "enable_itn": true,
        "enable_punc": true,
        "enable_ddc": false,
        "result_type": "full",
    });
    if !cfg.boosting_table_id.is_empty() {
        request["corpus"] = json!({ "boosting_table_id": cfg.boosting_table_id });
    }
    json!({
        "user": { "uid": "voice-type" },
        "audio": {
            "format": "pcm",
            "codec": "raw",
            "rate": 16000,
            "bits": 16,
            "channel": 1,
        },
        "request": request,
    })
}

// ---------------------------------------------------------------------------
// VolcEngine v3 binary framing
// ---------------------------------------------------------------------------

fn frame_header(message_type: u8, flags: u8, serialization: u8, compression: u8) -> [u8; 4] {
    [
        0x11,
        (message_type << 4) | flags,
        (serialization << 4) | compression,
        0,
    ]
}

/// protocol version 1, header size 1 (4 bytes)
fn full_request(seq: i32, payload: &Value) -> Result<Vec<u8>> {
    let body = gzip(payload.to_string().as_bytes())?;
    let mut out = Vec::with_capacity(16 + body.len());
    out.extend_from_slice(&frame_header(
        MSG_CLIENT_FULL_REQUEST,
        FLAG_POS_SEQUENCE,
        SERIALIZATION_JSON,
        COMPRESSION_GZIP,
    ));
    out.extend_from_slice(&seq.to_be_bytes());
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(&body);
    Ok(out)
}

/// Audio-only packet; the last packet uses a negative sequence + flag 0b0011.
fn audio_request(seq: i32, audio: &[u8], is_last: bool) -> Result<Vec<u8>> {
    let body = gzip(audio)?;
    let flags = if is_last {
        FLAG_NEG_WITH_SEQUENCE
    } else {
        FLAG_POS_SEQUENCE
    };
    let wire_seq = if is_last { -seq } else { seq };
    let mut out = Vec::with_capacity(16 + body.len());
    out.extend_from_slice(&frame_header(
        MSG_CLIENT_AUDIO_ONLY,
        flags,
        SERIALIZATION_NONE,
        COMPRESSION_GZIP,
    ));
    out.extend_from_slice(&wire_seq.to_be_bytes());
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(&body);
    Ok(out)
}

#[derive(Debug)]
struct ServerFrame {
    is_last: bool,
    payload: Value,
}

fn parse_frame(msg: &[u8]) -> Result<ServerFrame> {
    if msg.len() < 4 {
        bail!("ASR frame too short");
    }
    let header_size = (msg[0] & 0x0f) as usize * 4;
    if msg.len() < header_size {
        bail!("ASR frame header truncated");
    }
    let message_type = msg[1] >> 4;
    let flags = msg[1] & 0x0f;
    let serialization = msg[2] >> 4;
    let compression = msg[2] & 0x0f;
    let mut off = header_size;

    if flags & FLAG_POS_SEQUENCE != 0 {
        if off + 4 > msg.len() {
            bail!("ASR frame sequence truncated");
        }
        let _seq = i32::from_be_bytes(msg[off..off + 4].try_into().unwrap());
        off += 4;
    }

    match message_type {
        MSG_SERVER_FULL_RESPONSE => {
            if off + 4 > msg.len() {
                bail!("ASR frame payload size truncated");
            }
            let size = u32::from_be_bytes(msg[off..off + 4].try_into().unwrap()) as usize;
            off += 4;
            if off + size > msg.len() {
                bail!("ASR frame payload truncated");
            }
            let payload = maybe_gunzip(&msg[off..off + size], compression)?;
            let value = if serialization == SERIALIZATION_JSON && !payload.is_empty() {
                serde_json::from_slice(&payload).context("failed to parse ASR JSON payload")?
            } else {
                Value::Null
            };
            Ok(ServerFrame {
                is_last: flags & FLAG_SERVER_LAST != 0,
                payload: value,
            })
        }
        MSG_SERVER_ERROR => {
            if off + 4 > msg.len() {
                bail!("ASR error frame truncated");
            }
            let code = i32::from_be_bytes(msg[off..off + 4].try_into().unwrap());
            off += 4;
            let size = if off + 4 <= msg.len() {
                let s = u32::from_be_bytes(msg[off..off + 4].try_into().unwrap()) as usize;
                off += 4;
                s
            } else {
                0
            };
            let payload = if size > 0 && off + size <= msg.len() {
                maybe_gunzip(&msg[off..off + size], compression)
                    .unwrap_or_else(|_| msg[off..off + size].to_vec())
            } else {
                Vec::new()
            };
            bail!(
                "VolcEngine ASR error {}: {}",
                code,
                String::from_utf8_lossy(&payload)
            );
        }
        _ => Ok(ServerFrame {
            is_last: false,
            payload: Value::Null,
        }),
    }
}

fn extract_text(payload: &Value) -> String {
    if let Some(text) = payload.pointer("/result/text").and_then(Value::as_str) {
        let text = text.trim();
        if !text.is_empty() {
            return text.to_string();
        }
    }
    if let Some(utts) = payload
        .pointer("/result/utterances")
        .and_then(Value::as_array)
    {
        let joined: String = utts
            .iter()
            .filter_map(|u| u.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("");
        let joined = joined.trim();
        if !joined.is_empty() {
            return joined.to_string();
        }
    }
    String::new()
}

// ---------------------------------------------------------------------------
// gzip helpers
// ---------------------------------------------------------------------------

fn gzip(data: &[u8]) -> Result<Vec<u8>> {
    let mut enc = GzEncoder::new(Vec::new(), Compression::fast());
    enc.write_all(data).context("gzip write failed")?;
    enc.finish().context("gzip finish failed")
}

fn maybe_gunzip(data: &[u8], compression: u8) -> Result<Vec<u8>> {
    if compression != COMPRESSION_GZIP || data.is_empty() {
        return Ok(data.to_vec());
    }
    let mut dec = GzDecoder::new(data);
    let mut out = Vec::new();
    dec.read_to_end(&mut out).context("gunzip failed")?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn env_api_key_takes_precedence_over_files() {
        // Point the file fallback at a nonexistent path: if the env var is
        // honored, no file is ever read.
        std::env::set_var("VT_VOLC_CONFIG", "/nonexistent/voice-input.config.json");
        std::env::set_var("VT_VOLC_API_KEY", "test-key-from-env");
        std::env::set_var("VT_VOLC_BOOSTING_TABLE_ID", "tbl-123");
        let cfg = load_config().unwrap();
        assert_eq!(cfg.api_key, "test-key-from-env");
        assert_eq!(cfg.boosting_table_id, "tbl-123");
        assert_eq!(cfg.config_path, PathBuf::from("<VT_VOLC_API_KEY>"));
        std::env::remove_var("VT_VOLC_API_KEY");
        std::env::remove_var("VT_VOLC_BOOSTING_TABLE_ID");
        std::env::remove_var("VT_VOLC_CONFIG");
    }

    fn server_full_frame(seq: i32, payload: &Value, is_last: bool) -> Vec<u8> {
        let body = gzip(payload.to_string().as_bytes()).unwrap();
        let flags = FLAG_POS_SEQUENCE | if is_last { FLAG_SERVER_LAST } else { 0 };
        let mut out = vec![
            0x11u8,
            (MSG_SERVER_FULL_RESPONSE << 4) | flags,
            (SERIALIZATION_JSON << 4) | COMPRESSION_GZIP,
            0,
        ];
        out.extend_from_slice(&seq.to_be_bytes());
        out.extend_from_slice(&(body.len() as u32).to_be_bytes());
        out.extend_from_slice(&body);
        out
    }

    #[test]
    fn parse_server_full_response_frame() {
        let payload = json!({"result": {"text": "你好 world"}});
        let frame = server_full_frame(7, &payload, false);
        let parsed = parse_frame(&frame).unwrap();
        assert!(!parsed.is_last);
        assert_eq!(extract_text(&parsed.payload), "你好 world");
    }

    #[test]
    fn parse_last_frame_with_utterances_fallback() {
        let payload =
            json!({"result": {"text": "", "utterances": [{"text": "a "}, {"text": "b"}]}});
        let frame = server_full_frame(9, &payload, true);
        let parsed = parse_frame(&frame).unwrap();
        assert!(parsed.is_last);
        assert_eq!(extract_text(&parsed.payload), "a b");
    }

    #[test]
    fn parse_error_frame() {
        let body = gzip(br#"{"message":"bad key"}"#).unwrap();
        let mut frame = vec![
            0x11u8,
            MSG_SERVER_ERROR << 4,
            (SERIALIZATION_JSON << 4) | COMPRESSION_GZIP,
            0,
        ];
        frame.extend_from_slice(&45000003i32.to_be_bytes());
        frame.extend_from_slice(&(body.len() as u32).to_be_bytes());
        frame.extend_from_slice(&body);
        let err = parse_frame(&frame).unwrap_err();
        assert!(err.to_string().contains("45000003"));
    }

    /// End-to-end protocol check against the real streaming endpoint using
    /// the shared pi-voice-input API key. Run with:
    ///   cargo test -- --ignored live_streaming_session --nocapture
    #[test]
    #[ignore]
    fn live_streaming_session() {
        if !available() {
            eprintln!("skip: no VolcEngine API key configured");
            return;
        }
        let session = VolcSession::start().unwrap();

        // ~1.5 s of 440 Hz tone at 44.1 kHz (exercises the resample path too).
        let sr = 44_100u32;
        let n = sr as usize * 3 / 2;
        let mut samples = Vec::with_capacity(n);
        let mut t = 0.0f32;
        for _ in 0..n {
            samples.push((t * 2.0 * std::f32::consts::PI * 440.0).sin() * 0.3);
            t += 1.0 / sr as f32;
        }
        session.send_audio(&samples, sr);
        session.finish();

        let deadline = Instant::now() + Duration::from_secs(30);
        let mut saw_partial = false;
        loop {
            match session.events.recv_timeout(Duration::from_millis(200)) {
                Ok(VolcEvent::Ready) => {}
                Ok(VolcEvent::Partial(_)) => saw_partial = true,
                Ok(VolcEvent::Final(text)) => {
                    eprintln!("final={text:?} partial_seen={saw_partial}");
                    break; // protocol round-trip succeeded (text may be empty)
                }
                Ok(VolcEvent::Failed(e)) => panic!("streaming failed: {e}"),
                Err(_) if Instant::now() > deadline => panic!("timeout waiting for final"),
                Err(_) => {}
            }
        }
    }
}
