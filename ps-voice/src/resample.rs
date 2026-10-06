const TAPS: usize = 32;
const HALF: usize = TAPS / 2;
const PHASES: usize = 128;

pub struct Resampler {
    channels: usize,
    step: f64,
    pos: f64,
    buf: Vec<f32>,
    table: Vec<f32>,
    passthrough: bool,
}

fn sinc(x: f64) -> f64 {
    if x.abs() < 1e-9 {
        1.0
    } else {
        let px = std::f64::consts::PI * x;
        px.sin() / px
    }
}

fn window(t: f64) -> f64 {
    let x = t / HALF as f64;
    if x.abs() >= 1.0 {
        return 0.0;
    }
    let a = std::f64::consts::PI * x;
    0.42 + 0.5 * a.cos() + 0.08 * (2.0 * a).cos()
}

fn build_table(cutoff: f64) -> Vec<f32> {
    let mut table = vec![0f32; (PHASES + 1) * TAPS];
    for phase in 0..=PHASES {
        let frac = phase as f64 / PHASES as f64;
        let mut row = [0f64; TAPS];
        let mut sum = 0.0;
        for (k, slot) in row.iter_mut().enumerate() {
            let t = (k as f64 - (HALF as f64 - 1.0)) - frac;
            let v = 2.0 * cutoff * sinc(2.0 * cutoff * t) * window(t);
            *slot = v;
            sum += v;
        }
        for (k, v) in row.iter().enumerate() {
            table[phase * TAPS + k] = (v / sum) as f32;
        }
    }
    table
}

impl Resampler {
    pub fn new(in_rate: u32, out_rate: u32, channels: usize) -> Self {
        let channels = channels.max(1);
        let in_rate = in_rate.max(1);
        let out_rate = out_rate.max(1);
        let passthrough = in_rate == out_rate;
        let ratio = out_rate as f64 / in_rate as f64;
        let cutoff = 0.5 * ratio.min(1.0) * 0.94;
        let table = if passthrough { Vec::new() } else { build_table(cutoff) };
        Self {
            channels,
            step: in_rate as f64 / out_rate as f64,
            pos: (HALF - 1) as f64,
            buf: vec![0.0; (HALF - 1) * channels],
            table,
            passthrough,
        }
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    pub fn reset(&mut self) {
        self.pos = (HALF - 1) as f64;
        self.buf.clear();
        self.buf.resize((HALF - 1) * self.channels, 0.0);
    }

    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        if self.passthrough {
            out.extend_from_slice(input);
            return;
        }
        let ch = self.channels;
        self.buf.extend_from_slice(input);
        let frames = self.buf.len() / ch;
        loop {
            let base = self.pos.floor();
            let i = base as usize;
            if i + HALF >= frames {
                break;
            }
            let frac = self.pos - base;
            let pf = frac * PHASES as f64;
            let p0 = (pf.floor() as usize).min(PHASES - 1);
            let blend = (pf - p0 as f64) as f32;
            let row0 = &self.table[p0 * TAPS..(p0 + 1) * TAPS];
            let row1 = &self.table[(p0 + 1) * TAPS..(p0 + 2) * TAPS];
            let start = (i + 1 - HALF) * ch;
            for c in 0..ch {
                let mut acc = 0f32;
                for k in 0..TAPS {
                    let coeff = row0[k] + (row1[k] - row0[k]) * blend;
                    acc += self.buf[start + k * ch + c] * coeff;
                }
                out.push(acc);
            }
            self.pos += self.step;
        }
        let keep_from = (self.pos.floor() as usize).saturating_sub(HALF - 1);
        if keep_from > 0 {
            let drop_frames = keep_from.min(self.buf.len() / ch);
            self.buf.drain(..drop_frames * ch);
            self.pos -= drop_frames as f64;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(rate: u32, freq: f64, frames: usize, channels: usize) -> Vec<f32> {
        let mut out = Vec::with_capacity(frames * channels);
        for i in 0..frames {
            let v = (0.5 * (2.0 * std::f64::consts::PI * freq * i as f64 / rate as f64).sin()) as f32;
            for c in 0..channels {
                out.push(if c == 0 { v } else { -v });
            }
        }
        out
    }

    fn rms(x: &[f32]) -> f64 {
        (x.iter().map(|v| (*v as f64) * (*v as f64)).sum::<f64>() / x.len().max(1) as f64).sqrt()
    }

    fn frequency(mono: &[f32], rate: u32) -> f64 {
        let mut crossings = 0usize;
        for w in mono.windows(2) {
            if w[0] <= 0.0 && w[1] > 0.0 {
                crossings += 1;
            }
        }
        crossings as f64 * rate as f64 / mono.len() as f64
    }

    fn run(in_rate: u32, out_rate: u32, freq: f64, chunk: usize) -> Vec<f32> {
        let input = sine(in_rate, freq, in_rate as usize, 1);
        let mut r = Resampler::new(in_rate, out_rate, 1);
        let mut out = Vec::new();
        for part in input.chunks(chunk) {
            r.process(part, &mut out);
        }
        out
    }

    #[test]
    fn passthrough_is_exact() {
        let input = sine(48_000, 440.0, 1000, 2);
        let mut r = Resampler::new(48_000, 48_000, 2);
        let mut out = Vec::new();
        r.process(&input, &mut out);
        assert_eq!(out, input);
    }

    #[test]
    fn upsamples_44100_to_48000() {
        let out = run(44_100, 48_000, 1000.0, 441);
        assert!((out.len() as i64 - 48_000).abs() < 64, "len {}", out.len());
        let body = &out[2000..out.len() - 2000];
        assert!((rms(body) - 0.35355).abs() < 0.004, "rms {}", rms(body));
        assert!((frequency(body, 48_000) - 1000.0).abs() < 3.0);
    }

    #[test]
    fn downsamples_96000_to_48000() {
        let out = run(96_000, 48_000, 3000.0, 1000);
        assert!((out.len() as i64 - 48_000).abs() < 64, "len {}", out.len());
        let body = &out[2000..out.len() - 2000];
        assert!((rms(body) - 0.35355).abs() < 0.004, "rms {}", rms(body));
        assert!((frequency(body, 48_000) - 3000.0).abs() < 5.0);
    }

    #[test]
    fn downsampling_removes_content_above_nyquist() {
        let out = run(96_000, 48_000, 36_000.0, 960);
        let body = &out[2000..out.len() - 2000];
        assert!(rms(body) < 0.005, "aliased energy {}", rms(body));
    }

    #[test]
    fn handles_odd_rates_and_tiny_chunks() {
        let a = run(48_000, 44_100, 500.0, 7);
        let b = run(48_000, 44_100, 500.0, 4800);
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(b.iter()) {
            assert!((x - y).abs() < 1e-5);
        }
        let low = run(16_000, 48_000, 400.0, 160);
        assert!((low.len() as i64 - 48_000).abs() < 128);
        let body = &low[4000..low.len() - 4000];
        assert!((rms(body) - 0.35355).abs() < 0.004, "rms {}", rms(body));
    }

    #[test]
    fn keeps_channels_separate() {
        let input = sine(44_100, 800.0, 44_100, 2);
        let mut r = Resampler::new(44_100, 48_000, 2);
        let mut out = Vec::new();
        for part in input.chunks(882) {
            r.process(part, &mut out);
        }
        assert_eq!(out.len() % 2, 0);
        for frame in out.chunks(2).skip(1000).take(20_000) {
            assert!((frame[0] + frame[1]).abs() < 1e-4);
        }
        r.reset();
        let mut again = Vec::new();
        r.process(&input[..882], &mut again);
        assert_eq!(&again[..], &out[..again.len()]);
    }
}
