mod asr;
mod audio;
mod dsp;
mod hotkey;
mod overlay;
mod statusbar;
mod sway_focus;
mod typewriter;
mod vad;

use anyhow::{Context, Result};
use asr::StreamTranscriber;
use crossbeam_channel::{bounded, select};
use hotkey::HotkeyEvent;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use vad::VoiceActivityDetector;

const VAD_SILENCE_SECS: f32 = 1.5;
const MIN_CHUNK_SAMPLES: usize = 8000;

#[derive(Clone, Debug)]
pub enum UiState {
    Idle,
    Recording { started_at: Instant },
    Transcribing { started_at: Instant },
    Typing { started_at: Instant },
    Done { text: String },
    Error { msg: String },
}

#[derive(Clone, Debug, PartialEq)]
enum State {
    Idle,
    Recording { started_at: Instant },
}

fn main() -> Result<()> {
    let whisper_model = std::env::var("WHISPER_MODEL").unwrap_or_else(|_| {
        format!(
            "{}/.local/share/whisper/ggml-small.bin",
            std::env::var("HOME").unwrap_or_default()
        )
    });
    let whisper_bin = std::env::var("WHISPER_BIN").unwrap_or_else(|_| "whisper-cli".to_string());
    let model_path = PathBuf::from(whisper_model);

    if !model_path.exists() {
        eprintln!(
            "Warning: Whisper model not found at {}",
            model_path.display()
        );
    }

    let transcriber = Arc::new(StreamTranscriber::new(&whisper_bin, &model_path));

    let (audio_tx, audio_rx) = bounded::<Vec<f32>>(256);
    audio::add_sender(audio_tx);

    let running = Arc::new(AtomicBool::new(true));

    let hotkey_rx = hotkey::start_hotkey_listener()
        .context("Failed to start hotkey listener. Make sure you have input group permissions.")?;

    let (cmd_tx, cmd_rx) = bounded::<overlay::OverlayCmd>(16);

    let (_overlay_audio_tx, overlay_audio_rx) = bounded::<Vec<f32>>(64);
    let overlay_running = running.clone();
    std::thread::spawn(move || {
        if let Err(e) = overlay::run_overlay(overlay_audio_rx, overlay_running, cmd_rx) {
            eprintln!("Overlay error: {:#}", e);
        }
    });

    let (status_tx, status_rx) = bounded::<statusbar::StatusBarCmd>(16);
    let status_running = running.clone();
    std::thread::spawn(move || {
        if let Err(e) = statusbar::run_status_bar(status_running, status_rx) {
            eprintln!("Status bar error: {:#}", e);
        }
    });

    // Initialize status bar as idle
    let _ = status_tx.send(statusbar::StatusBarCmd::UpdateState(UiState::Idle));

    println!("sway-voice-type stream mode");
    println!("Hold Right Alt key to record, release to stop.");
    println!("Audio will be transcribed in chunks during silence.");

    let mut state = State::Idle;
    let mut buffer: Vec<f32> = Vec::new();
    let mut vad = VoiceActivityDetector::new(VAD_SILENCE_SECS);
    let mut sample_rate: u32 = 44_100;
    let mut audio_engine: Option<audio::AudioEngine> = None;

    loop {
        if !running.load(Ordering::SeqCst) {
            break;
        }

        select! {
            recv(hotkey_rx) -> event => {
                match event {
                    Ok(HotkeyEvent::Pressed) => {
                        if state == State::Idle {
                            println!("[Recording started]");
                            buffer.clear();
                            vad.reset();
                            let _ = status_tx.send(statusbar::StatusBarCmd::UpdateState(UiState::Recording { started_at: Instant::now() }));

                            match audio::AudioEngine::start_default_input(None) {
                                Ok(engine) => {
                                    sample_rate = engine.sample_rate();
                                    audio_engine = Some(engine);
                                    state = State::Recording { started_at: Instant::now() };
                                }
                                Err(e) => {
                                    eprintln!("Failed to start recording: {:#}", e);
                                    let _ = status_tx.send(statusbar::StatusBarCmd::UpdateState(UiState::Error { msg: e.to_string() }));
                                }
                            }
                        }
                    }
                    Ok(HotkeyEvent::Released) => {
                        if state != State::Idle {
                            if let Some(engine) = audio_engine.take() {
                                engine.stop();
                            }

                            while let Ok(chunk) = audio_rx.try_recv() {
                                buffer.extend_from_slice(&chunk);
                            }

                            if buffer.len() >= MIN_CHUNK_SAMPLES {
                                println!("[Transcribing final chunk...]");
                                let _ = status_tx.send(statusbar::StatusBarCmd::UpdateState(UiState::Transcribing { started_at: Instant::now() }));
                                match transcriber.transcribe_chunk(&buffer, sample_rate) {
                                    Ok(text) if !text.trim().is_empty() => {
                                        println!("> {}", text.trim());
                                        let _ = status_tx.send(statusbar::StatusBarCmd::UpdateState(UiState::Typing { started_at: Instant::now() }));
                                        if let Err(e) = typewriter::type_text_auto(&text) {
                                            eprintln!("Failed to type: {:#}", e);
                                            let _ = status_tx.send(statusbar::StatusBarCmd::UpdateState(UiState::Error { msg: e.to_string() }));
                                        }
                                    }
                                    Ok(text) => {
                                        let _ = status_tx.send(statusbar::StatusBarCmd::UpdateState(UiState::Done { text }));
                                    }
                                    Err(e) => {
                                        eprintln!("Transcription error: {:#}", e);
                                        let _ = status_tx.send(statusbar::StatusBarCmd::UpdateState(UiState::Error { msg: e.to_string() }));
                                    }
                                }
                            }

                            buffer.clear();
                            state = State::Idle;
                            println!("[Recording stopped]");
                            let _ = status_tx.send(statusbar::StatusBarCmd::UpdateState(UiState::Idle));
                        }
                    }
                    Err(_) => {
                        eprintln!("Hotkey channel closed");
                        break;
                    }
                }
            }

            recv(audio_rx) -> chunk => {
                if let Ok(chunk) = chunk {
                    if let State::Recording { .. } = state {
                        buffer.extend_from_slice(&chunk);
                        vad.push_samples(&chunk);

                        if vad.is_silence_timeout() && buffer.len() >= MIN_CHUNK_SAMPLES {
                            println!("[VAD: silence detected, transcribing chunk...]");
                            let _ = status_tx.send(statusbar::StatusBarCmd::UpdateState(UiState::Transcribing { started_at: Instant::now() }));

                            let chunk_samples: Vec<f32> = buffer.drain(..).collect();
                            vad.reset();

                            let sr = sample_rate;
                            let transcriber = Arc::clone(&transcriber);
                            let status_tx_clone = status_tx.clone();
                            std::thread::spawn(move || {
                                match transcriber.transcribe_chunk(&chunk_samples, sr) {
                                    Ok(text) if !text.trim().is_empty() => {
                                        println!("> {}", text.trim());
                                        let _ = status_tx_clone.send(statusbar::StatusBarCmd::UpdateState(UiState::Typing { started_at: Instant::now() }));
                                        if let Err(e) = typewriter::type_text_auto(&text) {
                                            eprintln!("Failed to type: {:#}", e);
                                            let _ = status_tx_clone.send(statusbar::StatusBarCmd::UpdateState(UiState::Error { msg: e.to_string() }));
                                        }
                                    }
                                    Ok(text) => {
                                        let _ = status_tx_clone.send(statusbar::StatusBarCmd::UpdateState(UiState::Done { text }));
                                    }
                                    Err(e) => {
                                        eprintln!("Transcription error: {:#}", e);
                                        let _ = status_tx_clone.send(statusbar::StatusBarCmd::UpdateState(UiState::Error { msg: e.to_string() }));
                                    }
                                }
                            });
                        }
                    }
                }
            }

            default(Duration::from_millis(10)) => {
                if state == State::Idle {
                    while let Ok(_) = audio_rx.try_recv() {}
                }
            }
        }
    }

    let _ = cmd_tx.send(overlay::OverlayCmd::Quit);
    let _ = status_tx.send(statusbar::StatusBarCmd::Quit);
    running.store(false, Ordering::SeqCst);

    Ok(())
}
