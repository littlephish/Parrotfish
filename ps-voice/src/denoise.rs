use std::collections::VecDeque;
use std::f32::consts::PI;

const HOP: usize = 192;
const SPAN: usize = 2 * HOP;
const N: usize = 512;
const K: usize = N / 2 + 1;
const SMOOTH: f32 = 0.6;
const PRESENCE: f32 = 16.0;
const NOISE_RATE: f32 = 0.08;
const SPEECH_KEEP: f32 = 0.9;
const LEAST_BIAS: f32 = 6.0;
const SUBS: usize = 6;
const SUB_HOPS: usize = 60;
const OVERSUBTRACT: f32 = 2.0;
const PRIOR_KEEP: f32 = 0.9;
const PRIOR_LIMIT: f32 = 1e6;
const GAIN_FLOOR: f32 = 0.1;
const GAIN_RISE: f32 = 0.6;
const GAIN_FALL: f32 = 0.3;
const TINY: f32 = 1e-20;

pub const DENOISE_DELAY_SAMPLES: usize = HOP;

struct Fft {
    cos: Vec<f32>,
    sin: Vec<f32>,
    rev: Vec<usize>,
    re: Vec<f32>,
    im: Vec<f32>,
}

impl Fft {
    fn new() -> Self {
        let bits = N.trailing_zeros();
        Self {
            cos: (0..N / 2).map(|k| (2.0 * PI * k as f32 / N as f32).cos()).collect(),
            sin: (0..N / 2).map(|k| -(2.0 * PI * k as f32 / N as f32).sin()).collect(),
            rev: (0..N).map(|i| i.reverse_bits() >> (usize::BITS - bits)).collect(),
            re: vec![0.0; N],
            im: vec![0.0; N],
        }
    }

    fn run(&mut self) {
        let mut len = 2;
        while len <= N {
            let half = len / 2;
            let step = N / len;
            let mut start = 0;
            while start < N {
                for k in 0..half {
                    let (c, s) = (self.cos[k * step], self.sin[k * step]);
                    let (i, j) = (start + k, start + k + half);
                    let tr = self.re[j] * c - self.im[j] * s;
                    let ti = self.re[j] * s + self.im[j] * c;
                    self.re[j] = self.re[i] - tr;
                    self.im[j] = self.im[i] - ti;
                    self.re[i] += tr;
                    self.im[i] += ti;
                }
                start += len;
            }
            len *= 2;
        }
    }

    fn forward(&mut self, time: &[f32], re: &mut [f32], im: &mut [f32]) {
        for (i, sample) in time.iter().enumerate().take(N) {
            let at = self.rev[i];
            self.re[at] = *sample;
            self.im[at] = 0.0;
        }
        self.run();
        let scale = 1.0 / N as f32;
        for k in 0..K {
            re[k] = self.re[k] * scale;
            im[k] = self.im[k] * scale;
        }
    }

    fn inverse(&mut self, re: &[f32], im: &[f32], time: &mut [f32]) {
        for k in 0..K {
            let at = self.rev[k];
            self.re[at] = re[k];
            self.im[at] = -im[k];
        }
        for k in 1..N / 2 {
            let at = self.rev[N - k];
            self.re[at] = re[k];
            self.im[at] = im[k];
        }
        self.run();
        time[..N].copy_from_slice(&self.re);
    }
}

struct Overlap {
    fft: Fft,
    shape: Vec<f32>,
    hist: Vec<f32>,
    acc: Vec<f32>,
    time: Vec<f32>,
    re: Vec<f32>,
    im: Vec<f32>,
}

impl Overlap {
    fn new() -> Self {
        Self {
            fft: Fft::new(),
            shape: (0..SPAN).map(|i| 0.5 - 0.5 * (2.0 * PI * i as f32 / SPAN as f32).cos()).collect(),
            hist: vec![0.0; SPAN],
            acc: vec![0.0; N],
            time: vec![0.0; N],
            re: vec![0.0; K],
            im: vec![0.0; K],
        }
    }

    fn reset(&mut self) {
        self.hist.fill(0.0);
        self.acc.fill(0.0);
        self.time.fill(0.0);
        self.re.fill(0.0);
        self.im.fill(0.0);
    }

    fn analyse(&mut self, hop: &[f32]) {
        self.hist.copy_within(HOP.., 0);
        self.hist[HOP..].copy_from_slice(hop);
        for i in 0..SPAN {
            self.time[i] = self.hist[i] * self.shape[i];
        }
        self.time[SPAN..].fill(0.0);
        self.fft.forward(&self.time, &mut self.re, &mut self.im);
    }

    fn synthesise(&mut self, gain: &[f32], hop: &mut [f32]) {
        for k in 0..K {
            self.re[k] *= gain[k];
            self.im[k] *= gain[k];
        }
        self.fft.inverse(&self.re, &self.im, &mut self.time);
        for i in 0..N {
            self.acc[i] += self.time[i];
        }
        hop.copy_from_slice(&self.acc[..HOP]);
        self.acc.copy_within(HOP.., 0);
        self.acc[N - HOP..].fill(0.0);
    }
}

struct Gains {
    smooth: Vec<f32>,
    track: Vec<f32>,
    slots: Vec<f32>,
    least: Vec<f32>,
    held: usize,
    turn: usize,
    filled: usize,
    noise: Vec<f32>,
    speech: Vec<f32>,
    prior: Vec<f32>,
    raw: Vec<f32>,
    gain: Vec<f32>,
}

impl Gains {
    fn new() -> Self {
        Self {
            smooth: vec![0.0; K],
            track: vec![f32::MAX; K],
            slots: vec![f32::MAX; SUBS * K],
            least: vec![f32::MAX; K],
            held: 0,
            turn: 0,
            filled: 0,
            noise: vec![0.0; K],
            speech: vec![0.0; K],
            prior: vec![0.0; K],
            raw: vec![1.0; K],
            gain: vec![1.0; K],
        }
    }

    fn reset(&mut self) {
        self.smooth.fill(0.0);
        self.track.fill(f32::MAX);
        self.slots.fill(f32::MAX);
        self.least.fill(f32::MAX);
        self.held = 0;
        self.turn = 0;
        self.filled = 0;
        self.noise.fill(0.0);
        self.speech.fill(0.0);
        self.prior.fill(0.0);
        self.raw.fill(1.0);
        self.gain.fill(1.0);
    }

    fn step_window(&mut self) {
        self.held += 1;
        if self.held < SUB_HOPS {
            return;
        }
        self.held = 0;
        self.filled = (self.filled + 1).min(SUBS);
        self.slots[self.turn * K..(self.turn + 1) * K].copy_from_slice(&self.track);
        self.turn = (self.turn + 1) % SUBS;
        self.least.copy_from_slice(&self.slots[..K]);
        for sub in 1..SUBS {
            for k in 0..K {
                self.least[k] = self.least[k].min(self.slots[sub * K + k]);
            }
        }
        self.track.copy_from_slice(&self.smooth);
    }

    fn update(&mut self, re: &[f32], im: &[f32]) -> &[f32] {
        let bound = if self.filled >= 2 { LEAST_BIAS } else { 0.0 };
        for k in 0..K {
            let power = re[k] * re[k] + im[k] * im[k];
            let smooth = SMOOTH * self.smooth[k] + (1.0 - SMOOTH) * power;
            self.smooth[k] = smooth;
            self.track[k] = self.track[k].min(smooth);
            let least = self.least[k].min(self.track[k]);
            self.speech[k] = if smooth > PRESENCE * least { 1.0 } else { SPEECH_KEEP * self.speech[k] };
            self.noise[k] += NOISE_RATE * (1.0 - self.speech[k]) * (smooth - self.noise[k]);
            self.noise[k] = self.noise[k].max(bound * least);
            let floor = (OVERSUBTRACT * self.noise[k]).max(TINY);
            let seen = power / floor;
            let fresh = (seen - 1.0).max(0.0);
            let prior = (PRIOR_KEEP * self.prior[k] + (1.0 - PRIOR_KEEP) * fresh).clamp(0.0, PRIOR_LIMIT);
            let wanted = prior / (1.0 + prior);
            self.prior[k] = (wanted * wanted * seen).min(PRIOR_LIMIT);
            self.raw[k] = wanted.max(GAIN_FLOOR);
        }
        for k in 0..K {
            let below = self.raw[k.saturating_sub(1)];
            let above = self.raw[(k + 1).min(K - 1)];
            let target = 0.25 * below + 0.5 * self.raw[k] + 0.25 * above;
            let keep = if target > self.gain[k] { GAIN_RISE } else { GAIN_FALL };
            self.gain[k] += keep * (target - self.gain[k]);
        }
        self.step_window();
        &self.gain
    }
}

pub struct Denoiser {
    overlap: Overlap,
    gains: Gains,
    pending: VecDeque<f32>,
    ready: VecDeque<f32>,
    hop: Vec<f32>,
}

impl Default for Denoiser {
    fn default() -> Self {
        Self::new()
    }
}

impl Denoiser {
    pub fn new() -> Self {
        Self {
            overlap: Overlap::new(),
            gains: Gains::new(),
            pending: VecDeque::new(),
            ready: VecDeque::new(),
            hop: vec![0.0; HOP],
        }
    }

    pub fn reset(&mut self) {
        self.overlap.reset();
        self.gains.reset();
        self.pending.clear();
        self.ready.clear();
        self.hop.fill(0.0);
    }

    pub fn process(&mut self, frame: &mut [f32]) {
        self.pending
            .extend(frame.iter().map(|s| if s.is_finite() { s.clamp(-1.0, 1.0) } else { 0.0 }));
        while self.pending.len() >= HOP {
            for slot in self.hop.iter_mut() {
                *slot = self.pending.pop_front().unwrap_or(0.0);
            }
            self.overlap.analyse(&self.hop);
            let gain = self.gains.update(&self.overlap.re, &self.overlap.im);
            self.overlap.synthesise(gain, &mut self.hop);
            self.ready.extend(self.hop.iter().copied());
        }
        for slot in frame.iter_mut() {
            *slot = self.ready.pop_front().unwrap_or(0.0).clamp(-1.0, 1.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    const RATE: f32 = 48_000.0;
    const SECOND: usize = 48_000;
    const FRAME: usize = 960;

    struct Hiss(u64);

    impl Hiss {
        fn next(&mut self) -> f32 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            ((self.0 >> 40) as f32 / (1u64 << 23) as f32) - 1.0
        }
    }

    fn scaled(mut signal: Vec<f32>, level_db: f32) -> Vec<f32> {
        let rms = (signal.iter().map(|s| s * s).sum::<f32>() / signal.len().max(1) as f32).sqrt();
        let want = 10f32.powf(level_db / 20.0);
        let scale = if rms > 0.0 { want / rms } else { 0.0 };
        signal.iter_mut().for_each(|s| *s *= scale);
        signal
    }

    fn speech(samples: usize, level_db: f32) -> Vec<f32> {
        let mut out = Vec::with_capacity(samples);
        let mut phase = 0.0f32;
        for n in 0..samples {
            let t = n as f32 / RATE;
            let pitch = 130.0 + 30.0 * (2.0 * PI * 0.6 * t).sin();
            phase += 2.0 * PI * pitch / RATE;
            let syllable = 0.5 - 0.5 * (2.0 * PI * 3.0 * t).cos();
            let gate = if (0.6 * t).fract() < 0.6 { 1.0 } else { 0.0 };
            let voiced: f32 = (1..=14).map(|h| (h as f32 * phase).sin() / h as f32).sum();
            out.push(gate * syllable * voiced);
        }
        scaled(out, level_db)
    }

    fn noise(seed: u64, samples: usize, level_db: f32) -> Vec<f32> {
        let mut hiss = Hiss(seed | 1);
        scaled((0..samples).map(|_| hiss.next()).collect(), level_db)
    }

    fn peaked(mut signal: Vec<f32>, peak: f32) -> Vec<f32> {
        let most = signal.iter().fold(0.0f32, |most, s| most.max(s.abs()));
        let scale = if most > 0.0 { peak / most } else { 0.0 };
        signal.iter_mut().for_each(|s| *s *= scale);
        signal
    }

    fn add(a: &[f32], b: &[f32]) -> Vec<f32> {
        a.iter().zip(b.iter()).map(|(x, y)| x + y).collect()
    }

    fn run(signal: &[f32]) -> Vec<f32> {
        let mut denoiser = Denoiser::new();
        let mut out = Vec::with_capacity(signal.len());
        for frame in signal.chunks(FRAME) {
            let mut work = frame.to_vec();
            denoiser.process(&mut work);
            out.extend_from_slice(&work);
        }
        out
    }

    fn through_one_set_of_gains(mix: &[f32], left: &[f32], right: &[f32]) -> (Vec<f32>, Vec<f32>, Vec<f32>) {
        let mut gains = Gains::new();
        let mut paths = [Overlap::new(), Overlap::new(), Overlap::new()];
        let mut outs = [Vec::new(), Vec::new(), Vec::new()];
        let mut hop = vec![0.0; HOP];
        let hops = mix.len() / HOP;
        for h in 0..hops {
            let at = h * HOP;
            for (path, input) in paths.iter_mut().zip([mix, left, right]) {
                path.analyse(&input[at..at + HOP]);
            }
            let gain = gains.update(&paths[0].re, &paths[0].im).to_vec();
            for (path, out) in paths.iter_mut().zip(outs.iter_mut()) {
                path.synthesise(&gain, &mut hop);
                out.extend_from_slice(&hop);
            }
        }
        let [mixed, a, b] = outs;
        (mixed, a, b)
    }

    fn power(signal: &[f32]) -> f32 {
        signal.iter().map(|s| s * s).sum::<f32>() / signal.len().max(1) as f32
    }

    fn db(signal: &[f32]) -> f32 {
        10.0 * power(signal).max(1e-30).log10()
    }

    #[test]
    fn the_added_delay_is_what_the_constant_says() {
        assert_eq!(DENOISE_DELAY_SAMPLES, HOP);
        assert!(DENOISE_DELAY_SAMPLES <= 480);
        let signal = speech(SECOND, -12.0);
        let mut path = Overlap::new();
        let flat = vec![1.0f32; K];
        let mut hop = vec![0.0; HOP];
        let mut out = Vec::new();
        for h in 0..signal.len() / HOP {
            path.analyse(&signal[h * HOP..h * HOP + HOP]);
            path.synthesise(&flat, &mut hop);
            out.extend_from_slice(&hop);
        }
        assert!(out[..DENOISE_DELAY_SAMPLES].iter().all(|s| s.abs() < 1e-6));
        let worst = (DENOISE_DELAY_SAMPLES..out.len())
            .map(|n| (out[n] - signal[n - DENOISE_DELAY_SAMPLES]).abs())
            .fold(0.0f32, f32::max);
        assert!(worst < 1e-5, "the untouched signal comes back {worst} off");
    }

    #[test]
    fn steady_noise_alone_is_taken_well_down() {
        let hiss = noise(7, 3 * SECOND, -40.0);
        let out = run(&hiss);
        let settled = &out[3 * SECOND / 2..];
        let before = db(&hiss[3 * SECOND / 2..]);
        let after = db(settled);
        assert!(after < before - 12.0, "{after:.1} dB out against {before:.1} dB in");
        assert!(after > before - 24.0, "no more than about 20 dB of attenuation, got {:.1}", before - after);
    }

    #[test]
    fn speech_survives_while_the_noise_under_it_is_cut() {
        let voice = speech(5 * SECOND, -20.0);
        let hiss = noise(11, voice.len(), -40.0);
        let mix = add(&voice, &hiss);
        let (mixed, kept_voice, kept_hiss) = through_one_set_of_gains(&mix, &voice, &hiss);
        let plain = run(&mix);
        assert_eq!(mixed.len(), plain.len());
        assert!(mixed.iter().zip(plain.iter()).all(|(a, b)| (a - b).abs() < 1e-6));

        let from = 3 * SECOND / 2;
        let late = from + DENOISE_DELAY_SAMPLES;
        let spoken = db(&voice[from..]);
        let passed = db(&kept_voice[late..]);
        assert!((passed - spoken).abs() < 1.5, "speech went from {spoken:.1} dB to {passed:.1} dB");
        let was = db(&voice[from..]) - db(&hiss[from..]);
        let now = db(&kept_voice[late..]) - db(&kept_hiss[late..]);
        assert!(now > was + 8.0, "signal to noise went from {was:.1} dB to {now:.1} dB");
    }

    #[test]
    fn silence_stays_silent_and_an_onset_comes_through() {
        let quiet = vec![0.0f32; 2 * SECOND];
        assert!(run(&quiet).iter().all(|s| *s == 0.0));

        let voice = speech(2 * SECOND, -14.0);
        let mut signal = quiet.clone();
        signal.extend_from_slice(&voice[SECOND / 6..]);
        let out = run(&signal);
        let onset = 2 * SECOND;
        let window = 48 * 40;
        assert!(db(&signal[onset..onset + window]) > db(&signal[onset - window..onset]) + 40.0);
        let heard = db(&out[onset + DENOISE_DELAY_SAMPLES..onset + DENOISE_DELAY_SAMPLES + window]);
        let sent = db(&signal[onset..onset + window]);
        assert!(heard > sent - 3.0, "onset arrived at {heard:.1} dB against {sent:.1} dB");
    }

    #[test]
    fn the_output_stays_inside_the_sample_range() {
        let loud = peaked(speech(2 * SECOND, -10.0), 0.999);
        assert!(loud.iter().all(|s| s.abs() <= 1.0));
        assert!(db(&loud) > -20.0, "the test signal is loud, at {:.1} dB", db(&loud));
        let out = run(&loud);
        assert!(out.iter().all(|s| s.is_finite() && *s >= -1.0 && *s <= 1.0));
    }

    #[test]
    fn broken_samples_never_turn_into_nan_or_infinity() {
        let mut denoiser = Denoiser::new();
        let mut frame = vec![0.0f32; FRAME];
        frame[0] = f32::NAN;
        frame[1] = f32::INFINITY;
        frame[2] = f32::NEG_INFINITY;
        frame[3] = 1e9;
        frame[4] = -42.0;
        for (i, slot) in frame.iter_mut().enumerate().skip(5) {
            *slot = if i % 2 == 0 { 3.0 } else { -3.0 };
        }
        for _ in 0..10 {
            let mut work = frame.clone();
            denoiser.process(&mut work);
            assert!(work.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
        }
        let mut tail = speech(FRAME * 4, -20.0);
        denoiser.process(&mut tail);
        assert!(tail.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
    }

    #[test]
    fn reset_puts_it_back_to_the_state_of_a_new_one() {
        let voice = speech(SECOND, -20.0);
        let hiss = noise(3, voice.len(), -40.0);
        let mix = add(&voice, &hiss);
        let mut denoiser = Denoiser::new();
        let mut first = Vec::new();
        for frame in mix.chunks(FRAME) {
            let mut work = frame.to_vec();
            denoiser.process(&mut work);
            first.extend_from_slice(&work);
        }
        denoiser.reset();
        let mut again = Vec::new();
        for frame in mix.chunks(FRAME) {
            let mut work = frame.to_vec();
            denoiser.process(&mut work);
            again.extend_from_slice(&work);
        }
        assert_eq!(first, again);
        assert_eq!(first, run(&mix));
    }

    #[test]
    fn it_costs_a_small_part_of_one_processor() {
        let voice = speech(10 * SECOND, -20.0);
        let hiss = noise(5, voice.len(), -40.0);
        let mix = add(&voice, &hiss);
        let mut denoiser = Denoiser::new();
        let mut work = vec![0.0f32; FRAME];
        let mut total = 0.0f32;
        let started = Instant::now();
        for chunk in mix.chunks_exact(FRAME) {
            work.copy_from_slice(chunk);
            denoiser.process(&mut work);
            total += work[0];
        }
        let cost = started.elapsed().as_secs_f32() / 10.0;
        assert!(total.is_finite());
        println!("cost: {:.2}% of one processor, {:.1} us a frame", cost * 100.0, cost * 20_000.0);
        assert!(cost < 0.1, "{cost} of one processor");
    }
}
