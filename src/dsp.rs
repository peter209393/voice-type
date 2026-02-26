pub struct LevelMeter {
    rms_smooth: f32,
    peak_smooth: f32,
}

impl LevelMeter {
    pub fn new() -> Self {
        Self {
            rms_smooth: 0.0,
            peak_smooth: 0.0,
        }
    }

    pub fn reset(&mut self) {
        self.rms_smooth = 0.0;
        self.peak_smooth = 0.0;
    }

    pub fn push_samples(&mut self, samples: &[f32]) {
        if samples.is_empty() {
            return;
        }

        let mut sum = 0.0f32;
        let mut peak = 0.0f32;

        for &s in samples {
            let a = s.abs();
            peak = peak.max(a);
            sum += s * s;
        }

        let rms = (sum / samples.len() as f32).sqrt();

        // simple attack/release smoothing (tweakable)
        self.rms_smooth = smooth(self.rms_smooth, rms, 0.35, 0.06);
        self.peak_smooth = smooth(self.peak_smooth, peak, 0.55, 0.12);
    }

    pub fn current(&self) -> (f32, f32, f32) {
        let rms = self.rms_smooth.clamp(0.0, 1.0);
        let peak = self.peak_smooth.clamp(0.0, 1.0);
        let db = amp_to_dbfs(rms);
        (rms, peak, db)
    }
}

fn smooth(prev: f32, next: f32, attack: f32, release: f32) -> f32 {
    if next > prev {
        prev + (next - prev) * attack
    } else {
        prev + (next - prev) * release
    }
}

fn amp_to_dbfs(amp: f32) -> f32 {
    let a = amp.max(1e-9);
    20.0 * a.log10()
}

pub struct Oscilloscope {
    buf: Vec<f32>,
    write_idx: usize,
    filled: bool,
}

impl Oscilloscope {
    pub fn new(capacity: usize) -> Self {
        Self {
            buf: vec![0.0; capacity],
            write_idx: 0,
            filled: false,
        }
    }

    pub fn reset(&mut self) {
        self.buf.fill(0.0);
        self.write_idx = 0;
        self.filled = false;
    }

    pub fn push_samples(&mut self, samples: &[f32]) {
        for &s in samples {
            self.buf[self.write_idx] = s;
            self.write_idx = (self.write_idx + 1) % self.buf.len();
            if self.write_idx == 0 {
                self.filled = true;
            }
        }
    }

    pub fn render_points(&self, points: usize) -> Vec<f32> {
        let n = points.max(2);
        let available = if self.filled {
            self.buf.len()
        } else {
            self.write_idx.max(1)
        };
        let mut out = Vec::with_capacity(n);

        // take most recent window and downsample
        for i in 0..n {
            let t = i as f32 / (n - 1) as f32;
            let idx_from_end = ((1.0 - t) * (available as f32 - 1.0)) as usize;
            let idx = self.index_recent(idx_from_end);
            out.push(self.buf[idx]);
        }
        out.reverse(); // left->right chronological
        out
    }

    fn index_recent(&self, idx_from_end: usize) -> usize {
        // idx_from_end=0 => newest sample just before write_idx
        let len = self.buf.len();
        let newest = (self.write_idx + len - 1) % len;
        (newest + len - (idx_from_end % len)) % len
    }
}
