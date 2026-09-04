use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::Sender;
use std::sync::{Arc, Mutex};

use std::sync::OnceLock;

static GLOBAL_SENDERS: OnceLock<Mutex<Vec<Sender<Vec<f32>>>>> = OnceLock::new();

fn senders() -> &'static Mutex<Vec<Sender<Vec<f32>>>> {
    GLOBAL_SENDERS.get_or_init(|| Mutex::new(Vec::new()))
}

pub fn add_sender(tx: Sender<Vec<f32>>) {
    if let Ok(mut s) = senders().lock() {
        s.push(tx);
    }
}

fn send_chunk(chunk: Vec<f32>) {
    if let Ok(senders) = senders().lock() {
        for tx in senders.iter() {
            let _ = tx.try_send(chunk.clone());
        }
    }
}

pub struct AudioEngine {
    stream: cpal::Stream,
    sample_rate: u32,
    stopped: Arc<Mutex<bool>>,
}

impl AudioEngine {
    pub fn start_default_input(sample_rate_hint: Option<u32>) -> Result<Self> {
        let host = cpal::default_host();

        let device = host
            .input_devices()
            .ok()
            .and_then(|mut devices| {
                let picked = devices.find(|d| {
                    if let Ok(name) = d.name() {
                        #[cfg(target_os = "linux")]
                        {
                            name.contains("pipewire")
                                || name.contains("CARD=")
                                || name.contains("Mic")
                        }
                        #[cfg(target_os = "macos")]
                        {
                            name.contains("MacBook")
                                || name.contains("Microphone")
                                || name.contains("Built-in")
                        }
                        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
                        {
                            true
                        }
                    } else {
                        false
                    }
                });
                if std::env::var_os("VT_LOG").is_some_and(|v| !v.is_empty()) {
                    if let Ok(devs) = host.input_devices() {
                        let names: Vec<String> = devs.filter_map(|d| d.name().ok()).collect();
                        eprintln!(
                            "[vt] input devices: {:?}; picked: {:?}; default: {:?}",
                            names,
                            picked.as_ref().and_then(|d| d.name().ok()),
                            host.default_input_device().and_then(|d| d.name().ok())
                        );
                    }
                }
                picked
            })
            .or_else(|| host.default_input_device())
            .context("No input device available")?;

        let supported = device
            .default_input_config()
            .context("No default input config")?;
        let mut config: cpal::StreamConfig = supported.clone().into();

        if let Some(sr) = sample_rate_hint {
            config.sample_rate.0 = sr;
        }

        let sample_rate = config.sample_rate.0;
        let channels = config.channels as usize;

        let stopped = Arc::new(Mutex::new(false));
        let stopped_cb = stopped.clone();

        let err_fn = move |err| eprintln!("audio stream error: {err}");

        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => device.build_input_stream(
                &config,
                move |data: &[f32], _| {
                    if *stopped_cb.lock().unwrap() {
                        return;
                    }
                    let mono = interleave_to_mono_f32(data, channels);
                    send_chunk(mono);
                },
                err_fn,
                None,
            )?,
            cpal::SampleFormat::I16 => device.build_input_stream(
                &config,
                move |data: &[i16], _| {
                    if *stopped_cb.lock().unwrap() {
                        return;
                    }
                    let mono = interleave_to_mono_i16(data, channels);
                    send_chunk(mono);
                },
                err_fn,
                None,
            )?,
            cpal::SampleFormat::U16 => device.build_input_stream(
                &config,
                move |data: &[u16], _| {
                    if *stopped_cb.lock().unwrap() {
                        return;
                    }
                    let mono = interleave_to_mono_u16(data, channels);
                    send_chunk(mono);
                },
                err_fn,
                None,
            )?,
            other => anyhow::bail!("Unsupported sample format: {:?}", other),
        };

        stream.play().context("Failed to play input stream")?;

        Ok(Self {
            stream,
            sample_rate,
            stopped,
        })
    }

    pub fn stop(self) {
        if let Ok(mut s) = self.stopped.lock() {
            *s = true;
        }
        drop(self.stream);
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
}

fn interleave_to_mono_f32(data: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return data.to_vec();
    }
    let frames = data.len() / channels;
    let mut out = Vec::with_capacity(frames);
    for f in 0..frames {
        let mut acc = 0.0f32;
        for c in 0..channels {
            acc += data[f * channels + c];
        }
        out.push(acc / channels as f32);
    }
    out
}

fn interleave_to_mono_i16(data: &[i16], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return data.iter().map(|&s| s as f32 / i16::MAX as f32).collect();
    }
    let frames = data.len() / channels;
    let mut out = Vec::with_capacity(frames);
    for f in 0..frames {
        let mut acc = 0.0f32;
        for c in 0..channels {
            acc += data[f * channels + c] as f32 / i16::MAX as f32;
        }
        out.push(acc / channels as f32);
    }
    out
}

fn interleave_to_mono_u16(data: &[u16], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return data
            .iter()
            .map(|&s| (s as f32 - 32768.0) / 32768.0)
            .collect();
    }
    let frames = data.len() / channels;
    let mut out = Vec::with_capacity(frames);
    for f in 0..frames {
        let mut acc = 0.0f32;
        for c in 0..channels {
            acc += (data[f * channels + c] as f32 - 32768.0) / 32768.0;
        }
        out.push(acc / channels as f32);
    }
    out
}
