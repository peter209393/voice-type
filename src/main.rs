mod asr;
mod audio;
mod hotkey;
mod tray;
mod typewriter;

use anyhow::{Context, Result};
use asr::StreamTranscriber;
use crossbeam_channel::{bounded, select};
use hotkey::HotkeyEvent;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const MIN_CHUNK_SAMPLES: usize = 8000;
const TRANSCRIBE_INTERVAL_SECS: f64 = 0.5;

#[derive(Clone, Debug)]
pub enum UiState {
    Idle,
    Recording { started_at: Instant, text: String },
    Transcribing { started_at: Instant },
    Done { text: String },
    Error { msg: String },
}

fn main() -> Result<()> {
    // Check for required ZAI_API_TOKEN environment variable
    if std::env::var("ZAI_API_TOKEN").is_err() {
        eprintln!("Error: ZAI_API_TOKEN environment variable must be set");
        eprintln!("Get your API key from https://z.ai");
        std::process::exit(1);
    }

    // Dummy path - not used for API-based transcription
    let dummy_path = PathBuf::from("/dev/null");
    let transcriber = Arc::new(StreamTranscriber::new("", &dummy_path));

    let (audio_tx, audio_rx) = bounded::<Vec<f32>>(256);
    audio::add_sender(audio_tx);

    let running = Arc::new(AtomicBool::new(true));

    let hotkey_rx = hotkey::start_hotkey_listener()
        .context("Failed to start hotkey listener. Make sure you have input group permissions.")?;

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
    let mut last_transcribe_time: Option<Instant> = None;
    let typed_text: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    let mut is_recording = false;

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
                            typed_text.lock().unwrap().clear();
                            last_transcribe_time = Some(Instant::now());
                            let _ = cmd_tx.send(tray::TrayCmd::UpdateState(UiState::Recording {
                                started_at: Instant::now(),
                                text: String::new()
                            }));

                            match audio::AudioEngine::start_default_input(None) {
                                Ok(engine) => {
                                    sample_rate = engine.sample_rate();
                                    audio_engine = Some(engine);
                                    is_recording = true;
                                }
                                Err(e) => {
                                    eprintln!("Failed to start recording: {:#}", e);
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
                            }

                            if buffer.len() >= MIN_CHUNK_SAMPLES {
                                let _ = cmd_tx.send(tray::TrayCmd::UpdateState(UiState::Transcribing { started_at: Instant::now() }));
                                match transcriber.transcribe_chunk(&buffer, sample_rate) {
                                    Ok(text) if !text.trim().is_empty() => {
                                        if let Err(e) = typewriter::type_text_auto(&text) {
                                            eprintln!("Failed to type: {:#}", e);
                                        }
                                    }
                                    Ok(_) => {}
                                    Err(e) => {
                                        eprintln!("Transcription error: {:#}", e);
                                    }
                                }
                                let _ = cmd_tx.send(tray::TrayCmd::UpdateState(UiState::Idle));
                            } else {
                                let _ = cmd_tx.send(tray::TrayCmd::UpdateState(UiState::Idle));
                            }

                            buffer.clear();
                            is_recording = false;
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

                        if let Some(last_time) = last_transcribe_time {
                            let elapsed = last_time.elapsed().as_secs_f64();
                            if elapsed >= TRANSCRIBE_INTERVAL_SECS && buffer.len() >= MIN_CHUNK_SAMPLES {
                                let chunk_samples: Vec<f32> = buffer.drain(..).collect();
                                let sr = sample_rate;
                                let transcriber = Arc::clone(&transcriber);
                                let cmd_tx_clone = cmd_tx.clone();
                                let typed_text_clone = Arc::clone(&typed_text);

                                std::thread::spawn(move || {
                                    match transcriber.transcribe_chunk(&chunk_samples, sr) {
                                        Ok(text) if !text.trim().is_empty() => {
                                            if let Err(e) = typewriter::type_text_auto(&text) {
                                                eprintln!("Failed to type: {:#}", e);
                                            }
                                            let mut tt = typed_text_clone.lock().unwrap();
                                            tt.push_str(&text);
                                            let _ = cmd_tx_clone.send(tray::TrayCmd::UpdateState(UiState::Recording {
                                                started_at: Instant::now(),
                                                text: tt.clone()
                                            }));
                                        }
                                        _ => {}
                                    }
                                });

                                last_transcribe_time = Some(Instant::now());
                            }
                        }
                    }
                }
            }

            default(Duration::from_millis(50)) => {
                if !is_recording {
                    while let Ok(_) = audio_rx.try_recv() {}
                }
            }
        }
    }

    let _ = cmd_tx.send(tray::TrayCmd::Quit);
    running.store(false, Ordering::SeqCst);

    Ok(())
}
