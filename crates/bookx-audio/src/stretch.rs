//! Speed change that keeps the pitch: WSOLA (waveform similarity overlap-add). Streaming: push
//! interleaved samples, get interleaved samples; at speed 1.0 the samples pass through unchanged.
//! Speech at 0.5x to 3x sounds natural; music would show the artefacts, which is fine for books.

pub const MIN_SPEED: f32 = 0.5;
pub const MAX_SPEED: f32 = 3.0;

pub fn clamp_speed(s: f32) -> f32 {
    if s.is_nan() {
        1.0
    } else {
        s.clamp(MIN_SPEED, MAX_SPEED)
    }
}

pub struct Stretcher {
    ch: usize,
    speed: f32,
    n: usize,
    hs: usize,
    delta: usize,
    window: Vec<f32>,
    /// Input frames not yet dropped; `base` is the absolute frame index of `input[0]`.
    input: Vec<f32>,
    base: u64,
    /// Ideal analysis position of the next window (absolute frame).
    next_in: f64,
    prev_start: Option<u64>,
    /// Overlap-add accumulator, `n` frames.
    ola: Vec<f32>,
    total_in: u64,
}

impl Stretcher {
    pub fn new(rate: u32, channels: u8, speed: f32) -> Stretcher {
        let ch = channels.max(1) as usize;
        // a 40 ms window, even so that the hop is half of it
        let n = ((rate as usize * 40 / 1000) & !1).max(64);
        let window = (0..n)
            .map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / n as f32).cos())
            .collect();
        Stretcher {
            ch,
            speed: clamp_speed(speed),
            n,
            hs: n / 2,
            delta: (rate as usize * 10 / 1000).max(16),
            window,
            input: Vec::new(),
            base: 0,
            next_in: 0.0,
            prev_start: None,
            ola: vec![0.0; n * ch],
            total_in: 0,
        }
    }

    pub fn speed(&self) -> f32 {
        self.speed
    }

    /// Changes the speed from the next window on.
    pub fn set_speed(&mut self, speed: f32) {
        self.speed = clamp_speed(speed);
    }

    /// Forgets everything buffered (after a seek).
    pub fn reset(&mut self) {
        self.input.clear();
        self.base = 0;
        self.next_in = 0.0;
        self.prev_start = None;
        self.ola.iter_mut().for_each(|x| *x = 0.0);
        self.total_in = 0;
    }

    fn avail(&self) -> u64 {
        self.base + (self.input.len() / self.ch) as u64
    }

    fn mono(&self, abs: u64) -> f32 {
        let i = (abs - self.base) as usize * self.ch;
        self.input[i..i + self.ch].iter().sum::<f32>() / self.ch as f32
    }

    /// Adds `samples` and appends what can be produced to `out`.
    pub fn push(&mut self, samples: &[f32], out: &mut Vec<f32>) {
        if (self.speed - 1.0).abs() < 1e-3 && self.prev_start.is_none() && self.input.is_empty() {
            out.extend_from_slice(samples);
            return;
        }
        let whole = samples.len() / self.ch * self.ch;
        self.input.extend_from_slice(&samples[..whole]);
        self.total_in += (whole / self.ch) as u64;
        self.run(out);
    }

    /// Produces the tail after the last `push` (the input is padded with silence).
    pub fn finish(&mut self, out: &mut Vec<f32>) {
        if self.prev_start.is_none() && self.input.is_empty() {
            return;
        }
        let pad = vec![0.0; (self.n + self.delta * 2 + self.hs) * self.ch];
        self.input.extend_from_slice(&pad);
        self.run(out);
        self.reset();
    }

    fn run(&mut self, out: &mut Vec<f32>) {
        let (n, hs, ch) = (self.n, self.hs, self.ch);
        loop {
            let first = self.prev_start.is_none();
            let target = self.next_in.round() as u64;
            let lo = target.saturating_sub(self.delta as u64).max(self.base);
            let hi = target + self.delta as u64;
            // need the whole window at the furthest candidate, and the template
            if self.avail() < hi + n as u64 {
                return;
            }
            let start = if first {
                0
            } else {
                let prev = self.prev_start.unwrap_or(0);
                let tpl = prev + hs as u64;
                if tpl < self.base || self.avail() < tpl + hs as u64 {
                    return;
                }
                self.best_match(tpl, lo, hi)
            };
            if start < self.base {
                // cannot happen: `lo` is clamped; keep going safely
                self.next_in = self.base as f64;
                continue;
            }
            // overlap-add the windowed segment
            let off = (start - self.base) as usize * ch;
            for i in 0..n {
                let w = self.window[i];
                for c in 0..ch {
                    self.ola[i * ch + c] += self.input[off + i * ch + c] * w;
                }
            }
            out.extend_from_slice(&self.ola[..hs * ch]);
            self.ola.drain(..hs * ch);
            self.ola.extend(std::iter::repeat_n(0.0, hs * ch));
            self.prev_start = Some(start);
            self.next_in += hs as f64 * self.speed as f64;
            // drop what is no longer needed
            let keep_from = (start + hs as u64)
                .min(self.next_in as u64 - self.delta.min(self.next_in as usize) as u64);
            if keep_from > self.base {
                let drop = ((keep_from - self.base) as usize * ch).min(self.input.len());
                self.input.drain(..drop);
                self.base += (drop / ch) as u64;
            }
        }
    }

    /// The candidate start in `lo..=hi` whose first `hs` frames are most like the template.
    fn best_match(&self, tpl: u64, lo: u64, hi: u64) -> u64 {
        let hs = self.hs;
        let step = 4usize;
        let mut best = (f32::NEG_INFINITY, (lo + hi) / 2);
        let tpl_v: Vec<f32> = (0..hs)
            .step_by(step)
            .map(|i| self.mono(tpl + i as u64))
            .collect();
        let mut cand = lo;
        while cand <= hi {
            let mut dot = 0.0f32;
            let mut energy = 1e-9f32;
            for (k, t) in tpl_v.iter().enumerate() {
                let x = self.mono(cand + (k * step) as u64);
                dot += x * t;
                energy += x * x;
            }
            let score = dot / energy.sqrt();
            if score > best.0 {
                best = (score, cand);
            }
            cand += 1;
        }
        best.1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(rate: u32, secs: f32, hz: f32) -> Vec<f32> {
        (0..(rate as f32 * secs) as usize)
            .map(|i| (i as f32 / rate as f32 * hz * std::f32::consts::TAU).sin() * 0.5)
            .collect()
    }

    fn crossings(v: &[f32]) -> usize {
        v.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count()
    }

    fn run(speed: f32, input: &[f32], rate: u32, chunk: usize) -> Vec<f32> {
        let mut s = Stretcher::new(rate, 1, speed);
        let mut out = Vec::new();
        for c in input.chunks(chunk) {
            s.push(c, &mut out);
        }
        s.finish(&mut out);
        out
    }

    #[test]
    fn speed_one_passes_the_samples_through() {
        let x = tone(8000, 1.0, 300.0);
        assert_eq!(run(1.0, &x, 8000, 500), x);
    }

    #[test]
    fn the_length_follows_the_speed_and_the_pitch_stays() {
        let rate = 16000;
        let x = tone(rate, 4.0, 220.0);
        for speed in [0.5f32, 0.75, 1.5, 2.0, 3.0] {
            let y = run(speed, &x, rate, 1000);
            let want = x.len() as f32 / speed;
            let ratio = y.len() as f32 / want;
            assert!(
                (0.9..=1.12).contains(&ratio),
                "speed {speed}: {} samples, wanted about {want}",
                y.len()
            );
            // the pitch: zero crossings per second of output stay near 220
            let hz = crossings(&y[2000..y.len().saturating_sub(2000)]) as f32
                / ((y.len() - 4000) as f32 / rate as f32);
            assert!((hz - 220.0).abs() < 12.0, "speed {speed}: {hz} Hz");
        }
    }

    #[test]
    fn stereo_channels_stay_in_step() {
        let rate = 16000;
        let m = tone(rate, 2.0, 300.0);
        let mut st: Vec<f32> = Vec::new();
        for s in &m {
            st.push(*s);
            st.push(-*s);
        }
        let mut s = Stretcher::new(rate, 2, 1.7);
        let mut out = Vec::new();
        for c in st.chunks(2000) {
            s.push(c, &mut out);
        }
        s.finish(&mut out);
        assert_eq!(out.len() % 2, 0);
        assert!(out.chunks(2).all(|f| (f[0] + f[1]).abs() < 1e-4));
    }

    #[test]
    fn chunk_size_does_not_matter() {
        let x = tone(8000, 2.0, 200.0);
        let a = run(2.0, &x, 8000, 100);
        let b = run(2.0, &x, 8000, 7777);
        assert_eq!(a.len(), b.len());
        assert!(a.iter().zip(&b).all(|(p, q)| (p - q).abs() < 1e-5));
    }

    #[test]
    fn speed_is_clamped_and_changes_midway() {
        assert_eq!(clamp_speed(10.0), 3.0);
        assert_eq!(clamp_speed(0.0), 0.5);
        assert_eq!(clamp_speed(f32::NAN), 1.0);
        let x = tone(8000, 3.0, 200.0);
        let mut s = Stretcher::new(8000, 1, 1.0);
        let mut out = Vec::new();
        s.set_speed(2.0);
        s.push(&x[..12000], &mut out);
        s.set_speed(1.0 / 2.0);
        s.push(&x[12000..], &mut out);
        s.finish(&mut out);
        assert!(!out.is_empty());
        assert!(out.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn empty_and_tiny_input_is_fine() {
        let mut s = Stretcher::new(8000, 1, 2.0);
        let mut out = Vec::new();
        s.push(&[], &mut out);
        s.push(&[0.1; 3], &mut out);
        s.finish(&mut out);
        s.finish(&mut out);
        assert!(out.iter().all(|v| v.is_finite()));
    }
}
