use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};

use crate::codec::{is_end_marker, is_supported_codec, Decoder, FRAME_SAMPLES};

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
const DRAIN_GAINS: [f32; 6] = [1.0, 0.45, 0.12, 0.0, 0.0, 0.0];
const BLEND_FRAMES: usize = 96;
const FADE_FRAMES: usize = 384;
const STRAGGLER_PULLS: u32 = 50;
const STRAGGLER_REACH: i32 = 64;
const MAX_MARKERS: usize = 64;
const MAX_PLAUSIBLE_RMS: f32 = 0.7;

fn frame_rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
}

fn fade(samples: &mut [f32], rising: bool) {
    let frames = samples.len() / MIX_CHANNELS;
    let span = FADE_FRAMES.min(frames);
    for i in 0..span {
        let gain = 0.5 - 0.5 * (std::f32::consts::PI * (i as f32 + 0.5) / span as f32).cos();
        let at = if rising { i } else { frames - 1 - i };
        for sample in &mut samples[at * MIX_CHANNELS..(at + 1) * MIX_CHANNELS] {
            *sample *= gain;
        }
    }
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
    markers: BTreeSet<u32>,
    state: StreamState,
    next_seq: u32,
    last_ext: u32,
    target: usize,
    started: bool,
    ended: bool,
    end_at: u32,
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
    recent: Vec<f32>,
    faded: bool,
    draining: bool,
    celt_only: bool,
    pub volume: f32,
    pub stats: TalkerStats,
}

impl Talker {
    pub fn new() -> Result<Self, String> {
        Ok(Self {
            decoder: Decoder::new(MIX_CHANNELS)?,
            queue: BTreeMap::new(),
            markers: BTreeSet::new(),
            state: StreamState::Idle,
            next_seq: 0,
            last_ext: 0,
            target: DEFAULT_TARGET,
            started: false,
            ended: false,
            end_at: 0,
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
            recent: Vec::new(),
            faded: false,
            draining: false,
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
        self.markers.clear();
        self.pcm.clear();
        self.started = true;
        self.ended = false;
        self.end_at = 0;
        self.stretched = 0;
        self.waited = 0;
        self.window_pulls = 0;
        self.window_min_queue = usize::MAX;
        self.last_good.clear();
        self.last_rms = 0.0;
        self.conceal_run = 0;
        self.blend_from = None;
        self.recent.clear();
        self.faded = false;
        self.draining = false;
        self.last_ext = 0x0010_0000 + seq as u32;
        self.queue.insert(self.last_ext, data.to_vec());
        self.state = StreamState::Buffering;
    }

    pub fn push(&mut self, seq: u16, data: &[u8]) {
        let quiet_for = self.idle_pulls;
        self.idle_pulls = 0;
        let diff = i32::from(seq.wrapping_sub(self.last_ext as u16) as i16);
        if is_end_marker(data) {
            if self.state != StreamState::Idle {
                if diff > 0 && diff <= RESYNC_DISTANCE {
                    self.last_ext += diff as u32;
                    self.markers.insert(self.last_ext);
                    if self.markers.len() > MAX_MARKERS {
                        self.markers.pop_first();
                    }
                }
                self.ended = true;
                self.end_at = self.last_ext;
            }
            return;
        }
        if self.state == StreamState::Idle {
            let stale = self.started && quiet_for < STRAGGLER_PULLS && diff < 0 && diff > -STRAGGLER_REACH;
            if stale {
                self.stats.late += 1;
            } else {
                self.begin(seq, data);
            }
            return;
        }
        if diff.abs() > RESYNC_DISTANCE {
            self.begin(seq, data);
            return;
        }
        let ext = (self.last_ext as i64 + diff as i64) as u32;
        if diff > 0 {
            self.last_ext = ext;
        }
        if self.ended && ext > self.end_at {
            self.ended = false;
        }
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
        let len = samples * MIX_CHANNELS;
        self.pcm.extend(self.scratch[..len].iter().copied());
        let keep = FADE_FRAMES.min(samples) * MIX_CHANNELS;
        self.recent.clear();
        self.recent.extend_from_slice(&self.scratch[len - keep..len]);
        self.faded = false;
    }

    fn append_tail(&mut self) {
        let frames = self.recent.len() / MIX_CHANNELS;
        for i in 0..frames {
            let gain = 0.5 + 0.5 * (std::f32::consts::PI * (i as f32 + 0.5) / frames as f32).cos();
            let from = (frames - 1 - i) * MIX_CHANNELS;
            for c in 0..MIX_CHANNELS {
                self.pcm.push_back(self.recent[from + c] * gain);
            }
        }
        self.recent.clear();
        self.faded = true;
    }

    fn blend_in(&mut self, samples: usize) {
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
    }

    fn keep_tail(&mut self, samples: usize) {
        let len = samples * MIX_CHANNELS;
        let mut tail = [0f32; MIX_CHANNELS];
        tail.copy_from_slice(&self.scratch[len - MIX_CHANNELS..len]);
        self.blend_from = Some(tail);
    }

    fn accept_good_frame(&mut self, samples: usize, last: bool) {
        let len = samples * MIX_CHANNELS;
        self.blend_in(samples);
        self.last_good.clear();
        self.last_good.extend_from_slice(&self.scratch[..len]);
        self.last_rms = frame_rms(&self.scratch[..len]);
        self.conceal_run = 0;
        if self.faded {
            fade(&mut self.scratch[..len], true);
        }
        if last {
            fade(&mut self.scratch[..len], false);
        }
        self.append_decoded(samples);
        self.faded = last;
    }

    fn plausible(&self, samples: usize) -> bool {
        let rms = frame_rms(&self.scratch[..samples * MIX_CHANNELS]);
        rms.is_finite() && rms <= MAX_PLAUSIBLE_RMS && rms <= 2.0 * self.last_rms + 0.02
    }

    fn conceal_gains(&self) -> (f32, f32) {
        let gains = if self.draining { &DRAIN_GAINS } else { &CONCEAL_GAINS };
        let run = self.conceal_run;
        (gains[run.min(gains.len() - 1)], gains[(run + 1).min(gains.len() - 1)])
    }

    fn synthesize_concealment(&mut self) {
        let frames = self.frame_samples;
        let len = frames * MIX_CHANNELS;
        self.scratch.clear();
        self.scratch.resize(len, 0.0);
        let (start_gain, end_gain) = self.conceal_gains();
        if self.last_good.len() == len && start_gain > 0.0 {
            let reversed = self.conceal_run % 2 == 0;
            for i in 0..frames {
                let src = if reversed { frames - 1 - i } else { i };
                let gain = start_gain + (end_gain - start_gain) * (i as f32 / frames as f32);
                for c in 0..MIX_CHANNELS {
                    self.scratch[i * MIX_CHANNELS + c] = self.last_good[src * MIX_CHANNELS + c] * gain;
                }
            }
        }
        self.blend_in(frames);
        self.keep_tail(frames);
        self.conceal_run += 1;
        self.append_decoded(frames);
    }

    fn finish_concealment(&mut self, decoded: usize) {
        if decoded == self.frame_samples && self.conceal_run == 0 && self.plausible(decoded) {
            if self.draining {
                let (start_gain, end_gain) = self.conceal_gains();
                for i in 0..decoded {
                    let gain = start_gain + (end_gain - start_gain) * (i as f32 / decoded as f32);
                    for sample in &mut self.scratch[i * MIX_CHANNELS..(i + 1) * MIX_CHANNELS] {
                        *sample *= gain;
                    }
                }
            }
            self.blend_in(decoded);
            self.keep_tail(decoded);
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
        while self.markers.first().is_some_and(|marker| *marker <= self.next_seq) {
            if self.markers.pop_first() == Some(self.next_seq) {
                self.next_seq += 1;
            }
        }
        if let Some(data) = self.queue.remove(&self.next_seq) {
            let samples = Decoder::packet_samples(&data).unwrap_or(FRAME_SAMPLES);
            self.celt_only = data[0] >> 3 >= 16;
            self.scratch.clear();
            self.scratch.resize(samples * MIX_CHANNELS, 0.0);
            self.draining = false;
            match self.decoder.decode(&data, false, &mut self.scratch) {
                Ok(n) if n > 0 => {
                    self.frame_samples = n;
                    let last = self.ended && self.queue.is_empty();
                    self.accept_good_frame(n, last);
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
                if !self.faded && !self.recent.is_empty() {
                    self.append_tail();
                    return true;
                }
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
            self.draining = true;
            self.conceal();
            return true;
        };
        if first < self.next_seq || first - self.next_seq > MAX_GAP {
            self.stats.lost += first.saturating_sub(self.next_seq) as u64;
            self.next_seq = first;
            return self.decode_next();
        }
        self.stats.lost += 1;
        self.draining = false;
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
            if !is_end_marker(data) {
                self.unsupported_codec = Some(codec);
            }
            return;
        }
        if !self.talkers.contains_key(&key) {
            if is_end_marker(data) {
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
    use crate::codec::{Encoder, CODEC_OPUS_MUSIC, CODEC_OPUS_VOICE, MAX_PACKET_BYTES, SAMPLE_RATE};

    fn tone_packets(count: usize, freq: f32, codec: u8, quality: u8) -> Vec<Vec<u8>> {
        let mut enc = Encoder::new(codec, quality).unwrap();
        let channels = enc.channels();
        let mut buf = [0u8; MAX_PACKET_BYTES];
        (0..count)
            .map(|f| {
                let mut pcm = Vec::with_capacity(FRAME_SAMPLES * channels);
                for i in 0..FRAME_SAMPLES {
                    let t = (f * FRAME_SAMPLES + i) as f32 / SAMPLE_RATE as f32;
                    let sample = 0.4 * (2.0 * std::f32::consts::PI * freq * t).sin();
                    for _ in 0..channels {
                        pcm.push(sample);
                    }
                }
                let n = enc.encode(&pcm, &mut buf).unwrap();
                buf[..n].to_vec()
            })
            .collect()
    }

    fn packets(count: usize, freq: f32) -> Vec<Vec<u8>> {
        tone_packets(count, freq, CODEC_OPUS_VOICE, 6)
    }

    fn largest_step(samples: &[f32]) -> f32 {
        samples.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0, f32::max)
    }

    #[derive(Debug, Clone, Copy, PartialEq)]
    enum Ending {
        MarkerInTime,
        MarkerLate,
        OneByteMarker,
        NoMarker,
    }

    fn play_to_the_end(data: &[Vec<u8>], ending: Ending) -> (Vec<f32>, Talker) {
        let mut t = Talker::new().unwrap();
        let mut out = vec![0f32; BLOCK];
        let mut left = Vec::new();
        for (i, p) in data.iter().enumerate() {
            t.push(i as u16, p);
        }
        let marker = data.len() as u16;
        match ending {
            Ending::MarkerInTime => t.push(marker, &[]),
            Ending::OneByteMarker => t.push(marker, &data[0][..1]),
            Ending::MarkerLate | Ending::NoMarker => {}
        }
        let mut pulls = 0;
        loop {
            if ending == Ending::MarkerLate && pulls == data.len() {
                t.push(marker, &[]);
            }
            if !t.pull(&mut out) {
                break;
            }
            pulls += 1;
            left.extend(out.chunks(MIX_CHANNELS).map(|frame| frame[0]));
            assert!(pulls < data.len() + 20, "{ending:?}: the stream never stopped");
        }
        left.extend_from_slice(&[0.0; 8]);
        (left, t)
    }

    fn undisturbed(data: &[Vec<u8>]) -> Vec<f32> {
        let mut decoder = Decoder::new(MIX_CHANNELS).unwrap();
        let mut left = Vec::new();
        let mut frame = vec![0f32; BLOCK];
        for packet in data {
            assert_eq!(decoder.decode(packet, false, &mut frame).unwrap(), FRAME_SAMPLES);
            left.extend(frame.chunks(MIX_CHANNELS).map(|pair| pair[0]));
        }
        left
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
    fn a_stream_never_ends_with_a_step() {
        let cases = [
            (97.0, CODEC_OPUS_VOICE, 6u8),
            (113.0, CODEC_OPUS_VOICE, 6),
            (131.0, CODEC_OPUS_VOICE, 2),
            (149.0, CODEC_OPUS_VOICE, 10),
            (173.0, CODEC_OPUS_VOICE, 4),
            (211.0, CODEC_OPUS_VOICE, 8),
            (127.0, CODEC_OPUS_MUSIC, 7),
            (163.0, CODEC_OPUS_MUSIC, 10),
        ];
        let mut cut_would_click = 0;
        for (freq, codec, quality) in cases {
            let data = tone_packets(25, freq, codec, quality);
            let plain = undisturbed(&data);
            let real = plain.len();
            if plain[real - 1].abs() > 0.1 {
                cut_would_click += 1;
            }
            let usual = largest_step(&plain[FRAME_SAMPLES * 3..]);
            for ending in [Ending::MarkerInTime, Ending::MarkerLate, Ending::OneByteMarker, Ending::NoMarker] {
                let (left, talker) = play_to_the_end(&data, ending);
                let what = format!("{freq} Hz, codec {codec}, quality {quality}, {ending:?}");
                assert!(left.len() >= real, "{what}: {} of {real} samples", left.len());
                assert_eq!(talker.stats.decoded, 25, "{what}");
                let end = largest_step(&left[real - FRAME_SAMPLES / 2..]);
                assert!(end <= (usual * 1.5).max(0.012), "{what}: step {end} at the end, {usual} while playing");
                let after = &left[real..];
                let limit = if ending == Ending::NoMarker { FRAME_SAMPLES * MAX_STRETCH as usize } else { FRAME_SAMPLES };
                assert!(after.len() <= limit + 8, "{what}: {} samples after the end", after.len());
                let last = rms(&plain[real - FRAME_SAMPLES..]);
                for (n, frame) in after.chunks(FRAME_SAMPLES).enumerate() {
                    assert!(rms(frame) <= last * 1.05 + 0.001, "{what}: frame {n} after the end is louder than the last one");
                }
                if ending == Ending::NoMarker {
                    assert!(rms(&after[(FRAME_SAMPLES * 3).min(after.len())..]) < 0.002, "{what}: still audible 60 ms after the end");
                } else {
                    assert_eq!(talker.stats.concealed, 0, "{what}");
                    assert!(rms(&after[(FRAME_SAMPLES / 2).min(after.len())..]) < 0.002, "{what}: still audible 10 ms after the end");
                }
            }
        }
        assert!(cut_would_click >= 4, "only {cut_would_click} of the test signals end away from zero");
    }

    #[test]
    fn a_stream_starts_without_a_step() {
        for freq in [97.0, 131.0, 173.0] {
            let data = tone_packets(12, freq, CODEC_OPUS_VOICE, 6)[6..].to_vec();
            let (left, _) = play_to_the_end(&data, Ending::MarkerInTime);
            let body = largest_step(&left[FRAME_SAMPLES * 2..FRAME_SAMPLES * 4]);
            assert!(left[0].abs() < 0.01, "{freq} Hz: starts at {}", left[0]);
            let start = largest_step(&left[..FRAME_SAMPLES]);
            assert!(start <= (body * 1.5).max(0.012), "{freq} Hz: step {start} at the start, {body} while playing");
        }
    }

    #[test]
    fn a_one_byte_packet_ends_the_stream() {
        let data = packets(6, 300.0);
        let (left, talker) = play_to_the_end(&data, Ending::OneByteMarker);
        assert_eq!(left.len(), 6 * FRAME_SAMPLES + 8);
        assert!(!talker.is_active());
        assert_eq!((talker.stats.decoded, talker.stats.concealed, talker.stats.underruns), (6, 0, 0));
        assert_eq!(talker.target_frames(), DEFAULT_TARGET);

        let mut p = Playback::new();
        p.push(0, 9, 0, CODEC_OPUS_VOICE, &data[0][..1]);
        assert!(p.talker(0, 9).is_none());
        p.push(0, 9, 0, 2, &[7]);
        assert_eq!(p.take_unsupported_codec(), None);
    }

    #[test]
    fn late_packets_of_a_finished_stream_are_not_played() {
        let data = packets(8, 300.0);
        let mut t = Talker::new().unwrap();
        let mut out = vec![0f32; BLOCK];
        for (i, p) in data.iter().enumerate().take(6) {
            t.push(i as u16, p);
        }
        t.push(6, &[]);
        let mut frames = 0;
        while t.pull(&mut out) {
            frames += 1;
        }
        assert_eq!(frames, 6);
        t.push(4, &data[4]);
        t.push(5, &data[5]);
        assert!(!t.is_active(), "an old packet must not start a new stream");
        assert_eq!(t.stats.late, 2);
        for _ in 0..10 {
            assert!(!t.pull(&mut out));
        }
        t.push(7, &data[6]);
        assert!(t.is_active());

        let mut fresh = Talker::new().unwrap();
        fresh.push(0, &data[0]);
        assert!(fresh.is_active(), "the very first packet always starts a stream");

        let mut later = Talker::new().unwrap();
        later.push(10, &data[0]);
        later.push(11, &[]);
        while later.pull(&mut out) {}
        for _ in 0..STRAGGLER_PULLS {
            later.pull(&mut out);
        }
        later.push(9, &data[1]);
        assert!(later.is_active(), "after a pause any packet starts a stream");
    }

    #[test]
    fn a_marker_that_overtakes_the_last_packet_still_ends_the_stream() {
        let data = packets(6, 300.0);
        let mut t = Talker::new().unwrap();
        let mut out = vec![0f32; BLOCK];
        for (i, p) in data.iter().enumerate().take(5) {
            t.push(i as u16, p);
        }
        t.push(6, &[]);
        t.push(5, &data[5]);
        let mut frames = 0;
        while t.pull(&mut out) {
            frames += 1;
            assert!(frames < 20);
        }
        assert_eq!(frames, 6);
        assert_eq!((t.stats.decoded, t.stats.concealed, t.stats.underruns), (6, 0, 0));
    }

    #[test]
    fn talking_again_at_once_plays_no_filler() {
        let data = packets(12, 300.0);
        let mut t = Talker::new().unwrap();
        let mut out = vec![0f32; BLOCK];
        for (i, p) in data.iter().enumerate().take(6) {
            t.push(i as u16, p);
        }
        t.push(6, &[]);
        for (i, p) in data.iter().enumerate().skip(6) {
            t.push(i as u16 + 1, p);
        }
        t.push(13, &[]);
        let mut frames = 0;
        while t.pull(&mut out) {
            frames += 1;
            assert!(frames < 30);
        }
        assert_eq!(frames, 12);
        assert_eq!((t.stats.decoded, t.stats.concealed, t.stats.lost), (12, 0, 0));
    }

    #[test]
    fn dropping_a_frame_to_catch_up_leaves_no_step() {
        let data = tone_packets(700, 113.0, CODEC_OPUS_VOICE, 6);
        let plain = undisturbed(&data);
        let usual = largest_step(&plain[FRAME_SAMPLES * 3..]);
        let mut t = Talker::new().unwrap();
        let mut out = vec![0f32; BLOCK];
        let mut left = Vec::new();
        for (i, p) in data.iter().enumerate() {
            t.push(i as u16, p);
            if i >= 8 && t.pull(&mut out) {
                left.extend(out.chunks(MIX_CHANNELS).map(|frame| frame[0]));
            }
        }
        assert!(t.stats.skipped >= 1, "the test must make the buffer catch up");
        assert_eq!(t.stats.concealed, 0);
        let step = largest_step(&left[FRAME_SAMPLES * 3..]);
        assert!(step <= (usual * 1.5).max(0.012), "step {step} while catching up, {usual} while playing");
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
