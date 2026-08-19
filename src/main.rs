mod asr;
mod audio;
mod hotkey;
mod tray;
mod typewriter;
mod volc;

use anyhow::{Context, Result};
use asr::StreamTranscriber;
use crossbeam_channel::{bounded, select};
use hotkey::HotkeyEvent;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use volc::{VolcEvent, VolcSession};

const MIN_CHUNK_SAMPLES: usize = 8000;
const VOLC_FINAL_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Debug)]
pub enum UiState {
    Idle,
    Recording { started_at: Instant },
    Transcribing { started_at: Instant },
    Done { text: String },
    Error { msg: String },
}

enum AsrProvider {
    Auto,
    Volc,
    Whisper,
}

fn asr_provider() -> AsrProvider {
    match std::env::var("VT_ASR_PROVIDER")
        .unwrap_or_default()
        .to_lowercase()
        .as_str()
    {
        "volc" | "volcengine" | "pi-voice-input" => AsrProvider::Volc,
        "whisper" | "local" => AsrProvider::Whisper,
        _ => AsrProvider::Auto,
    }
}

/// VolcEngine streaming ASR (the pi-voice-input backend) is the default
/// whenever its API key is configured.
fn use_volc() -> bool {
    match asr_provider() {
        AsrProvider::Volc => true,
        AsrProvider::Whisper => false,
        AsrProvider::Auto => volc::available(),
    }
}

/// What must be typed to move the on-screen text from `typed` to `target`.
#[derive(Debug, PartialEq, Eq)]
enum TypingPlan {
    /// Nothing to do (identical).
    Noop,
    /// The new text only appended: type this delta at the cursor.
    Append(String),
    /// The ASR rewrote earlier text: `backspaces` chars to delete, then this
    /// text to type (from the longest common prefix).
    Replace { backspaces: usize, retype: String },
}

/// Computes the minimal typing action between two streaming results.
/// Pure function (unit-testable; the caller performs the actual typing).
fn typing_plan(typed: &str, target: &str) -> TypingPlan {
    if typed == target {
        return TypingPlan::Noop;
    }
    if target.starts_with(typed) {
        return TypingPlan::Append(target[typed.len()..].to_string());
    }
    let common = typed
        .chars()
        .zip(target.chars())
        .take_while(|(a, b)| a == b)
        .count();
    TypingPlan::Replace {
        backspaces: typed.chars().count() - common,
        retype: target.chars().skip(common).collect(),
    }
}

/// Tracks what has already been typed at the cursor for the current
/// utterance so streaming updates only need to type the delta (and can
/// backspace over a divergence if the ASR rewrites earlier text).
#[derive(Default)]
struct TypedState {
    text: String,
}

impl TypedState {
    fn reset(&mut self) {
        self.text.clear();
    }

    /// Types the difference between what is already on screen and `target`,
    /// updating the tracked state. Returns the final on-screen text.
    fn type_towards(&mut self, target: &str) -> String {
        let target = target.trim();
        if target.is_empty() {
            return self.text.clone();
        }

        match typing_plan(&self.text, target) {
            TypingPlan::Noop => {}
            TypingPlan::Append(delta) => {
                if let Err(e) = typewriter::type_text_auto(&delta) {
                    eprintln!("[vt] failed to type partial: {e:#}");
                    return self.text.clone();
                }
            }
            TypingPlan::Replace { backspaces, retype } => {
                if backspaces > 0 {
                    if let Err(e) = typewriter::backspace(backspaces) {
                        eprintln!("[vt] backspace failed: {e:#}");
                        return self.text.clone();
                    }
                }
                if let Err(e) = typewriter::type_text_auto(&retype) {
                    eprintln!("[vt] failed to retype partial: {e:#}");
                }
            }
        }
        self.text = target.to_string();
        self.text.clone()
    }
}

fn main() -> Result<()> {
    let dummy_path = PathBuf::from("/dev/null");
    let transcriber = Arc::new(StreamTranscriber::new("", &dummy_path));

    let (audio_tx, audio_rx) = bounded::<Vec<f32>>(256);
    audio::add_sender(audio_tx);

    let running = Arc::new(AtomicBool::new(true));

    let hotkey_rx = hotkey::start_hotkey_listener()
        .context("Failed to start hotkey listener.")?;

    let (cmd_tx, cmd_rx) = bounded::<tray::TrayCmd>(16);

    let tray_running = running.clone();
    std::thread::spawn(move || {
        if let Err(e) = tray::run_tray(tray_running, cmd_rx) {
            eprintln!("Tray error: {:#}", e);
        }
    });

    let _ = cmd_tx.send(tray::TrayCmd::UpdateState(UiState::Idle));

    let mut buffer: Vec<f32> = Vec::new();
    let mut sample_rate: u32 = 44_100;
    let mut audio_engine: Option<audio::AudioEngine> = None;
    let mut is_recording = false;

    // VolcEngine streaming session for the current utterance.
    let mut volc_session: Option<VolcSession> = None;
    let mut volc_pending_since: Option<Instant> = None;
    let mut typed = TypedState::default();

    loop {
        if !running.load(Ordering::SeqCst) {
            break;
        }

        select! {
            recv(hotkey_rx) -> event => {
                match event {
                    Ok(HotkeyEvent::Pressed) => {
                        if !is_recording {
                            buffer.clear();
                            typed.reset();
                            volc_pending_since = None;
                            let _ = cmd_tx.send(tray::TrayCmd::UpdateState(UiState::Recording {
                                started_at: Instant::now()
                            }));

                            // Default provider: VolcEngine streaming (pi-voice-input
                            // backend) when configured; falls back to local whisper.
                            volc_session = if use_volc() {
                                match VolcSession::start() {
                                    Ok(session) => Some(session),
                                    Err(e) => {
                                        eprintln!(
                                            "[vt] volc streaming unavailable, using local whisper: {e:#}"
                                        );
                                        None
                                    }
                                }
                            } else {
                                None
                            };

                            match audio::AudioEngine::start_default_input(None) {
                                Ok(engine) => {
                                    sample_rate = engine.sample_rate();
                                    audio_engine = Some(engine);
                                    is_recording = true;
                                }
                                Err(e) => {
                                    eprintln!("Failed to start recording: {:#}", e);
                                    volc_session = None;
                                    let _ = cmd_tx.send(tray::TrayCmd::UpdateState(UiState::Error { msg: e.to_string() }));
                                }
                            }
                        }
                    }
                    Ok(HotkeyEvent::Released) => {
                        if is_recording {
                            if let Some(engine) = audio_engine.take() {
                                engine.stop();
                            }

                            while let Ok(chunk) = audio_rx.try_recv() {
                                buffer.extend_from_slice(&chunk);
                                if let Some(session) = volc_session.as_ref() {
                                    session.send_audio(&chunk, sample_rate);
                                }
                            }

                            is_recording = false;

                            if let Some(session) = volc_session.as_ref() {
                                // Streaming path: partials were already typed
                                // live; flush the last packet and wait for the
                                // final result in the polling branch below.
                                let _ = cmd_tx.send(tray::TrayCmd::UpdateState(
                                    UiState::Transcribing { started_at: Instant::now() },
                                ));
                                session.finish();
                                volc_pending_since = Some(Instant::now());
                            } else {
                                // Local whisper path: transcribe the whole
                                // recording after release, then type it.
                                if buffer.len() >= MIN_CHUNK_SAMPLES {
                                    let _ = cmd_tx.send(tray::TrayCmd::UpdateState(
                                        UiState::Transcribing { started_at: Instant::now() },
                                    ));
                                    let samples = buffer.clone();
                                    let sr = sample_rate;
                                    let transcriber = Arc::clone(&transcriber);
                                    let cmd_tx_clone = cmd_tx.clone();
                                    std::thread::spawn(move || {
                                        match transcriber.transcribe_chunk(&samples, sr) {
                                            Ok(text) if !text.trim().is_empty() => {
                                                let _ = cmd_tx_clone.send(tray::TrayCmd::UpdateState(
                                                    UiState::Done { text: text.trim().to_string() },
                                                ));
                                                if let Err(e) = typewriter::type_text_auto(text.trim()) {
                                                    eprintln!("[vt] failed to type transcript: {e:#}");
                                                    let _ = cmd_tx_clone.send(tray::TrayCmd::UpdateState(
                                                        UiState::Error { msg: e.to_string() },
                                                    ));
                                                }
                                            }
                                            Ok(_) => {
                                                let _ = cmd_tx_clone.send(tray::TrayCmd::UpdateState(UiState::Idle));
                                            }
                                            Err(e) => {
                                                eprintln!("Transcription error: {:#}", e);
                                                let _ = cmd_tx_clone.send(tray::TrayCmd::UpdateState(UiState::Error { msg: e.to_string() }));
                                            }
                                        }
                                    });
                                } else {
                                    let _ = cmd_tx.send(tray::TrayCmd::UpdateState(UiState::Idle));
                                }

                                buffer.clear();
                            }
                        }
                    }
                    Err(_) => {
                        break;
                    }
                }
            }

            recv(audio_rx) -> chunk => {
                if let Ok(chunk) = chunk {
                    if is_recording {
                        buffer.extend_from_slice(&chunk);
                        if let Some(session) = volc_session.as_ref() {
                            session.send_audio(&chunk, sample_rate);
                        }
                    }
                }
            }

            default(Duration::from_millis(50)) => {
                if !is_recording {
                    while audio_rx.try_recv().is_ok() {}
                }

                // Streaming ASR events: Partial -> type the delta live at the
                // cursor, Final/Failed -> resolve the utterance.
                let mut resolution: Option<Result<String, String>> = None;
                if let Some(session) = volc_session.as_ref() {
                    loop {
                        match session.events.try_recv() {
                            Ok(VolcEvent::Ready) => {}
                            Ok(VolcEvent::Partial(text)) => {
                                if !text.is_empty() {
                                    typed.type_towards(&text);
                                }
                            }
                            Ok(VolcEvent::Final(text)) => {
                                resolution = Some(Ok(text));
                                break;
                            }
                            Ok(VolcEvent::Failed(msg)) => {
                                resolution = Some(Err(msg));
                                break;
                            }
                            Err(_) => break,
                        }
                    }
                    if resolution.is_none() {
                        if let Some(since) = volc_pending_since {
                            if since.elapsed() > VOLC_FINAL_TIMEOUT {
                                resolution = Some(Err(
                                    "volc streaming timed out waiting for the final transcript"
                                        .to_string(),
                                ));
                            }
                        }
                    }
                }

                if let Some(res) = resolution {
                    volc_session = None;
                    volc_pending_since = None;

                    match res {
                        Ok(text) if !text.trim().is_empty() => {
                            let final_text = typed.type_towards(&text);
                            let _ = cmd_tx.send(tray::TrayCmd::UpdateState(UiState::Done {
                                text: final_text,
                            }));
                        }
                        Ok(_) => {
                            let _ = cmd_tx.send(tray::TrayCmd::UpdateState(UiState::Idle));
                        }
                        Err(msg) => {
                            // Streaming failed: retry once with the local whisper
                            // server using the buffered recording. Any partials
                            // already typed stay on screen; whisper's result is
                            // typed after them.
                            eprintln!("[vt] volc streaming failed: {msg}");
                            if buffer.len() >= MIN_CHUNK_SAMPLES {
                                eprintln!("[vt] falling back to local whisper ASR");
                                let _ = cmd_tx.send(tray::TrayCmd::UpdateState(
                                    UiState::Transcribing { started_at: Instant::now() },
                                ));
                                let samples = buffer.clone();
                                let sr = sample_rate;
                                let transcriber = Arc::clone(&transcriber);
                                let cmd_tx_clone = cmd_tx.clone();
                                std::thread::spawn(move || {
                                    match transcriber.transcribe_chunk(&samples, sr) {
                                        Ok(text) if !text.trim().is_empty() => {
                                            let _ = cmd_tx_clone.send(tray::TrayCmd::UpdateState(
                                                UiState::Done { text: text.trim().to_string() },
                                            ));
                                            if let Err(e) = typewriter::type_text_auto(text.trim()) {
                                                eprintln!("[vt] failed to type transcript: {e:#}");
                                            }
                                        }
                                        _ => {
                                            let _ = cmd_tx_clone.send(tray::TrayCmd::UpdateState(UiState::Idle));
                                        }
                                    }
                                });
                            } else {
                                let _ = cmd_tx.send(tray::TrayCmd::UpdateState(UiState::Error {
                                    msg,
                                }));
                            }
                        }
                    }

                    buffer.clear();
                }
            }
        }
    }

    let _ = cmd_tx.send(tray::TrayCmd::Quit);
    running.store(false, Ordering::SeqCst);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_noop_and_append() {
        assert_eq!(typing_plan("你好", "你好"), TypingPlan::Noop);
        assert_eq!(
            typing_plan("你好", "你好世界 hello"),
            TypingPlan::Append("世界 hello".to_string())
        );
        // Empty start appends everything.
        assert_eq!(typing_plan("", "hi"), TypingPlan::Append("hi".to_string()));
    }

    #[test]
    fn plan_replace_on_divergence() {
        // ASR rewrote the last word (homophone): backspace 2 chars, retype.
        assert_eq!(
            typing_plan("你好时间", "你好世界"),
            TypingPlan::Replace {
                backspaces: 2,
                retype: "世界".to_string()
            }
        );
        // Complete rewrite keeps the common prefix only.
        assert_eq!(
            typing_plan("abc", "abd"),
            TypingPlan::Replace {
                backspaces: 1,
                retype: "d".to_string()
            }
        );
        // No common prefix at all.
        assert_eq!(
            typing_plan("xyz", "abc"),
            TypingPlan::Replace {
                backspaces: 3,
                retype: "abc".to_string()
            }
        );
    }

    #[test]
    fn plan_handles_multibyte_boundaries() {
        // Byte-prefix but char-divergent strings must not panic.
        let plan = typing_plan("é", "e");
        assert!(matches!(plan, TypingPlan::Replace { .. }));
    }
}
