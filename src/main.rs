mod asr;
mod audio;
mod correct;
mod hotkey;
mod popup;
mod tray;
mod typewriter;

use anyhow::{Context, Result};
use asr::StreamTranscriber;
use correct::Corrector;
use crossbeam_channel::{bounded, select};
use hotkey::HotkeyEvent;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const MIN_CHUNK_SAMPLES: usize = 8000;

#[derive(Clone, Debug)]
pub enum UiState {
    Idle,
    Recording { started_at: Instant },
    Transcribing { started_at: Instant },
    Done { text: String },
    Error { msg: String },
}

fn main() -> Result<()> {
    #[cfg(target_os = "linux")]
    popup::init_gtk()?;

    #[cfg(target_os = "macos")]
    popup::init_popup()?;

    let dummy_path = PathBuf::from("/dev/null");
    let transcriber = Arc::new(StreamTranscriber::new("", &dummy_path));
    let corrector = Arc::new(Corrector::new());

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

    loop {
        if !running.load(Ordering::SeqCst) {
            break;
        }

        #[cfg(target_os = "linux")]
        popup::process_events();

        select! {
            recv(hotkey_rx) -> event => {
                match event {
                    Ok(HotkeyEvent::Pressed) => {
                        if !is_recording {
                            buffer.clear();
                            let _ = cmd_tx.send(tray::TrayCmd::UpdateState(UiState::Recording {
                                started_at: Instant::now()
                            }));

                            let (cx, cy) = popup::get_cursor_position();
                            popup::show_popup(cx, cy);

                            match audio::AudioEngine::start_default_input(None) {
                                Ok(engine) => {
                                    sample_rate = engine.sample_rate();
                                    audio_engine = Some(engine);
                                    is_recording = true;
                                }
                                Err(e) => {
                                    eprintln!("Failed to start recording: {:#}", e);
                                    let _ = cmd_tx.send(tray::TrayCmd::UpdateState(UiState::Error { msg: e.to_string() }));
                                    popup::hide_popup();
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

                            popup::hide_popup();

                            if buffer.len() >= MIN_CHUNK_SAMPLES {
                                let _ = cmd_tx.send(tray::TrayCmd::UpdateState(UiState::Transcribing { started_at: Instant::now() }));

                                let samples = buffer.clone();
                                let sr = sample_rate;
                                let transcriber = Arc::clone(&transcriber);
                                let corrector = Arc::clone(&corrector);
                                let cmd_tx_clone = cmd_tx.clone();

                                std::thread::spawn(move || {
                                    match transcriber.transcribe_chunk(&samples, sr) {
                                        Ok(raw) if !raw.trim().is_empty() => {
                                            // 1. Type the raw ASR text immediately for fast
                                            //    feedback, and remember how many chars we
                                            //    emitted so we can backspace over them later.
                                            if let Err(e) = typewriter::type_text_auto(&raw) {
                                                eprintln!("Failed to type raw text: {:#}", e);
                                            }
                                            let raw_chars = raw.chars().count();
                                            let _ = cmd_tx_clone.send(tray::TrayCmd::UpdateState(
                                                UiState::Done { text: raw.clone() },
                                            ));

                                            // 2. Background-correct with the small LLM and, if
                                            //    it differs, backspace over the raw text and
                                            //    type the corrected version.
                                            if corrector.enabled() {
                                                eprintln!("[vt] raw ({raw_chars} chars): {:?}", raw);
                                                match corrector.correct(&raw) {
                                                    Ok(corrected) => {
                                                        eprintln!("[vt] corrected: {:?}", corrected);
                                                        if corrected.trim().is_empty() {
                                                            eprintln!("[vt] corrected empty, skip replace");
                                                        } else if corrected.trim() == raw.trim() {
                                                            eprintln!("[vt] corrected == raw, skip replace");
                                                        } else {
                                                            eprintln!("[vt] replacing: backspace({}) then type", raw_chars);
                                                            if let Err(e) =
                                                                typewriter::backspace(raw_chars)
                                                            {
                                                                eprintln!(
                                                                    "[vt] backspace FAILED: {:#}",
                                                                    e
                                                                );
                                                            }
                                                            if let Err(e) = typewriter::type_text_auto(
                                                                &corrected,
                                                            ) {
                                                                eprintln!(
                                                                    "[vt] type corrected FAILED: {:#}",
                                                                    e
                                                                );
                                                            }
                                                            let _ = cmd_tx_clone.send(
                                                                tray::TrayCmd::UpdateState(
                                                                    UiState::Done {
                                                                        text: corrected,
                                                                    },
                                                                ),
                                                            );
                                                        }
                                                    }
                                                    Err(e) => {
                                                        eprintln!("[vt] correction error: {:#}", e);
                                                    }
                                                }
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
                    }
                }
            }

            default(Duration::from_millis(50)) => {
                if !is_recording {
                    while audio_rx.try_recv().is_ok() {}
                }
            }
        }
    }

    let _ = cmd_tx.send(tray::TrayCmd::Quit);
    running.store(false, Ordering::SeqCst);

    Ok(())
}
