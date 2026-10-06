use std::collections::{BTreeMap, HashMap, VecDeque};

use crate::codec::{is_supported_codec, Decoder, FRAME_SAMPLES};

pub const MIX_CHANNELS: usize = 2;
pub const BLOCK: usize = FRAME_SAMPLES * MIX_CHANNELS;

const MIN_TARGET: usize = 2;
const DEFAULT_TARGET: usize = 3;
const MAX_TARGET: usize = 12;
const MAX_QUEUE: usize = 60;
const MAX_STRETCH: u32 = 6;
const MAX_GAP: u32 = 12;
const WINDOW_PULLS: u32 = 250;
const CALM_WINDOWS: u32 = 6;
const IDLE_PULLS_BEFORE_DROP: u32 = 3000;
const RESYNC_DISTANCE: i32 = 2000;
const CONCEAL_GAINS: [f32; 6] = [1.0, 0.8, 0.55, 0.3, 0.1, 0.0];
const BLEND_FRAMES: usize = 96;
const MAX_PLAUSIBLE_RMS: f32 = 0.7;

fn frame_rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StreamState {
    Idle,
    Buffering,
    Playing,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TalkerStats {
    pub decoded: u64,
    pub concealed: u64,
    pub lost: u64,
    pub late: u64,
    pub skipped: u64,
    pub underruns: u64,
}

pub struct Talker {
    decoder: Decoder,
    queue: BTreeMap<u32, Vec<u8>>,
    state: StreamState,
    next_seq: u32,
    last_ext: u32,
    target: usize,
    ended: bool,
    stretched: u32,
    waited: u32,
    pcm: VecDeque<f32>,
    frame_samples: usize,
    scratch: Vec<f32>,
    window_pulls: u32,
    window_min_queue: usize,
    calm_windows: u32,
    idle_pulls: u32,
    last_good: Vec<f32>,
    last_rms: f32,
    conceal_run: usize,
    blend_from: Option<[f32; MIX_CHANNELS]>,
    celt_only: bool,
    pub volume: f32,
    pub stats: TalkerStats,
}

impl Talker {
    pub fn new() -> Result<Self, String> {
        Ok(Self {
            decoder: Decoder::new(MIX_CHANNELS)?,
            queue: BTreeMap::new(),
            state: StreamState::Idle,
            next_seq: 0,
            last_ext: 0,
            target: DEFAULT_TARGET,
            ended: false,
            stretched: 0,
            waited: 0,
            pcm: VecDeque::new(),
            frame_samples: FRAME_SAMPLES,
            scratch: Vec::new(),
            window_pulls: 0,
            window_min_queue: usize::MAX,
            calm_windows: 0,
            idle_pulls: 0,
            last_good: Vec::new(),
            last_rms: 0.0,
            conceal_run: 0,
            blend_from: None,
            celt_only: false,
            volume: 1.0,
            stats: TalkerStats::default(),
        })
    }

    pub fn is_active(&self) -> bool {
        self.state != StreamState::Idle
    }

    pub fn is_playing(&self) -> bool {
        self.state == StreamState::Playing
    }

    pub fn target_frames(&self) -> usize {
        self.target
    }

    pub fn queued(&self) -> usize {
        self.queue.len()
    }

    fn begin(&mut self, seq: u16, data: &[u8]) {
        self.decoder.reset();
        self.queue.clear();
        self.pcm.clear();
        self.ended = false;
        self.stretched = 0;
        self.waited = 0;
        self.window_pulls = 0;
        self.window_min_queue = usize::MAX;
        self.last_good.clear();
        self.last_rms = 0.0;
        self.conceal_run = 0;
        self.blend_from = None;
        self.last_ext = 0x0010_0000 + seq as u32;
        self.queue.insert(self.last_ext, data.to_vec());
        self.state = StreamState::Buffering;
    }

    pub fn push(&mut self, seq: u16, data: &[u8]) {
        self.idle_pulls = 0;
        if data.is_empty() {
            if self.state != StreamState::Idle {
                self.ended = true;
            }
            return;
        }
        if self.state == StreamState::Idle {
            self.begin(seq, data);
            return;
        }
        let diff = seq.wrapping_sub(self.last_ext as u16) as i16 as i32;
        if diff.abs() > RESYNC_DISTANCE {
            self.begin(seq, data);
            return;
        }
        let ext = (self.last_ext as i64 + diff as i64) as u32;
        if diff > 0 {
            self.last_ext = ext;
        }
        self.ended = false;
        if self.state == StreamState::Playing && ext < self.next_seq {
            self.stats.late += 1;
            return;
        }
        self.queue.entry(ext).or_insert_with(|| data.to_vec());
        while self.queue.len() > MAX_QUEUE {
            if let Some((oldest, _)) = self.queue.pop_first() {
                self.stats.skipped += 1;
                if self.state == StreamState::Playing && oldest >= self.next_seq {
                    self.next_seq = oldest + 1;
                }
            }
        }
    }

    pub fn pull(&mut self, out: &mut [f32]) -> bool {
        self.idle_pulls = self.idle_pulls.saturating_add(1);
        match self.state {
            StreamState::Idle => return false,
            StreamState::Buffering => {
                self.waited += 1;
                let ready = self.queue.len() >= self.target
                    || (self.ended && !self.queue.is_empty())
                    || (self.waited as usize > self.target + 2 && !self.queue.is_empty());
                if ready {
                    self.state = StreamState::Playing;
                    self.next_seq = *self.queue.keys().next().expect("queue is not empty");
                } else {
                    if self.queue.is_empty() && (self.ended || self.waited > 50) {
                        self.state = StreamState::Idle;
                    }
                    return false;
                }
            }
            StreamState::Playing => {}
        }
        while self.pcm.len() < out.len() {
            if !self.decode_next() {
                break;
            }
        }
        if self.pcm.is_empty() {
            return false;
        }
        for slot in out.iter_mut() {
            *slot = self.pcm.pop_front().unwrap_or(0.0);
        }
        self.adapt();
        true
    }

    fn adapt(&mut self) {
        self.window_min_queue = self.window_min_queue.min(self.queue.len());
        self.window_pulls += 1;
        if self.window_pulls < WINDOW_PULLS {
            return;
        }
        if self.window_min_queue > self.target && self.window_min_queue != usize::MAX {
            if self.queue.remove(&self.next_seq).is_some() {
                self.next_seq += 1;
                self.stats.skipped += 1;
            }
        }
        self.calm_windows += 1;
        if self.calm_windows >= CALM_WINDOWS && self.target > MIN_TARGET {
            self.target -= 1;
            self.calm_windows = 0;
        }
        self.window_pulls = 0;
        self.window_min_queue = usize::MAX;
    }

    fn append_decoded(&mut self, samples: usize) {
        self.pcm.extend(self.scratch[..samples * MIX_CHANNELS].iter().copied());
    }

    fn accept_good_frame(&mut self, samples: usize) {
        let len = samples * MIX_CHANNELS;
        if let Some(from) = self.blend_from.take() {
            let frames = BLEND_FRAMES.min(samples);
            for i in 0..frames {
                let w = i as f32 / frames as f32;
                for c in 0..MIX_CHANNELS {
                    let s = &mut self.scratch[i * MIX_CHANNELS + c];
                    *s = *s * w + from[c] * (1.0 - w);
                }
            }
        }
        self.last_good.clear();
        self.last_good.extend_from_slice(&self.scratch[..len]);
        self.last_rms = frame_rms(&self.scratch[..len]);
        self.conceal_run = 0;
        self.append_decoded(samples);
    }

    fn plausible(&self, samples: usize) -> bool {
        let rms = frame_rms(&self.scratch[..samples * MIX_CHANNELS]);
        rms.is_finite() && rms <= MAX_PLAUSIBLE_RMS && rms <= 2.0 * self.last_rms + 0.02
    }

    fn synthesize_concealment(&mut self) {
        let frames = self.frame_samples;
        let len = frames * MIX_CHANNELS;
        self.scratch.clear();
        self.scratch.resize(len, 0.0);
        let run = self.conceal_run;
        let start_gain = CONCEAL_GAINS[run.min(CONCEAL_GAINS.len() - 1)];
        let end_gain = CONCEAL_GAINS[(run + 1).min(CONCEAL_GAINS.len() - 1)];
        if self.last_good.len() == len && start_gain > 0.0 {
            let reversed = run % 2 == 0;
            for i in 0..frames {
                let src = if reversed { frames - 1 - i } else { i };
                let gain = start_gain + (end_gain - start_gain) * (i as f32 / frames as f32);
                for c in 0..MIX_CHANNELS {
                    self.scratch[i * MIX_CHANNELS + c] = self.last_good[src * MIX_CHANNELS + c] * gain;
                }
            }
        }
        let mut tail = [0f32; MIX_CHANNELS];
        tail.copy_from_slice(&self.scratch[len - MIX_CHANNELS..len]);
        self.blend_from = Some(tail);
        self.conceal_run += 1;
        self.append_decoded(frames);
    }

    fn finish_concealment(&mut self, decoded: usize) {
        if decoded == self.frame_samples && self.conceal_run == 0 && self.plausible(decoded) {
            let len = decoded * MIX_CHANNELS;
            let mut tail = [0f32; MIX_CHANNELS];
            tail.copy_from_slice(&self.scratch[len - MIX_CHANNELS..len]);
            self.blend_from = Some(tail);
            self.conceal_run += 1;
            self.append_decoded(decoded);
        } else {
            self.synthesize_concealment();
        }
        self.stats.concealed += 1;
    }

    fn conceal(&mut self) {
        if !self.celt_only {
            self.synthesize_concealment();
            self.stats.concealed += 1;
            return;
        }
        self.scratch.clear();
        self.scratch.resize(self.frame_samples * MIX_CHANNELS, 0.0);
        let n = self.decoder.conceal(&mut self.scratch).unwrap_or(0);
        self.finish_concealment(n);
    }

    fn decode_next(&mut self) -> bool {
        if let Some(data) = self.queue.remove(&self.next_seq) {
            let samples = Decoder::packet_samples(&data).unwrap_or(FRAME_SAMPLES);
            self.celt_only = data[0] >> 3 >= 16;
            self.scratch.clear();
            self.scratch.resize(samples * MIX_CHANNELS, 0.0);
            match self.decoder.decode(&data, false, &mut self.scratch) {
                Ok(n) if n > 0 => {
                    self.frame_samples = n;
                    self.accept_good_frame(n);
                    self.stats.decoded += 1;
                }
                _ => self.conceal(),
            }
            self.next_seq += 1;
            self.stretched = 0;
            return true;
        }
        let Some(&first) = self.queue.keys().next() else {
            if self.ended {
                self.state = StreamState::Idle;
                return false;
            }
            if self.stretched >= MAX_STRETCH {
                self.state = StreamState::Idle;
                return false;
            }
            if self.stretched == 0 {
                self.stats.underruns += 1;
                self.target = (self.target + 1).min(MAX_TARGET);
                self.calm_windows = 0;
                self.window_pulls = 0;
                self.window_min_queue = usize::MAX;
            }
            self.stretched += 1;
            self.conceal();
            return true;
        };
        if first < self.next_seq || first - self.next_seq > MAX_GAP {
            self.stats.lost += first.saturating_sub(self.next_seq) as u64;
            self.next_seq = first;
            return self.decode_next();
        }
        self.stats.lost += 1;
        self.conceal();
        self.next_seq += 1;
        self.stretched = 0;
        true
    }
}

pub fn soft_limit(x: f32) -> f32 {
    const KNEE: f32 = 0.8;
    let a = x.abs();
    if a <= KNEE {
        x
    } else {
        let over = (a - KNEE) / (1.0 - KNEE);
        x.signum() * (KNEE + (1.0 - KNEE) * over.tanh())
    }
}

#[derive(Default)]
pub struct Playback {
    talkers: HashMap<u32, Talker>,
    volumes: HashMap<u32, f32>,
    block: Vec<f32>,
    unsupported_codec: Option<u8>,
}

impl Playback {
    pub fn new() -> Self {
        Self::default()
    }

    fn key(session: u16, client_id: u16) -> u32 {
        (u32::from(session) << 16) | u32::from(client_id)
    }

    pub fn push(&mut self, session: u16, client_id: u16, voice_id: u16, codec: u8, data: &[u8]) {
        let key = Self::key(session, client_id);
        if !is_supported_codec(codec) {
            if !data.is_empty() {
                self.unsupported_codec = Some(codec);
            }
            return;
        }
        if !self.talkers.contains_key(&key) {
            if data.is_empty() {
                return;
            }
            match Talker::new() {
                Ok(mut talker) => {
                    talker.volume = self.volumes.get(&key).copied().unwrap_or(1.0);
                    self.talkers.insert(key, talker);
                }
                Err(_) => return,
            }
        }
        if let Some(talker) = self.talkers.get_mut(&key) {
            talker.push(voice_id, data);
        }
    }

    pub fn take_unsupported_codec(&mut self) -> Option<u8> {
        self.unsupported_codec.take()
    }

    pub fn set_volume(&mut self, session: u16, client_id: u16, volume: f32) {
        let key = Self::key(session, client_id);
        self.volumes.insert(key, volume);
        if let Some(t) = self.talkers.get_mut(&key) {
            t.volume = volume;
        }
    }

    pub fn remove(&mut self, session: u16, client_id: u16) {
        self.talkers.remove(&Self::key(session, client_id));
    }

    pub fn clear_session(&mut self, session: u16) {
        self.talkers.retain(|key, _| (key >> 16) as u16 != session);
        self.volumes.retain(|key, _| (key >> 16) as u16 != session);
    }

    pub fn clear(&mut self) {
        self.talkers.clear();
    }

    pub fn talker(&self, session: u16, client_id: u16) -> Option<&Talker> {
        self.talkers.get(&Self::key(session, client_id))
    }

    pub fn active_talkers(&self) -> usize {
        self.talkers.values().filter(|t| t.is_playing()).count()
    }

    pub fn mix(&mut self, out: &mut [f32]) -> usize {
        for s in out.iter_mut() {
            *s = 0.0;
        }
        self.block.clear();
        self.block.resize(out.len(), 0.0);
        let mut active = 0;
        for talker in self.talkers.values_mut() {
            if talker.pull(&mut self.block) {
                active += 1;
                let gain = talker.volume;
                for (o, s) in out.iter_mut().zip(self.block.iter()) {
                    *o += *s * gain;
                }
            }
        }
        if active > 0 {
            for s in out.iter_mut() {
                *s = soft_limit(*s);
            }
        }
        self.talkers.retain(|_, t| t.idle_pulls < IDLE_PULLS_BEFORE_DROP);
        active
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::{Encoder, CODEC_OPUS_VOICE, MAX_PACKET_BYTES, SAMPLE_RATE};

    fn packets(count: usize, freq: f32) -> Vec<Vec<u8>> {
        let mut enc = Encoder::new(CODEC_OPUS_VOICE, 6).unwrap();
        let mut buf = [0u8; MAX_PACKET_BYTES];
        (0..count)
            .map(|f| {
                let pcm: Vec<f32> = (0..FRAME_SAMPLES)
                    .map(|i| {
                        let t = (f * FRAME_SAMPLES + i) as f32 / SAMPLE_RATE as f32;
                        0.4 * (2.0 * std::f32::consts::PI * freq * t).sin()
                    })
                    .collect();
                let n = enc.encode(&pcm, &mut buf).unwrap();
                buf[..n].to_vec()
            })
            .collect()
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
    }

    #[test]
    fn buffers_then_plays_in_order() {
        let data = packets(40, 440.0);
        let mut t = Talker::new().unwrap();
        let mut out = vec![0f32; BLOCK];
        assert!(!t.pull(&mut out));
        t.push(100, &data[0]);
        assert!(t.is_active());
        assert!(!t.pull(&mut out), "must wait for the target depth");
        t.push(101, &data[1]);
        t.push(102, &data[2]);
        let mut produced = Vec::new();
        for i in 3..40 {
            assert!(t.pull(&mut out));
            produced.extend_from_slice(&out);
            t.push(100 + i as u16, &data[i]);
        }
        assert!(t.is_playing());
        assert_eq!(t.stats.concealed, 0);
        assert_eq!(t.stats.lost, 0);
        assert_eq!(t.stats.decoded, 37);
        assert!((rms(&produced[BLOCK * 5..]) - 0.2828).abs() < 0.06);
    }

    #[test]
    fn loss_concealment_never_gets_louder_than_the_signal() {
        for quality in [2u8, 6, 10] {
            let mut enc = Encoder::new(CODEC_OPUS_VOICE, quality).unwrap();
            let mut buf = [0u8; MAX_PACKET_BYTES];
            let data: Vec<Vec<u8>> = (0..120)
                .map(|f| {
                    let pcm: Vec<f32> = (0..FRAME_SAMPLES)
                        .map(|i| {
                            let t = (f * FRAME_SAMPLES + i) as f32 / SAMPLE_RATE as f32;
                            0.4 * (2.0 * std::f32::consts::PI * 660.0 * t).sin()
                        })
                        .collect();
                    let n = enc.encode(&pcm, &mut buf).unwrap();
                    buf[..n].to_vec()
                })
                .collect();
            let mut t = Talker::new().unwrap();
            let mut out = vec![0f32; BLOCK];
            let mut produced = Vec::new();
            for (i, p) in data.iter().enumerate() {
                let dropped = i > 20 && (i % 4 == 1 || i % 11 == 5 || i % 11 == 6);
                if !dropped {
                    t.push(i as u16, p);
                }
                if i >= 3 && t.pull(&mut out) {
                    produced.extend_from_slice(&out);
                }
            }
            assert!(t.stats.lost > 20, "quality {quality}: lost {}", t.stats.lost);
            assert!(t.stats.concealed > 20);
            let peak = produced.iter().fold(0f32, |m, s| m.max(s.abs()));
            assert!(peak < 0.62, "quality {quality}: peak {peak}");
            for (n, frame) in produced.chunks(BLOCK).enumerate().skip(10) {
                assert!(rms(frame) < 0.36, "quality {quality}: frame {n} rms {}", rms(frame));
            }
            let overall = rms(&produced[BLOCK * 10..]);
            assert!(overall > 0.15 && overall < 0.30, "quality {quality}: overall rms {overall}");
        }
    }

    #[test]
    fn long_outage_fades_to_silence() {
        let data = packets(10, 500.0);
        let mut t = Talker::new().unwrap();
        let mut out = vec![0f32; BLOCK];
        for (i, p) in data.iter().enumerate() {
            t.push(i as u16, p);
        }
        for _ in 0..10 {
            assert!(t.pull(&mut out));
        }
        let mut levels = Vec::new();
        while t.pull(&mut out) {
            levels.push(rms(&out));
            assert!(levels.len() < 20);
        }
        assert_eq!(levels.len(), MAX_STRETCH as usize);
        assert!(levels[0] < 0.36);
        for pair in levels.windows(2) {
            assert!(pair[1] <= pair[0] + 0.01, "levels must not grow: {levels:?}");
        }
        assert!(*levels.last().unwrap() < 0.05, "{levels:?}");
    }

    #[test]
    fn end_marker_drains_and_stops() {
        let data = packets(6, 300.0);
        let mut t = Talker::new().unwrap();
        let mut out = vec![0f32; BLOCK];
        for (i, p) in data.iter().enumerate() {
            t.push(i as u16, p);
        }
        t.push(6, &[]);
        let mut frames = 0;
        while t.pull(&mut out) {
            frames += 1;
            assert!(frames < 20);
        }
        assert_eq!(frames, 6);
        assert!(!t.is_active());
        assert_eq!(t.stats.concealed, 0);

        t.push(7, &data[0]);
        t.push(8, &[]);
        assert!(t.pull(&mut out), "a short burst still plays after the end marker");
        assert!(!t.pull(&mut out));
    }

    #[test]
    fn reorders_and_conceals_loss() {
        let data = packets(30, 500.0);
        let mut t = Talker::new().unwrap();
        let mut out = vec![0f32; BLOCK];
        let order = [0usize, 2, 1, 3, 4, 6, 7, 8, 9, 10, 11, 12];
        for i in order {
            t.push(i as u16, &data[i]);
        }
        let mut frames = 0;
        for _ in 0..12 {
            if t.pull(&mut out) {
                frames += 1;
            }
        }
        assert_eq!(frames, 12);
        assert_eq!(t.stats.lost, 1);
        assert_eq!(t.stats.decoded, 11);
        assert!(t.stats.concealed >= 1);
        assert_eq!(t.stats.late, 0);

        t.push(5, &data[5]);
        assert_eq!(t.stats.late, 1);
    }

    #[test]
    fn underrun_stretches_then_recovers_without_dropping_audio() {
        let data = packets(20, 600.0);
        let mut t = Talker::new().unwrap();
        let mut out = vec![0f32; BLOCK];
        for i in 0..3 {
            t.push(i as u16, &data[i]);
        }
        for _ in 0..3 {
            assert!(t.pull(&mut out));
        }
        let before = t.target_frames();
        assert!(t.pull(&mut out), "concealment covers the hole");
        assert_eq!(t.stats.underruns, 1);
        assert_eq!(t.target_frames(), before + 1);
        for i in 3..10 {
            t.push(i as u16, &data[i]);
        }
        let mut frames = 0;
        for _ in 0..7 {
            if t.pull(&mut out) {
                frames += 1;
            }
        }
        assert_eq!(frames, 7);
        assert_eq!(t.stats.late, 0);
        assert_eq!(t.stats.decoded, 10);
    }

    #[test]
    fn stalled_stream_goes_idle_and_restarts() {
        let data = packets(8, 600.0);
        let mut t = Talker::new().unwrap();
        let mut out = vec![0f32; BLOCK];
        for i in 0..3 {
            t.push(i as u16, &data[i]);
        }
        let mut pulls = 0;
        while t.pull(&mut out) {
            pulls += 1;
            assert!(pulls < 50);
        }
        assert_eq!(pulls, 3 + MAX_STRETCH as usize);
        assert!(!t.is_active());
        t.push(50, &data[3]);
        assert!(t.is_active() && !t.is_playing());
    }

    #[test]
    fn sequence_wrap_and_resync() {
        let data = packets(12, 700.0);
        let mut t = Talker::new().unwrap();
        let mut out = vec![0f32; BLOCK];
        let seqs = [65533u16, 65534, 65535, 0, 1, 2, 3, 4];
        for (i, s) in seqs.iter().enumerate() {
            t.push(*s, &data[i]);
        }
        let mut frames = 0;
        for _ in 0..8 {
            if t.pull(&mut out) {
                frames += 1;
            }
        }
        assert_eq!(frames, 8);
        assert_eq!(t.stats.lost, 0);
        assert_eq!(t.stats.decoded, 8);

        t.push(30_000, &data[8]);
        assert!(!t.is_playing(), "a huge jump restarts buffering");
    }

    #[test]
    fn burst_is_trimmed_to_bound_latency() {
        let data = packets(100, 440.0);
        let mut t = Talker::new().unwrap();
        for (i, p) in data.iter().enumerate() {
            t.push(i as u16, p);
        }
        assert!(t.queued() <= MAX_QUEUE);
        assert!(t.stats.skipped >= 40);
    }

    #[test]
    fn mixer_sums_talkers_and_limits() {
        let a = packets(30, 440.0);
        let b = packets(30, 880.0);
        let mut p = Playback::new();
        let mut out = vec![0f32; BLOCK];
        assert_eq!(p.mix(&mut out), 0);
        assert!(out.iter().all(|s| *s == 0.0));
        for i in 0..30 {
            p.push(0, 1, i as u16, CODEC_OPUS_VOICE, &a[i]);
            p.push(0, 2, i as u16, CODEC_OPUS_VOICE, &b[i]);
        }
        p.push(0, 3, 0, 2, &[1, 2, 3]);
        assert_eq!(p.take_unsupported_codec(), Some(2));
        assert_eq!(p.take_unsupported_codec(), None);
        p.set_volume(0, 2, 0.0);
        let mut solo = Vec::new();
        for _ in 0..10 {
            assert_eq!(p.mix(&mut out), 2);
            solo.extend_from_slice(&out);
        }
        p.set_volume(0, 2, 1.0);
        let mut both = Vec::new();
        for _ in 0..10 {
            assert_eq!(p.mix(&mut out), 2);
            both.extend_from_slice(&out);
        }
        assert_eq!(p.active_talkers(), 2);
        assert!(rms(&both) > rms(&solo) * 1.2);
        assert!(both.iter().all(|s| s.abs() <= 1.0));
        p.remove(0, 1);
        assert_eq!(p.mix(&mut out), 1);
        p.clear();
        assert_eq!(p.mix(&mut out), 0);
    }

    #[test]
    fn two_connections_can_share_a_client_number() {
        let a = packets(30, 440.0);
        let b = packets(30, 880.0);
        let mut p = Playback::new();
        let mut out = vec![0f32; BLOCK];
        for i in 0..30 {
            p.push(1, 7, i as u16, CODEC_OPUS_VOICE, &a[i]);
            p.push(2, 7, i as u16, CODEC_OPUS_VOICE, &b[i]);
        }
        for _ in 0..10 {
            assert_eq!(p.mix(&mut out), 2);
        }
        assert_eq!(p.talker(1, 7).unwrap().stats.decoded, 10);
        assert_eq!(p.talker(2, 7).unwrap().stats.decoded, 10);
        assert!(p.talker(0, 7).is_none());
        p.set_volume(1, 7, 0.5);
        p.clear_session(1);
        assert!(p.talker(1, 7).is_none());
        assert_eq!(p.mix(&mut out), 1);
        assert_eq!(p.talker(2, 7).unwrap().stats.decoded, 11);
        p.remove(2, 7);
        assert_eq!(p.mix(&mut out), 0);
    }

    #[test]
    fn limiter_is_transparent_below_the_knee() {
        assert_eq!(soft_limit(0.5), 0.5);
        assert_eq!(soft_limit(-0.8), -0.8);
        assert!(soft_limit(1.5) < 1.0 && soft_limit(1.5) > 0.9);
        assert!(soft_limit(-10.0) >= -1.0);
        assert!(soft_limit(0.9) > 0.8 && soft_limit(0.9) < 0.9);
    }
}
