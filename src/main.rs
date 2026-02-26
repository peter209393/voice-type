mod asr;
mod audio;
mod dsp;
mod overlay;
mod sway_focus;
mod typewriter;

use anyhow::{Context, Result};
use crossbeam_channel::{bounded, TryRecvError};
use dsp::{LevelMeter, Oscilloscope};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

#[derive(Clone, Debug)]
pub struct VizFrame {
    pub osc: Vec<f32>,
    pub db: f32,
    pub peak: f32,
    pub rms: f32,
}

#[derive(Clone, Debug)]
pub enum UiState {
    Idle,
    Recording,
    Transcribing { started_at: Instant },
    Typing { started_at: Instant },
    Done { text: String },
    Error { msg: String },
}

#[derive(Clone)]
struct AppConfig {
    whisper_model: PathBuf,
    whisper_bin: String,
    sample_rate_hint: Option<u32>,
}

fn main() -> Result<()> {
    let whisper_model = std::env::var("WHISPER_MODEL").unwrap_or_else(|_| {
        format!(
            "{}/.local/share/whisper/ggml-small.bin",
            std::env::var("HOME").unwrap_or_default()
        )
    });
    let whisper_bin = std::env::var("WHISPER_BIN").unwrap_or_else(|_| "whisper-cli".to_string());

    let cfg = AppConfig {
        whisper_model: PathBuf::from(whisper_model),
        whisper_bin,
        sample_rate_hint: None,
    };

    let (audio_tx, audio_rx) = bounded::<Vec<f32>>(64);
    let (cmd_tx, cmd_rx) = bounded::<overlay::OverlayCmd>(16);

    audio::set_sender(audio_tx);

    let running = Arc::new(AtomicBool::new(true));
    let running_overlay = running.clone();
    let running_main = running.clone();

    let overlay_thread = std::thread::spawn(move || {
        if let Err(e) = overlay::run_overlay(audio_rx, running_overlay, cmd_rx) {
            eprintln!("Overlay error: {:#}", e);
        }
    });

    println!("sway-voice-type overlay running");
    println!("Commands: start, stop, type, q");

    let stdin = std::io::stdin();
    let mut line = String::new();
    let mut audio_engine: Option<audio::AudioEngine> = None;
    let mut recorded: Vec<f32> = Vec::new();
    let mut meter = LevelMeter::new();
    let mut scope = Oscilloscope::new(2048);
    let mut sample_rate = 48_000u32;

    let (_record_tx, record_rx) = bounded::<Vec<f32>>(1024);

    loop {
        if !running_main.load(Ordering::SeqCst) {
            break;
        }

        line.clear();
        if stdin.read_line(&mut line).is_err() {
            break;
        }
        let cmd = line.trim();

        match cmd {
            "start" => {
                if audio_engine.is_none() {
                    recorded.clear();
                    meter.reset();
                    scope.reset();
                    match audio::AudioEngine::start_default_input(
                        cfg.sample_rate_hint,
                        record_rx.clone(),
                    ) {
                        Ok(engine) => {
                            sample_rate = engine.sample_rate();
                            audio_engine = Some(engine);
                            println!("Recording...");
                        }
                        Err(e) => {
                            eprintln!("Failed to start recording: {:#}", e);
                        }
                    }
                }
            }
            "stop" => {
                if let Some(engine) = audio_engine.take() {
                    engine.stop();
                    println!("Stopped. {} samples recorded.", recorded.len());
                }
            }
            "type" | "transcribe" => {
                if audio_engine.is_some() {
                    if let Some(engine) = audio_engine.take() {
                        engine.stop();
                    }
                }

                if recorded.is_empty() {
                    println!("No audio recorded.");
                    continue;
                }

                let tmp_dir = std::env::temp_dir();
                let wav_path = tmp_dir.join(format!("sway-voice-type-{}.wav", std::process::id()));
                audio::write_wav_f32_mono(&wav_path, sample_rate, &recorded)
                    .with_context(|| format!("Failed writing wav to {}", wav_path.display()))?;

                println!("Transcribing...");
                sway_focus::focus_prev().ok();

                match asr::transcribe_with_whisper_cli(
                    &cfg.whisper_bin,
                    &cfg.whisper_model,
                    &wav_path,
                ) {
                    Ok(text) => {
                        println!("Text: {}", text);
                        println!("Typing...");
                        if let Err(e) = typewriter::type_text_auto(&text) {
                            eprintln!("Failed to type: {:#}", e);
                        } else {
                            println!("Done.");
                        }
                    }
                    Err(e) => {
                        eprintln!("Transcription failed: {:#}", e);
                    }
                }
            }
            "q" | "quit" | "exit" => {
                if let Some(engine) = audio_engine.take() {
                    engine.stop();
                }
                running.store(false, Ordering::SeqCst);
                let _ = cmd_tx.send(overlay::OverlayCmd::Quit);
                break;
            }
            _ => {
                if !cmd.is_empty() {
                    println!("Unknown command: {}", cmd);
                }
            }
        }

        loop {
            match record_rx.try_recv() {
                Ok(chunk) => {
                    recorded.extend_from_slice(&chunk);
                    meter.push_samples(&chunk);
                    scope.push_samples(&chunk);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }
    }

    let _ = overlay_thread.join();
    Ok(())
}
