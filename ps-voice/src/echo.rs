use std::collections::VecDeque;
use std::f32::consts::PI;

const B: usize = 256;
const N: usize = 2 * B;
const K: usize = B + 1;
const M: usize = 64;
const SCALE: f32 = 32768.0;
const PREEMPH: f32 = 0.9;
const DC_POLE: f32 = 0.995;
const MIN_LEAK: f32 = 0.005;
const DECISION_BLOCKS: usize = 4;
const QUIET_POWER: f32 = 4.0;
const LINEAR_IDLE_AFTER: usize = M + 4;
const SUPPRESS_IDLE_AFTER: usize = M + 220;
const START_BLOCKS: usize = 5;
const AHEAD_BLOCKS: usize = 3;
const AHEAD_WINDOW: usize = 400;
const MAX_REWINDS: usize = 8;
const MAX_RECENT_REWINDS: usize = 16;
const REWIND_FORGET_BLOCKS: usize = 50;
const MAX_AHEAD: usize = 8;
const TRIM_BLOCKS: usize = 24;
const FAR_LIMIT: usize = 96_000;
const OVERSUBTRACT: f32 = 2.0;
const GAIN_FLOOR: f32 = 0.03;
const ECHO_DECAY: f32 = 0.87;
const FAR_HOLD: f32 = 0.985;
const NOISE_RISE: f32 = 1.002;
const COHERENCE_KEEP: f32 = 0.95;
const LEAK_FLOOR: f32 = 0.01;
const COHERENCE_SPAN: f32 = (1.0 + COHERENCE_KEEP) / (1.0 - COHERENCE_KEEP);
const CHANCE_MARGIN: f32 = 4.0;
const DRIFT_EVERY: u64 = 64;
const DRIFT_READINGS: usize = 7;
const DRIFT_MIN_READINGS: usize = 5;
const DRIFT_DEADBAND: f64 = 4e-6;
const DRIFT_LIMIT: f64 = 600e-6;
const DRIFT_GAIN: f64 = 0.8;
const DRIFT_FIT: f32 = 0.8;
const DRIFT_BINS: std::ops::RangeInclusive<usize> = 2..=64;
const DRIFT_SETTLE: u32 = 3;
const ENVELOPE_SPAN: usize = 640;
const ALIGN_EVERY: u64 = 96;
const ALIGN_BACK: usize = 24;
const ALIGN_AHEAD: usize = 320;
const ALIGN_TRUST: f32 = 0.55;
const ALIGN_LATE: usize = 24;
const ALIGN_AIM: usize = 8;
const ALIGN_REST: u64 = 375;
const MAX_HOLD: usize = 150;
const STAT_KEEP: f32 = 0.995;
const STAT_STALE_AFTER: usize = 400;

pub const ECHO_DELAY_SAMPLES: usize = 2 * B;
pub const ECHO_TAIL_MS: usize = M * B * 1000 / 48_000;

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
        for k in 1..B {
            let at = self.rev[N - k];
            self.re[at] = re[k];
            self.im[at] = im[k];
        }
        self.run();
        time[..N].copy_from_slice(&self.re);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FarStep {
    Fresh,
    Again,
    Ahead(usize),
}

pub struct EchoCanceller {
    fft: Fft,
    window: Vec<f32>,
    tilt: Vec<f32>,
    x_prev: Vec<f32>,
    xf_re: Vec<f32>,
    xf_im: Vec<f32>,
    head: usize,
    w_re: Vec<f32>,
    w_im: Vec<f32>,
    f_re: Vec<f32>,
    f_im: Vec<f32>,
    power: Vec<f32>,
    prop: Vec<f32>,
    eh: Vec<f32>,
    yh: Vec<f32>,
    pey: f32,
    pyy: f32,
    leak: f32,
    davg1: f32,
    davg2: f32,
    dvar1: f32,
    dvar2: f32,
    acc_sff: f32,
    acc_see: f32,
    acc_dbf: f32,
    acc_blocks: usize,
    adapted: bool,
    sum_adapt: f32,
    cancel_count: usize,
    trouble: u32,
    sxx: f32,
    mem_x: f32,
    mem_d: f32,
    mem_e: f32,
    dc_x: f32,
    dc_y: f32,
    far_quiet: usize,
    lin_prev: Vec<f32>,
    echo_prev: Vec<f32>,
    tail: Vec<f32>,
    echo_env: Vec<f32>,
    far_env: Vec<f32>,
    noise: Vec<f32>,
    smooth: Vec<f32>,
    gain: Vec<f32>,
    coupling: f32,
    cross_re: Vec<f32>,
    cross_im: Vec<f32>,
    cross_yy: Vec<f32>,
    cross_ee: Vec<f32>,
    quick_yy: Vec<f32>,
    ratio: Vec<f32>,
    mic_fifo: VecDeque<f32>,
    far_fifo: VecDeque<f32>,
    out_fifo: VecDeque<f32>,
    stalled: bool,
    starved: usize,
    recent_rewinds: usize,
    fresh_blocks: usize,
    window_min: usize,
    window_blocks: usize,
    far_step: FarStep,
    hops: u64,
    skew: f64,
    far_pos: f64,
    far_last: f32,
    clock: u64,
    heard: u64,
    readings: Vec<f64>,
    snap_re: Vec<f32>,
    snap_im: Vec<f32>,
    snap_ready: bool,
    settle: u32,
    far_loud: f32,
    heard_far: VecDeque<f32>,
    heard_mic: VecDeque<f32>,
    lag: Option<(i32, u32)>,
    found: Option<i32>,
    hold: usize,
    rewinds: usize,
    aligned_at: u64,
    stat_in: f32,
    stat_out: f32,
    stat_age: usize,
    mic: Vec<f32>,
    far: Vec<f32>,
    dhp: Vec<f32>,
    d: Vec<f32>,
    lin: Vec<f32>,
    echo: Vec<f32>,
    out: Vec<f32>,
    e_fg: Vec<f32>,
    e_bg: Vec<f32>,
    y_fg: Vec<f32>,
    y_bg: Vec<f32>,
    t: Vec<f32>,
    a_re: Vec<f32>,
    a_im: Vec<f32>,
    b_re: Vec<f32>,
    b_im: Vec<f32>,
    step: Vec<f32>,
    rf: Vec<f32>,
    yf: Vec<f32>,
    #[cfg(test)]
    trace: Vec<f32>,
}

fn estimate(
    fft: &mut Fft,
    w_re: &[f32],
    w_im: &[f32],
    xf_re: &[f32],
    xf_im: &[f32],
    head: usize,
    acc_re: &mut [f32],
    acc_im: &mut [f32],
    time: &mut [f32],
) {
    acc_re.fill(0.0);
    acc_im.fill(0.0);
    for p in 0..M {
        let slot = ((head + p) % M) * K;
        let (xr, xi) = (&xf_re[slot..slot + K], &xf_im[slot..slot + K]);
        let (wr, wi) = (&w_re[p * K..p * K + K], &w_im[p * K..p * K + K]);
        for k in 0..K {
            acc_re[k] += wr[k] * xr[k] - wi[k] * xi[k];
            acc_im[k] += wr[k] * xi[k] + wi[k] * xr[k];
        }
    }
    fft.inverse(acc_re, acc_im, time);
}

fn energy(samples: &[f32]) -> f32 {
    samples.iter().map(|s| s * s).sum()
}

impl Default for EchoCanceller {
    fn default() -> Self {
        Self::new()
    }
}

impl EchoCanceller {
    pub fn new() -> Self {
        let window: Vec<f32> = (0..N).map(|i| (PI * (i as f32 + 0.5) / N as f32).sin()).collect();
        let tilt: Vec<f32> = (0..K)
            .map(|k| {
                let w = 2.0 * PI * k as f32 / N as f32;
                1.0 / (1.0 - 2.0 * PREEMPH * w.cos() + PREEMPH * PREEMPH)
            })
            .collect();
        let mut canceller = Self {
            fft: Fft::new(),
            window,
            tilt,
            x_prev: vec![0.0; B],
            xf_re: vec![0.0; M * K],
            xf_im: vec![0.0; M * K],
            head: 0,
            w_re: vec![0.0; M * K],
            w_im: vec![0.0; M * K],
            f_re: vec![0.0; M * K],
            f_im: vec![0.0; M * K],
            power: vec![0.0; K],
            prop: vec![0.0; M],
            eh: vec![0.0; K],
            yh: vec![0.0; K],
            pey: 1.0,
            pyy: 1.0,
            leak: 0.0,
            davg1: 0.0,
            davg2: 0.0,
            dvar1: 0.0,
            dvar2: 0.0,
            acc_sff: 0.0,
            acc_see: 0.0,
            acc_dbf: 0.0,
            acc_blocks: 0,
            adapted: false,
            sum_adapt: 0.0,
            cancel_count: 0,
            trouble: 0,
            sxx: 0.0,
            mem_x: 0.0,
            mem_d: 0.0,
            mem_e: 0.0,
            dc_x: 0.0,
            dc_y: 0.0,
            far_quiet: SUPPRESS_IDLE_AFTER + 1,
            lin_prev: vec![0.0; B],
            echo_prev: vec![0.0; B],
            tail: vec![0.0; B],
            echo_env: vec![0.0; K],
            far_env: vec![0.0; K],
            noise: vec![0.0; K],
            smooth: vec![0.0; K],
            gain: vec![1.0; K],
            coupling: 8.0,
            cross_re: vec![0.0; K],
            cross_im: vec![0.0; K],
            cross_yy: vec![0.0; K],
            cross_ee: vec![0.0; K],
            quick_yy: vec![0.0; K],
            ratio: vec![1.0; K],
            mic_fifo: VecDeque::new(),
            far_fifo: VecDeque::new(),
            out_fifo: VecDeque::from(vec![0.0; B]),
            stalled: true,
            starved: 0,
            recent_rewinds: 0,
            fresh_blocks: 0,
            window_min: usize::MAX,
            window_blocks: 0,
            far_step: FarStep::Fresh,
            hops: 0,
            skew: 0.0,
            far_pos: 0.0,
            far_last: 0.0,
            clock: 0,
            heard: 0,
            readings: Vec::new(),
            snap_re: vec![0.0; M * K],
            snap_im: vec![0.0; M * K],
            snap_ready: false,
            settle: 0,
            far_loud: 0.0,
            heard_far: VecDeque::new(),
            heard_mic: VecDeque::new(),
            lag: None,
            found: None,
            hold: 0,
            rewinds: 0,
            aligned_at: 0,
            stat_in: 0.0,
            stat_out: 0.0,
            stat_age: STAT_STALE_AFTER,
            mic: vec![0.0; B],
            far: vec![0.0; B],
            dhp: vec![0.0; B],
            d: vec![0.0; B],
            lin: vec![0.0; B],
            echo: vec![0.0; B],
            out: vec![0.0; B],
            e_fg: vec![0.0; B],
            e_bg: vec![0.0; B],
            y_fg: vec![0.0; B],
            y_bg: vec![0.0; B],
            t: vec![0.0; N],
            a_re: vec![0.0; K],
            a_im: vec![0.0; K],
            b_re: vec![0.0; K],
            b_im: vec![0.0; K],
            step: vec![0.0; K],
            rf: vec![0.0; K],
            yf: vec![0.0; K],
            #[cfg(test)]
            trace: Vec::new(),
        };
        canceller.forget();
        canceller
    }

    fn forget(&mut self) {
        for buffer in [&mut self.w_re, &mut self.w_im, &mut self.f_re, &mut self.f_im, &mut self.xf_re, &mut self.xf_im] {
            buffer.fill(0.0);
        }
        for buffer in [&mut self.eh, &mut self.yh, &mut self.echo_env, &mut self.far_env, &mut self.x_prev] {
            buffer.fill(0.0);
        }
        for buffer in [&mut self.cross_re, &mut self.cross_im, &mut self.cross_yy, &mut self.cross_ee, &mut self.quick_yy] {
            buffer.fill(0.0);
        }
        self.ratio.fill(1.0);
        self.power.fill(1.0);
        let decay = (-2.4 / M as f32).exp();
        let mut weight = 0.7;
        let mut sum = 0.0;
        for slot in self.prop.iter_mut() {
            *slot = weight;
            sum += weight;
            weight *= decay;
        }
        for slot in self.prop.iter_mut() {
            *slot = 0.8 * *slot / sum;
        }
        self.pey = 1.0;
        self.pyy = 1.0;
        self.leak = 0.0;
        self.davg1 = 0.0;
        self.davg2 = 0.0;
        self.dvar1 = 0.0;
        self.dvar2 = 0.0;
        self.acc_sff = 0.0;
        self.acc_see = 0.0;
        self.acc_dbf = 0.0;
        self.acc_blocks = 0;
        self.adapted = false;
        self.sum_adapt = 0.0;
        self.cancel_count = 0;
        self.trouble = 0;
        self.sxx = 0.0;
        self.mem_x = 0.0;
        self.coupling = 8.0;
        self.window_min = usize::MAX;
        self.window_blocks = 0;
        self.far_pos = 0.0;
        self.far_last = 0.0;
        self.heard = 0;
        self.readings.clear();
        self.snap_ready = false;
        self.heard_far.clear();
        self.heard_mic.clear();
        self.lag = None;
        self.rewinds = 0;
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    pub fn push_far(&mut self, samples: &[f32]) {
        self.far_fifo.extend(samples.iter().map(|s| if s.is_finite() { s.clamp(-4.0, 4.0) * SCALE } else { 0.0 }));
        if self.far_fifo.len() > FAR_LIMIT {
            self.far_fifo.clear();
            self.stalled = true;
        }
    }

    pub fn process(&mut self, mic: &mut [f32]) {
        self.mic_fifo.extend(mic.iter().map(|s| if s.is_finite() { s.clamp(-1.0, 1.0) * SCALE } else { 0.0 }));
        while self.mic_fifo.len() >= B {
            for slot in self.mic.iter_mut() {
                *slot = self.mic_fifo.pop_front().unwrap_or(0.0);
            }
            self.take_far();
            self.linear();
            self.watch_drift();
            #[cfg(test)]
            self.trace.extend(self.lin.iter().map(|s| s / SCALE));
            self.suppress();
            self.out_fifo.extend(self.out.iter().map(|s| s / SCALE));
        }
        for slot in mic.iter_mut() {
            *slot = self.out_fifo.pop_front().unwrap_or(0.0).clamp(-1.0, 1.0);
        }
    }

    pub fn reduction_db(&self) -> Option<f32> {
        if self.stat_age >= STAT_STALE_AFTER || self.stat_in < 100.0 * B as f32 {
            return None;
        }
        Some(10.0 * ((self.stat_in + 1.0) / (self.stat_out + 1.0)).log10())
    }

    pub fn hops(&self) -> u64 {
        self.hops
    }

    pub fn is_converged(&self) -> bool {
        self.adapted
    }

    pub fn clock_drift_ppm(&self) -> f32 {
        (self.skew * 1e6) as f32
    }

    pub fn echo_delay_ms(&self) -> Option<f32> {
        self.found.map(|blocks| blocks as f32 * B as f32 / 48.0)
    }

    fn align(&mut self) {
        let count = self.heard_far.len();
        if count < ENVELOPE_SPAN / 2 || count != self.heard_mic.len() {
            return;
        }
        let far: Vec<f32> = self.heard_far.iter().copied().collect();
        let mic: Vec<f32> = self.heard_mic.iter().copied().collect();
        let far_mean = far.iter().sum::<f32>() / count as f32;
        let mic_mean = mic.iter().sum::<f32>() / count as f32;
        let far_size: f32 = far.iter().map(|v| (v - far_mean) * (v - far_mean)).sum();
        let mic_size: f32 = mic.iter().map(|v| (v - mic_mean) * (v - mic_mean)).sum();
        if far_mean < 8.0 || far_size <= 0.0 || mic_size <= 0.0 {
            return;
        }
        let (mut best, mut best_at) = (0.0f32, 0i32);
        for lag in -(ALIGN_BACK as i32)..=(ALIGN_AHEAD.min(count - 64) as i32) {
            let mut sum = 0.0f32;
            for (at, heard) in mic.iter().enumerate() {
                let from = at as i32 - lag;
                if from >= 0 && (from as usize) < count {
                    sum += (heard - mic_mean) * (far[from as usize] - far_mean);
                }
            }
            if sum > best {
                best = sum;
                best_at = lag;
            }
        }
        if best / (far_size * mic_size).sqrt() < ALIGN_TRUST {
            self.lag = None;
            return;
        }
        let seen = match self.lag {
            Some((at, times)) if (at - best_at).abs() <= 1 => times + 1,
            _ => 1,
        };
        self.lag = Some((best_at, seen));
        if seen < 2 {
            return;
        }
        self.found = Some(best_at + self.hold as i32);
        let rested = self.clock.saturating_sub(self.aligned_at) >= ALIGN_REST || self.aligned_at == 0;
        if best_at > ALIGN_LATE as i32 && rested && self.hold < MAX_HOLD {
            let more = (best_at as usize - ALIGN_AIM).min(MAX_HOLD - self.hold);
            self.hold += more;
            self.rewinds += more;
            self.aligned_at = self.clock;
            self.heard_far.clear();
            self.heard_mic.clear();
            self.lag = None;
        }
    }

    fn far_needed(&self) -> usize {
        if self.skew == 0.0 && self.far_pos == 0.0 {
            B
        } else {
            (self.far_pos + (B - 1) as f64 * (1.0 + self.skew)) as usize + 3
        }
    }

    fn pop_far(&mut self) {
        if self.skew == 0.0 && self.far_pos == 0.0 {
            for slot in self.far.iter_mut() {
                *slot = self.far_fifo.pop_front().unwrap_or(0.0);
            }
            self.far_last = self.far[B - 1];
            return;
        }
        let step = 1.0 + self.skew;
        let mut pos = self.far_pos;
        for slot in self.far.iter_mut() {
            let whole = pos as usize;
            let part = (pos - whole as f64) as f32;
            let before = if whole == 0 { self.far_last } else { self.far_fifo[whole - 1] };
            let (a, b, c) = (self.far_fifo[whole], self.far_fifo[whole + 1], self.far_fifo[whole + 2]);
            let curve = 2.0 * before - 5.0 * a + 4.0 * b - c + part * (3.0 * (a - b) + c - before);
            *slot = a + 0.5 * part * (b - before + part * curve);
            pos += step;
        }
        let used = pos as usize;
        if used > 0 {
            self.far_last = self.far_fifo[used - 1];
            self.far_fifo.drain(..used);
        }
        self.far_pos = pos - used as f64;
    }

    fn turned(&mut self) -> Option<f64> {
        let (mut sum_re, mut sum_im, mut size, mut lever, mut pull) = (0.0f32, 0.0f32, 0.0f32, 0.0f32, 0.0f32);
        let (mut now, mut then) = (0.0f32, 0.0f32);
        let scale = 2.0 * PI / N as f32;
        for k in DRIFT_BINS {
            let (mut re, mut im) = (0.0f32, 0.0f32);
            for p in 0..M {
                let at = p * K + k;
                let (wr, wi, sr, si) = (self.w_re[at], self.w_im[at], self.snap_re[at], self.snap_im[at]);
                re += wr * sr + wi * si;
                im += wi * sr - wr * si;
                now += wr * wr + wi * wi;
                then += sr * sr + si * si;
            }
            let length = (re * re + im * im).sqrt();
            self.rf[k] = re;
            self.yf[k] = im;
            size += length;
            lever += (k * k) as f32 * length;
            pull += k as f32 * im.atan2(re) * length;
        }
        let ready = self.snap_ready;
        self.snap_re.copy_from_slice(&self.w_re);
        self.snap_im.copy_from_slice(&self.w_im);
        self.snap_ready = true;
        if !ready || size <= 0.0 || lever <= 0.0 || now <= 0.0 || then <= 0.0 {
            return None;
        }
        let shift = -pull / (lever * scale);
        for k in DRIFT_BINS {
            let angle = scale * k as f32 * shift;
            sum_re += self.rf[k] * angle.cos() - self.yf[k] * angle.sin();
            sum_im += self.rf[k] * angle.sin() + self.yf[k] * angle.cos();
        }
        let straight = (sum_re * sum_re + sum_im * sum_im).sqrt() / size;
        let alike = size / (now * then).sqrt();
        if straight < DRIFT_FIT || alike < DRIFT_FIT || shift.abs() > 3.5 {
            return None;
        }
        Some(f64::from(shift))
    }

    fn watch_drift(&mut self) {
        self.clock += 1;
        if self.far_quiet <= LINEAR_IDLE_AFTER {
            self.heard += 1;
        }
        if self.far_step != FarStep::Fresh {
            self.readings.clear();
            self.snap_ready = false;
        }
        if self.clock % ALIGN_EVERY == 0 {
            self.align();
        }
        if self.clock % DRIFT_EVERY != 0 {
            return;
        }
        let busy = self.heard * 2 >= DRIFT_EVERY;
        self.heard = 0;
        if !self.adapted || !busy {
            self.snap_ready = false;
            return;
        }
        let turned = self.turned();
        if self.settle > 0 {
            self.settle -= 1;
            return;
        }
        let Some(shift) = turned else {
            return;
        };
        self.readings.push(shift / (DRIFT_EVERY as usize * B) as f64);
        if self.readings.len() > DRIFT_READINGS {
            self.readings.remove(0);
        }
        if self.readings.len() < DRIFT_MIN_READINGS {
            return;
        }
        let mut sorted = self.readings.clone();
        sorted.sort_by(f64::total_cmp);
        let middle = sorted[sorted.len() / 2];
        let near = (0.5 * middle.abs()).max(3e-6);
        let agree = sorted.iter().filter(|rate| (**rate - middle).abs() <= near).count();
        if middle.abs() >= DRIFT_DEADBAND && agree * 3 >= sorted.len() * 2 {
            self.skew = (self.skew - DRIFT_GAIN * middle).clamp(-DRIFT_LIMIT, DRIFT_LIMIT);
            self.readings.clear();
            self.settle = DRIFT_SETTLE;
        }
    }

    fn take_far(&mut self) {
        self.far_step = FarStep::Fresh;
        if self.stalled {
            let keep = (START_BLOCKS + 1 + self.hold) * B + 3;
            if self.far_fifo.len() < keep {
                self.far.fill(0.0);
                return;
            }
            let extra = self.far_fifo.len() - keep;
            self.far_fifo.drain(..extra);
            self.stalled = false;
            self.starved = 0;
            self.recent_rewinds = 0;
            self.forget();
        }
        if self.rewinds > 0 {
            self.rewinds -= 1;
            self.far_step = FarStep::Again;
            self.hops += 1;
            return;
        }
        if self.far_fifo.len() < self.far_needed() {
            if self.starved < MAX_REWINDS && self.recent_rewinds < MAX_RECENT_REWINDS {
                self.starved += 1;
                self.recent_rewinds += 1;
                self.far_step = FarStep::Again;
                self.hops += 1;
                return;
            }
            self.stalled = true;
            self.far.fill(0.0);
            return;
        }
        self.starved = 0;
        self.fresh_blocks += 1;
        if self.fresh_blocks >= REWIND_FORGET_BLOCKS {
            self.fresh_blocks = 0;
            self.recent_rewinds = self.recent_rewinds.saturating_sub(1);
        }
        self.pop_far();
        self.window_min = self.window_min.min(self.far_fifo.len());
        self.window_blocks += 1;
        if self.window_blocks < AHEAD_WINDOW {
            return;
        }
        let lowest = self.window_min;
        self.window_min = usize::MAX;
        self.window_blocks = 0;
        if lowest >= (TRIM_BLOCKS + self.hold) * B {
            self.stalled = true;
        } else if lowest >= (AHEAD_BLOCKS + self.hold) * B {
            let extra = (lowest / B + 1 - AHEAD_BLOCKS - self.hold).min(MAX_AHEAD);
            if extra > 0 {
                self.far_step = FarStep::Ahead(extra);
                self.hops += 1;
            }
        }
    }

    fn shift_weights(&mut self, later: bool) {
        for buffer in [&mut self.w_re, &mut self.w_im, &mut self.f_re, &mut self.f_im] {
            if later {
                buffer.copy_within(0..(M - 1) * K, K);
                buffer[..K].fill(0.0);
            } else {
                buffer.copy_within(K.., 0);
                buffer[(M - 1) * K..].fill(0.0);
            }
        }
    }

    fn advance(&mut self) {
        let loud = energy(&self.far);
        self.far_loud = (loud / B as f32).sqrt();
        let quiet = loud < QUIET_POWER * B as f32;
        self.far_quiet = if quiet { self.far_quiet.saturating_add(1) } else { 0 };
        self.t[..B].copy_from_slice(&self.x_prev);
        for i in 0..B {
            let sample = self.far[i];
            self.t[B + i] = sample - PREEMPH * self.mem_x;
            self.mem_x = sample;
        }
        self.x_prev.copy_from_slice(&self.t[B..]);
        self.sxx = energy(&self.t[B..]);
        self.head = (self.head + M - 1) % M;
        let slot = self.head * K;
        if self.far_quiet > LINEAR_IDLE_AFTER {
            self.xf_re[slot..slot + K].fill(0.0);
            self.xf_im[slot..slot + K].fill(0.0);
        } else {
            self.fft.forward(&self.t, &mut self.xf_re[slot..slot + K], &mut self.xf_im[slot..slot + K]);
        }
    }

    fn linear(&mut self) {
        match self.far_step {
            FarStep::Fresh => self.advance(),
            FarStep::Again => self.shift_weights(false),
            FarStep::Ahead(extra) => {
                self.advance();
                for _ in 0..extra {
                    if self.far_fifo.len() < self.far_needed() {
                        break;
                    }
                    self.pop_far();
                    self.advance();
                    self.shift_weights(true);
                }
            }
        }
        for i in 0..B {
            let sample = self.mic[i];
            let out = sample - self.dc_x + DC_POLE * self.dc_y;
            self.dc_x = sample;
            self.dc_y = out;
            self.dhp[i] = out;
        }
        if self.far_quiet > LINEAR_IDLE_AFTER {
            self.lin.copy_from_slice(&self.dhp);
            self.echo.fill(0.0);
            self.mem_d = self.dhp[B - 1];
            self.mem_e = self.dhp[B - 1];
            self.stat_age = self.stat_age.saturating_add(1);
            return;
        }
        self.heard_far.push_back(self.far_loud);
        self.heard_mic.push_back((energy(&self.dhp) / B as f32).sqrt());
        if self.heard_far.len() > ENVELOPE_SPAN {
            self.heard_far.pop_front();
            self.heard_mic.pop_front();
        }
        for i in 0..B {
            let sample = self.dhp[i];
            self.d[i] = sample - PREEMPH * self.mem_d;
            self.mem_d = sample;
        }
        let sdd = energy(&self.d);
        let clipped = self.mic.iter().any(|s| s.abs() >= 0.99 * SCALE);

        estimate(&mut self.fft, &self.f_re, &self.f_im, &self.xf_re, &self.xf_im, self.head, &mut self.a_re, &mut self.a_im, &mut self.t);
        self.y_fg.copy_from_slice(&self.t[B..]);
        estimate(&mut self.fft, &self.w_re, &self.w_im, &self.xf_re, &self.xf_im, self.head, &mut self.a_re, &mut self.a_im, &mut self.t);
        self.y_bg.copy_from_slice(&self.t[B..]);
        let (mut sff, mut see, mut syy, mut sey, mut dbf) = (0.0f32, 0.0f32, 0.0f32, 0.0f32, 0.0f32);
        for i in 0..B {
            self.e_fg[i] = self.d[i] - self.y_fg[i];
            self.e_bg[i] = self.d[i] - self.y_bg[i];
            sff += self.e_fg[i] * self.e_fg[i];
            see += self.e_bg[i] * self.e_bg[i];
            syy += self.y_bg[i] * self.y_bg[i];
            sey += self.e_bg[i] * self.y_bg[i];
            let apart = self.y_fg[i] - self.y_bg[i];
            dbf += apart * apart;
        }
        let limit = 1e9 * B as f32;
        let sane = sff.is_finite() && see.is_finite() && syy.is_finite() && self.sxx.is_finite();
        if !sane || sff > limit || syy > limit || self.sxx > limit {
            self.trouble += 50;
        } else if sff > sdd + 10_000.0 * B as f32 {
            self.trouble += 1;
        } else {
            self.trouble = 0;
        }
        if self.trouble >= 50 {
            self.forget();
            self.lin.copy_from_slice(&self.dhp);
            self.echo.fill(0.0);
            self.mem_d = self.dhp[B - 1];
            self.mem_e = self.dhp[B - 1];
            return;
        }

        self.acc_sff += sff;
        self.acc_see += see;
        self.acc_dbf += dbf;
        self.acc_blocks += 1;
        let mut promote = false;
        if self.acc_blocks >= DECISION_BLOCKS {
            let (fore, back, apart) = (self.acc_sff, self.acc_see, 10.0 + self.acc_dbf);
            self.acc_sff = 0.0;
            self.acc_see = 0.0;
            self.acc_dbf = 0.0;
            self.acc_blocks = 0;
            let gap = fore - back;
            self.davg1 = 0.6 * self.davg1 + 0.4 * gap;
            self.davg2 = 0.85 * self.davg2 + 0.15 * gap;
            self.dvar1 = 0.36 * self.dvar1 + 0.16 * fore * apart;
            self.dvar2 = 0.7225 * self.dvar2 + 0.0225 * fore * apart;
            promote = gap * gap.abs() > fore * apart
                || self.davg1 * self.davg1.abs() > 0.5 * self.dvar1
                || self.davg2 * self.davg2.abs() > 0.25 * self.dvar2;
            let demote = !promote
                && (-gap * gap.abs() > 4.0 * fore * apart
                    || -self.davg1 * self.davg1.abs() > 4.0 * self.dvar1
                    || -self.davg2 * self.davg2.abs() > 4.0 * self.dvar2);
            if promote || demote {
                self.davg1 = 0.0;
                self.davg2 = 0.0;
                self.dvar1 = 0.0;
                self.dvar2 = 0.0;
            }
            if promote {
                self.f_re.copy_from_slice(&self.w_re);
                self.f_im.copy_from_slice(&self.w_im);
            } else if demote {
                self.w_re.copy_from_slice(&self.f_re);
                self.w_im.copy_from_slice(&self.f_im);
                self.y_bg.copy_from_slice(&self.y_fg);
                self.e_bg.copy_from_slice(&self.e_fg);
                see = sff;
                syy = energy(&self.y_bg);
                sey = self.e_bg.iter().zip(self.y_bg.iter()).map(|(e, y)| e * y).sum();
            }
        }

        for i in 0..B {
            let error = if promote {
                let mix = (i as f32 + 0.5) / B as f32;
                mix * self.e_bg[i] + (1.0 - mix) * self.e_fg[i]
            } else {
                self.e_fg[i]
            };
            let out = error + PREEMPH * self.mem_e;
            self.mem_e = out;
            self.lin[i] = out;
            self.echo[i] = self.dhp[i] - out;
        }
        self.stat_in = STAT_KEEP * self.stat_in + energy(&self.dhp);
        self.stat_age = 0;

        self.t[..B].fill(0.0);
        self.t[B..].copy_from_slice(&self.e_bg);
        self.fft.forward(&self.t, &mut self.a_re, &mut self.a_im);
        self.t[B..].copy_from_slice(&self.y_bg);
        self.fft.forward(&self.t, &mut self.b_re, &mut self.b_im);
        let slot = self.head * K;
        let spread = 0.35 / M as f32;
        let average = B as f32 / 48_000.0;
        let (mut pey_now, mut pyy_now) = (0.0f32, 0.0f32);
        for k in 0..K {
            self.rf[k] = self.a_re[k] * self.a_re[k] + self.a_im[k] * self.a_im[k];
            self.yf[k] = self.b_re[k] * self.b_re[k] + self.b_im[k] * self.b_im[k];
            let xf = self.xf_re[slot + k] * self.xf_re[slot + k] + self.xf_im[slot + k] * self.xf_im[slot + k];
            self.power[k] = (1.0 - spread) * self.power[k] + 1.0 + spread * xf;
            let eh = self.rf[k] - self.eh[k];
            let yh = self.yf[k] - self.yh[k];
            pey_now += eh * yh;
            pyy_now += yh * yh;
            self.eh[k] = (1.0 - average) * self.eh[k] + average * self.rf[k];
            self.yh[k] = (1.0 - average) * self.yh[k] + average * self.yf[k];
        }
        pyy_now = pyy_now.sqrt();
        if pyy_now > 0.0 {
            pey_now /= pyy_now;
        }
        let see_safe = see.max(1e-3);
        let alpha = ((2.0 * average * syy).min(0.5 * average * see_safe) / see_safe).clamp(0.0, 1.0);
        self.pey = (1.0 - alpha) * self.pey + alpha * pey_now;
        self.pyy = (1.0 - alpha) * self.pyy + alpha * pyy_now;
        self.pyy = self.pyy.max(1.0);
        self.pey = self.pey.clamp(MIN_LEAK * self.pyy, self.pyy);
        self.leak = self.pey / self.pyy;

        let mut rer = (0.0001 * self.sxx + 3.0 * self.leak * syy) / see_safe;
        rer = rer.max(sey * sey / (1.0 + see * syy)).min(0.5);
        if !self.adapted && self.sum_adapt > M as f32 && self.leak > 0.03 {
            self.adapted = true;
        }
        if self.adapted {
            for k in 0..K {
                let error = self.rf[k] + 1.0;
                let residual = (self.leak * self.yf[k]).min(0.5 * error);
                let rate = 0.7 * residual + 0.3 * rer * error;
                self.step[k] = rate / (error * (self.power[k] + 10.0));
            }
        } else {
            let mut rate = 0.0;
            if self.sxx > 2000.0 * B as f32 {
                rate = (0.25 * self.sxx).min(0.25 * see_safe) / see_safe;
            }
            for k in 0..K {
                self.step[k] = rate / (self.power[k] + 10.0);
            }
            self.sum_adapt += rate;
        }

        if clipped {
            return;
        }
        if self.adapted {
            let mut largest = 1.0f32;
            for p in 0..M {
                let weights = self.w_re[p * K..p * K + K].iter().zip(self.w_im[p * K..p * K + K].iter());
                let size = (1.0 + weights.map(|(r, i)| r * r + i * i).sum::<f32>()).sqrt();
                self.prop[p] = size;
                largest = largest.max(size);
            }
            let mut sum = 0.0;
            for slot in self.prop.iter_mut() {
                *slot += 0.1 * largest;
                sum += *slot;
            }
            for slot in self.prop.iter_mut() {
                *slot = 0.99 * *slot / sum;
            }
        }
        for p in 0..M {
            let from = ((self.head + p) % M) * K;
            let share = self.prop[p];
            let (xr, xi) = (&self.xf_re[from..from + K], &self.xf_im[from..from + K]);
            let (wr, wi) = (&mut self.w_re[p * K..p * K + K], &mut self.w_im[p * K..p * K + K]);
            for k in 0..K {
                let rate = share * self.step[k];
                wr[k] += rate * (xr[k] * self.a_re[k] + xi[k] * self.a_im[k]);
                wi[k] += rate * (xr[k] * self.a_im[k] - xi[k] * self.a_re[k]);
            }
        }
        let turn = 1 + self.cancel_count % (M - 1);
        self.cancel_count += 1;
        for p in [0, turn] {
            let range = p * K..p * K + K;
            self.fft.inverse(&self.w_re[range.clone()], &self.w_im[range.clone()], &mut self.t);
            self.t[B..].fill(0.0);
            let (re, im) = (&mut self.w_re[range.clone()], &mut self.w_im[range]);
            self.fft.forward(&self.t, re, im);
        }
    }

    fn suppress(&mut self) {
        if self.far_quiet > SUPPRESS_IDLE_AFTER {
            for i in 0..B {
                let (rise, fall) = (self.window[i], self.window[B + i]);
                self.out[i] = self.tail[i] + self.lin_prev[i] * rise * rise;
                self.tail[i] = self.lin[i] * fall * fall;
            }
            self.gain.fill(1.0);
            self.lin_prev.copy_from_slice(&self.lin);
            self.echo_prev.copy_from_slice(&self.echo);
            return;
        }
        for i in 0..B {
            self.t[i] = self.echo_prev[i] * self.window[i];
            self.t[B + i] = self.echo[i] * self.window[B + i];
        }
        self.fft.forward(&self.t, &mut self.b_re, &mut self.b_im);
        for i in 0..B {
            self.t[i] = self.lin_prev[i] * self.window[i];
            self.t[B + i] = self.lin[i] * self.window[B + i];
        }
        self.fft.forward(&self.t, &mut self.a_re, &mut self.a_im);

        let slot = self.head * K;
        let far_active = self.far_quiet <= LINEAR_IDLE_AFTER;
        let (mut heard, mut played) = (0.0f32, 0.0f32);
        for k in 0..K {
            let se = self.a_re[k] * self.a_re[k] + self.a_im[k] * self.a_im[k];
            self.rf[k] = se;
            let xf = self.xf_re[slot + k] * self.xf_re[slot + k] + self.xf_im[slot + k] * self.xf_im[slot + k];
            self.far_env[k] = (self.far_env[k] * FAR_HOLD).max(xf * self.tilt[k]);
            if k >= 4 {
                heard += se;
                played += self.far_env[k];
            }
        }
        let starting = !self.adapted && self.sum_adapt <= M as f32;
        if starting && far_active && played > 1e-3 {
            self.coupling = (self.coupling * 1.002).min(heard / played).clamp(1e-4, 8.0);
        }
        for k in 0..K {
            let se = self.rf[k];
            let (er, ei, yr, yi) = (self.a_re[k], self.a_im[k], self.b_re[k], self.b_im[k]);
            let sy = yr * yr + yi * yi;
            if sy > 1e-3 {
                let fresh = 1.0 - COHERENCE_KEEP;
                self.cross_re[k] = COHERENCE_KEEP * self.cross_re[k] + fresh * (er * yr + ei * yi);
                self.cross_im[k] = COHERENCE_KEEP * self.cross_im[k] + fresh * (ei * yr - er * yi);
                self.cross_yy[k] = COHERENCE_KEEP * self.cross_yy[k] + fresh * sy;
                self.cross_ee[k] = COHERENCE_KEEP * self.cross_ee[k] + fresh * se;
                let alike = self.cross_re[k] * self.cross_re[k] + self.cross_im[k] * self.cross_im[k];
                let chance = CHANCE_MARGIN * self.cross_ee[k] * self.cross_yy[k] / COHERENCE_SPAN;
                let shared = (alike - chance).max(0.0);
                self.ratio[k] = (shared / (self.cross_yy[k] * self.cross_yy[k] + 1e-12)).clamp(LEAK_FLOOR, 1.0);
            }
            self.quick_yy[k] = 0.5 * self.quick_yy[k] + 0.5 * sy;
            let early = if starting { self.coupling * self.far_env[k] } else { 0.0 };
            self.echo_env[k] = (self.echo_env[k] * ECHO_DECAY).max(self.ratio[k] * self.quick_yy[k] + early);
            self.smooth[k] = 0.85 * self.smooth[k] + 0.15 * se;
            self.noise[k] = if self.smooth[k] < self.noise[k] || self.noise[k] <= 0.0 {
                self.smooth[k]
            } else {
                self.noise[k] * NOISE_RISE
            };
            let unwanted = OVERSUBTRACT * self.echo_env[k];
            let wanted = if se > unwanted { (se - unwanted) / se } else { 0.0 };
            let floor = (self.noise[k] / (se + 1e-9)).sqrt().clamp(GAIN_FLOOR, 1.0);
            self.step[k] = wanted.max(floor);
        }
        for k in 0..K {
            let below = self.step[k.saturating_sub(1)];
            let above = self.step[(k + 1).min(K - 1)];
            let target = 0.25 * below + 0.5 * self.step[k] + 0.25 * above;
            let gain = if target < self.gain[k] { target } else { 0.5 * self.gain[k] + 0.5 * target };
            self.gain[k] = gain;
            self.a_re[k] *= gain;
            self.a_im[k] *= gain;
        }
        self.fft.inverse(&self.a_re, &self.a_im, &mut self.t);
        for i in 0..B {
            self.out[i] = self.tail[i] + self.t[i] * self.window[i];
            self.tail[i] = self.t[B + i] * self.window[B + i];
        }
        if far_active {
            self.stat_out = STAT_KEEP * self.stat_out + energy(&self.out);
        }
        self.lin_prev.copy_from_slice(&self.lin);
        self.echo_prev.copy_from_slice(&self.echo);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Noise(u64);

    impl Noise {
        fn next(&mut self) -> f32 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            ((self.0 >> 40) as f32 / (1u64 << 23) as f32) - 1.0
        }
    }

    fn voice(seed: u64, samples: usize, level: f32) -> Vec<f32> {
        let mut noise = Noise(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let mut out = Vec::with_capacity(samples);
        let (mut low, mut mid, mut loudness, mut target, mut hold) = (0.0f32, 0.0f32, 0.0f32, 0.0f32, 0usize);
        for _ in 0..samples {
            if hold == 0 {
                target = if noise.next() > -0.4 { 0.35 + 0.65 * noise.next().abs() } else { 0.0 };
                hold = 2400 + (noise.next().abs() * 9600.0) as usize;
            }
            hold -= 1;
            loudness += (target - loudness) * 0.002;
            let white = noise.next();
            low += 0.12 * (white - low);
            mid += 0.5 * (white - mid);
            out.push(level * loudness * (2.2 * low + 0.35 * mid));
        }
        out
    }

    fn room(seed: u64, delay: usize, tail: usize, gain: f32) -> Vec<f32> {
        let mut noise = Noise(seed.wrapping_mul(0xD1B5_4A32_D192_ED03) | 1);
        let mut taps = vec![0.0f32; delay + tail];
        let mut total = 0.0;
        for n in 0..tail {
            let tap = noise.next() * (-(n as f32) * 5.0 / tail as f32).exp();
            taps[delay + n] = tap;
            total += tap * tap;
        }
        let scale = gain / total.sqrt();
        taps.iter_mut().for_each(|tap| *tap *= scale);
        taps
    }

    fn through(signal: &[f32], taps: &[f32]) -> Vec<f32> {
        let first = taps.iter().position(|tap| *tap != 0.0).unwrap_or(0);
        let live = &taps[first..];
        let mut out = vec![0.0f32; signal.len()];
        for (n, slot) in out.iter_mut().enumerate() {
            if n < first {
                continue;
            }
            let newest = n - first;
            let reach = live.len().min(newest + 1);
            let mut sum = 0.0f32;
            for (k, tap) in live.iter().enumerate().take(reach) {
                sum += tap * signal[newest - k];
            }
            *slot = sum;
        }
        out
    }

    fn hiss(seed: u64, samples: usize, level: f32) -> Vec<f32> {
        let mut noise = Noise(seed | 1);
        (0..samples).map(|_| level * noise.next()).collect()
    }

    fn add(a: &[f32], b: &[f32]) -> Vec<f32> {
        a.iter().zip(b.iter()).map(|(x, y)| x + y).collect()
    }

    fn run(canceller: &mut EchoCanceller, far: &[f32], mic: &[f32]) -> Vec<f32> {
        let mut out = Vec::with_capacity(mic.len());
        for (played, heard) in far.chunks(960).zip(mic.chunks(960)) {
            canceller.push_far(played);
            let mut frame = heard.to_vec();
            canceller.process(&mut frame);
            out.extend_from_slice(&frame);
        }
        out
    }

    fn reference(mic: &[f32]) -> Vec<f32> {
        let mut out = vec![0.0f32; mic.len()];
        let (mut before, mut after) = (0.0f32, 0.0f32);
        for n in 0..mic.len() {
            let value = mic[n] - before + DC_POLE * after;
            before = mic[n];
            after = value;
            if n + ECHO_DELAY_SAMPLES < mic.len() {
                out[n + ECHO_DELAY_SAMPLES] = value;
            }
        }
        out
    }

    fn db(ratio: f32) -> f32 {
        10.0 * ratio.max(1e-12).log10()
    }

    fn power(signal: &[f32]) -> f32 {
        energy(signal) / signal.len().max(1) as f32
    }

    fn apart(a: &[f32], b: &[f32]) -> f32 {
        a.iter().zip(b.iter()).map(|(x, y)| (x - y) * (x - y)).sum::<f32>() / a.len().max(1) as f32
    }

    const SECOND: usize = 48_000;

    #[test]
    fn the_transform_round_trips() {
        let mut fft = Fft::new();
        let mut noise = Noise(77);
        let time: Vec<f32> = (0..N).map(|_| noise.next()).collect();
        let (mut re, mut im, mut back) = (vec![0.0; K], vec![0.0; K], vec![0.0; N]);
        fft.forward(&time, &mut re, &mut im);
        fft.inverse(&re, &im, &mut back);
        assert!(apart(&time, &back) < 1e-10);
        let tone: Vec<f32> = (0..N).map(|n| (2.0 * PI * 8.0 * n as f32 / N as f32).cos()).collect();
        fft.forward(&tone, &mut re, &mut im);
        assert!((re[8] - 0.5).abs() < 1e-5 && im[8].abs() < 1e-5);
        assert!(re.iter().enumerate().all(|(k, v)| k == 8 || v.abs() < 1e-5));
    }

    #[test]
    fn my_voice_passes_untouched_when_nothing_plays() {
        let mine = voice(2, 3 * SECOND, 0.3);
        let silence = vec![0.0f32; mine.len()];
        let mut canceller = EchoCanceller::new();
        let out = run(&mut canceller, &silence, &mine);
        let expected = reference(&mine);
        assert!(db(apart(&out, &expected) / power(&expected)) < -80.0);
        assert_eq!(canceller.reduction_db(), None);
        assert!(!canceller.is_converged());
    }

    #[test]
    fn an_echo_is_removed() {
        let far = voice(1, 9 * SECOND, 0.3);
        let echo = through(&far, &room(3, 1920, 1440, 0.5));
        let mic = add(&echo, &hiss(5, far.len(), 0.0003));
        let mut canceller = EchoCanceller::new();
        let out = run(&mut canceller, &far, &mic);
        let early = db(power(&mic[SECOND..2 * SECOND]) / power(&out[SECOND..2 * SECOND]));
        let settled = db(power(&mic[6 * SECOND..]) / power(&out[6 * SECOND..]));
        let filter_only = db(power(&mic[6 * SECOND..8 * SECOND]) / power(&canceller.trace[6 * SECOND..8 * SECOND]));
        println!("echo removed: {early:.1} dB in the second second, {settled:.1} dB once settled ({filter_only:.1} dB by the filter alone)");
        assert!(filter_only > 18.0, "the filter alone removes only {filter_only:.1} dB");
        assert!(canceller.is_converged());
        assert!(settled > 25.0, "only {settled:.1} dB");
        assert!(early > 8.0, "only {early:.1} dB early on");
        let shown = canceller.reduction_db().unwrap();
        assert!((shown - settled).abs() < 12.0, "read-out {shown:.1} dB against {settled:.1} dB");
        assert_eq!(canceller.hops(), 0);
        assert_eq!(canceller.clock_drift_ppm(), 0.0);
    }

    #[test]
    fn my_voice_survives_talking_over_the_echo() {
        let far = voice(1, 11 * SECOND, 0.3);
        let echo = through(&far, &room(3, 1920, 1440, 0.5));
        let mut mine = voice(9, far.len(), 0.3);
        mine[..6 * SECOND].iter_mut().for_each(|s| *s = 0.0);
        let mic = add(&add(&echo, &mine), &hiss(5, far.len(), 0.0003));
        let mut canceller = EchoCanceller::new();
        let out = run(&mut canceller, &far, &mic);
        let expected = reference(&mine);
        let span = 7 * SECOND..far.len();
        let wrong = db(apart(&out[span.clone()], &expected[span.clone()]) / power(&expected[span.clone()]));
        let untreated = db(power(&echo[span.clone()]) / power(&expected[span.clone()]));
        let kept = db(power(&out[span.clone()]) / power(&expected[span]));
        let plain: Vec<f32> = reference(&mine)[ECHO_DELAY_SAMPLES..].to_vec();
        let filter_wrong = db(apart(&canceller.trace[7 * SECOND..10 * SECOND], &plain[7 * SECOND..10 * SECOND]) / power(&plain[7 * SECOND..10 * SECOND]));
        println!("talking over the echo: echo was {untreated:.1} dB against my voice, what is left wrong is {wrong:.1} dB ({filter_wrong:.1} dB after the filter alone), my level changed {kept:.1} dB");
        assert!(wrong < untreated - 8.0, "{wrong:.1} dB wrong, the echo alone was {untreated:.1} dB");
        assert!(kept > -3.0 && kept < 1.5, "my voice changed by {kept:.1} dB");
    }

    #[test]
    fn a_changed_room_is_learned_again() {
        let far = voice(4, 12 * SECOND, 0.3);
        let first = through(&far, &room(3, 1920, 1440, 0.5));
        let second = through(&far, &room(8, 2600, 1440, 0.4));
        let mut mic = first;
        mic[6 * SECOND..].copy_from_slice(&second[6 * SECOND..]);
        let mic = add(&mic, &hiss(5, far.len(), 0.0003));
        let mut canceller = EchoCanceller::new();
        let out = run(&mut canceller, &far, &mic);
        let before = db(power(&mic[4 * SECOND..6 * SECOND]) / power(&out[4 * SECOND..6 * SECOND]));
        let after = db(power(&mic[10 * SECOND..]) / power(&out[10 * SECOND..]));
        println!("room changed: {before:.1} dB before, {after:.1} dB two seconds after settling again");
        assert!(before > 25.0 && after > 22.0, "{before:.1} dB then {after:.1} dB");
    }

    #[test]
    fn short_and_long_delays_are_covered() {
        for delay in [720usize, 6000, 12_000] {
            let far = voice(6, 9 * SECOND, 0.3);
            let mic = add(&through(&far, &room(3, delay, 960, 0.4)), &hiss(5, far.len(), 0.0003));
            let mut canceller = EchoCanceller::new();
            let out = run(&mut canceller, &far, &mic);
            let settled = db(power(&mic[6 * SECOND..]) / power(&out[6 * SECOND..]));
            println!("delay of {} ms: {settled:.1} dB", delay / 48);
            assert!(settled > 22.0, "delay {delay}: only {settled:.1} dB");
        }
        assert!(ECHO_TAIL_MS > 300);
    }

    #[test]
    fn sound_that_is_not_an_echo_is_left_alone() {
        let far = voice(1, 8 * SECOND, 0.3);
        let mine = voice(9, far.len(), 0.3);
        let mic = add(&mine, &hiss(5, far.len(), 0.0003));
        let mut canceller = EchoCanceller::new();
        let out = run(&mut canceller, &far, &mic);
        let expected = reference(&mic);
        let span = 3 * SECOND..far.len();
        let wrong = db(apart(&out[span.clone()], &expected[span.clone()]) / power(&expected[span.clone()]));
        let kept = db(power(&out[span.clone()]) / power(&expected[span]));
        println!("headphones: {wrong:.1} dB of my voice is wrong, level changed {kept:.1} dB");
        assert!(wrong < -15.0, "{wrong:.1} dB");
        assert!(kept.abs() < 1.0, "{kept:.1} dB");
    }

    #[test]
    fn a_late_or_early_reference_does_not_lose_what_was_learned() {
        let far = voice(1, 16 * SECOND, 0.3);
        let mic = add(&through(&far, &room(3, 2400, 1440, 0.5)), &hiss(5, far.len(), 0.0003));
        let mut canceller = EchoCanceller::new();
        let mut out = Vec::with_capacity(mic.len());
        let mut pushed = 0usize;
        for (index, heard) in mic.chunks(960).enumerate() {
            let now = (index + 1) * 960;
            let wanted = match now / SECOND {
                0..=5 => now,
                6..=9 => now - 1100,
                _ => now + 1300,
            }
            .min(far.len());
            if wanted > pushed {
                canceller.push_far(&far[pushed..wanted]);
                pushed = wanted;
            }
            let mut frame = heard.to_vec();
            canceller.process(&mut frame);
            out.extend_from_slice(&frame);
        }
        let steady = db(power(&mic[4 * SECOND..6 * SECOND]) / power(&out[4 * SECOND..6 * SECOND]));
        let after_late = db(power(&mic[6 * SECOND + 4800..7 * SECOND]) / power(&out[6 * SECOND + 4800..7 * SECOND]));
        let after_early = db(power(&mic[14 * SECOND..]) / power(&out[14 * SECOND..]));
        println!(
            "reference timing: {steady:.1} dB steady, {after_late:.1} dB right after it ran late, {after_early:.1} dB after it ran early, {} hops",
            canceller.hops()
        );
        assert!(canceller.hops() >= 2 && canceller.hops() <= 16, "{} hops", canceller.hops());
        assert!(steady > 25.0 && after_late > 15.0 && after_early > 20.0);
    }

    #[test]
    fn a_stopped_reference_is_treated_as_silence_and_recovers() {
        let far = voice(1, 12 * SECOND, 0.3);
        let mic = add(&through(&far, &room(3, 1920, 1440, 0.5)), &hiss(5, far.len(), 0.0003));
        let mine = voice(9, far.len(), 0.3);
        let mut canceller = EchoCanceller::new();
        let mut out = Vec::with_capacity(mic.len());
        let mut fed = Vec::with_capacity(mic.len());
        for (index, (played, heard)) in far.chunks(960).zip(mic.chunks(960)).enumerate() {
            let stopped = (5 * 50..7 * 50).contains(&index);
            let mut frame = if stopped { mine[index * 960..index * 960 + heard.len()].to_vec() } else { heard.to_vec() };
            fed.extend_from_slice(&frame);
            if !stopped {
                canceller.push_far(played);
            }
            canceller.process(&mut frame);
            out.extend_from_slice(&frame);
        }
        let expected = reference(&fed);
        let quiet = 5 * SECOND + 24_000..7 * SECOND;
        let wrong = db(apart(&out[quiet.clone()], &expected[quiet.clone()]) / power(&expected[quiet]));
        let again = db(power(&mic[10 * SECOND..]) / power(&out[10 * SECOND..]));
        println!("reference stopped: my voice is {wrong:.1} dB wrong meanwhile; {again:.1} dB once it is back");
        assert!(wrong < -40.0, "{wrong:.1} dB");
        assert!(again > 22.0, "{again:.1} dB");
    }

    #[test]
    fn clocks_that_drift_apart_are_followed() {
        let far = voice(1, 20 * SECOND, 0.3);
        let slow: Vec<f32> = (0..far.len())
            .map(|n| {
                let at = n as f64 * (1.0 - 60e-6);
                let (whole, part) = (at as usize, (at - at.floor()) as f32);
                far[whole] * (1.0 - part) + far[(whole + 1).min(far.len() - 1)] * part
            })
            .collect();
        let mic = add(&through(&slow, &room(3, 1920, 1440, 0.5)), &hiss(5, far.len(), 0.0003));
        let mut canceller = EchoCanceller::new();
        let out = run(&mut canceller, &far, &mic);
        let settled = db(power(&mic[14 * SECOND..]) / power(&out[14 * SECOND..]));
        let filter_only = db(power(&mic[14 * SECOND..19 * SECOND]) / power(&canceller.trace[14 * SECOND..19 * SECOND]));
        println!(
            "60 parts per million of drift: {settled:.1} dB ({filter_only:.1} dB by the filter alone), measured {:.1} ppm",
            canceller.clock_drift_ppm()
        );
        assert!((canceller.clock_drift_ppm() + 60.0).abs() < 12.0, "measured {:.1} ppm", canceller.clock_drift_ppm());
        assert!(filter_only > 18.0, "the filter alone removes only {filter_only:.1} dB");
        assert!(settled > 25.0, "only {settled:.1} dB");
    }

    #[test]
    fn an_echo_that_arrives_very_late_is_found_and_lined_up() {
        let far = voice(1, 14 * SECOND, 0.3);
        let mic = add(&through(&far, &room(3, 24_000, 960, 0.5)), &hiss(5, far.len(), 0.0003));
        let mut canceller = EchoCanceller::new();
        let out = run(&mut canceller, &far, &mic);
        let settled = db(power(&mic[10 * SECOND..]) / power(&out[10 * SECOND..]));
        let found = canceller.echo_delay_ms().unwrap_or(0.0);
        println!("half a second of delay: found at {found:.0} ms, {settled:.1} dB once lined up, {} hops", canceller.hops());
        assert!((found - 500.0).abs() < 40.0, "found {found:.0} ms");
        assert!(settled > 22.0, "only {settled:.1} dB");
    }

    #[test]
    fn it_costs_a_small_part_of_one_processor() {
        let far = voice(1, 10 * SECOND, 0.3);
        let mic = add(&through(&far, &room(3, 1920, 960, 0.5)), &hiss(5, far.len(), 0.0003));
        let quiet = vec![0.0f32; far.len()];
        let mut canceller = EchoCanceller::new();
        let started = std::time::Instant::now();
        run(&mut canceller, &far, &mic);
        let busy = started.elapsed().as_secs_f32() / 10.0;
        let started = std::time::Instant::now();
        run(&mut canceller, &quiet, &mic);
        let idle = started.elapsed().as_secs_f32() / 10.0;
        println!("cost: {:.1}% of one processor while something plays, {:.2}% while nothing does", busy * 100.0, idle * 100.0);
        assert!(busy < 0.25 && idle < 0.05, "{busy} busy, {idle} idle");
    }

    #[test]
    fn loud_and_broken_input_does_not_break_it() {
        let far = voice(1, 4 * SECOND, 4.0);
        let mut mic = through(&far, &room(3, 1920, 960, 3.0));
        mic.iter_mut().for_each(|s| *s = s.clamp(-1.0, 1.0));
        mic[SECOND] = f32::NAN;
        mic[2 * SECOND] = f32::INFINITY;
        let mut canceller = EchoCanceller::new();
        let out = run(&mut canceller, &far, &mic);
        assert!(out[SECOND + 4800..].iter().all(|s| s.is_finite() && s.abs() <= 1.0));
        let fine = voice(2, SECOND, 0.3);
        let silence = vec![0.0f32; fine.len()];
        canceller.reset();
        let out = run(&mut canceller, &silence, &fine);
        assert!(db(apart(&out, &reference(&fine)) / power(&reference(&fine))) < -80.0);
    }
}
