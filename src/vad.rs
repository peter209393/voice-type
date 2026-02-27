use std::time::Instant;

pub struct VoiceActivityDetector {
    silence_threshold: f32,
    silence_duration_ms: u64,
    last_voice_time: Instant,
    in_silence: bool,
}

impl VoiceActivityDetector {
    pub fn new(silence_duration_secs: f32) -> Self {
        Self {
            silence_threshold: 0.01,
            silence_duration_ms: (silence_duration_secs * 1000.0) as u64,
            last_voice_time: Instant::now(),
            in_silence: false,
        }
    }

    pub fn push_samples(&mut self, samples: &[f32]) {
        let rms = compute_rms(samples);
        if rms > self.silence_threshold {
            self.last_voice_time = Instant::now();
            self.in_silence = false;
        } else if !self.in_silence {
            self.in_silence = true;
        }
    }

    pub fn is_silence_timeout(&self) -> bool {
        if !self.in_silence {
            return false;
        }
        self.last_voice_time.elapsed().as_millis() as u64 >= self.silence_duration_ms
    }

    pub fn reset(&mut self) {
        self.last_voice_time = Instant::now();
        self.in_silence = false;
    }

    pub fn set_threshold(&mut self, threshold: f32) {
        self.silence_threshold = threshold;
    }
}

fn compute_rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f32 = samples.iter().map(|s| s * s).sum();
    (sum / samples.len() as f32).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_silence_detection() {
        let mut vad = VoiceActivityDetector::new(0.1);

        let silence: Vec<f32> = vec![0.0; 1000];
        vad.push_samples(&silence);
        assert!(!vad.is_silence_timeout());

        std::thread::sleep(std::time::Duration::from_millis(150));
        assert!(vad.is_silence_timeout());
    }

    #[test]
    fn test_voice_resets_silence() {
        let mut vad = VoiceActivityDetector::new(0.1);

        let silence: Vec<f32> = vec![0.0; 1000];
        vad.push_samples(&silence);

        std::thread::sleep(std::time::Duration::from_millis(50));

        let voice: Vec<f32> = (0..1000).map(|i| (i as f32 / 1000.0 - 0.5) * 0.5).collect();
        vad.push_samples(&voice);

        assert!(!vad.is_silence_timeout());
    }
}
