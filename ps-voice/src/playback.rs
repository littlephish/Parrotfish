use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};

use ps_oldcodecs::speex::{self, Band};

use crate::codec::{
    is_end_marker, is_supported_codec, speex_band, Decoder, CODEC_OPUS_VOICE, FRAME_SAMPLES, SAMPLE_RATE,
};
use crate::level::Leveler;
use crate::resample::Resampler;
use crate::state::LOOPBACK_SESSION;

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
const DIM_ATTACK_SECONDS: f32 = 0.06;
const DIM_RELEASE_SECONDS: f32 = 0.3;
const DIM_HOLD_SECONDS: f32 = 0.4;
pub const DEEPEST_DIM_DB: f32 = -60.0;

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

struct SpeexLine {
    band: Band,
    decoder: speex::Decoder,
    up: Resampler,
    narrow: Vec<f32>,
    wide: Vec<f32>,
    frames: usize,
}

impl SpeexLine {
    fn new(band: Band) -> Self {
        Self {
            band,
            decoder: speex::Decoder::new(band),
            up: Resampler::new(band.sample_rate(), SAMPLE_RATE, 1),
            narrow: Vec::new(),
            wide: Vec::new(),
            frames: 1,
        }
    }

    fn reset(&mut self) {
        self.decoder.reset();
        self.up.reset();
        self.frames = 1;
    }

    fn widen(&mut self) -> usize {
        self.wide.clear();
        self.up.process(&self.narrow, &mut self.wide);
        self.wide.len()
    }

    fn decode(&mut self, data: &[u8]) -> Option<usize> {
        self.narrow.clear();
        self.frames = self.decoder.decode(data, &mut self.narrow).ok()?;
        Some(self.widen())
    }

    fn conceal(&mut self) -> usize {
        self.narrow.clear();
        for _ in 0..self.frames {
            self.decoder.conceal(&mut self.narrow);
        }
        self.widen()
    }
}

fn spread(mono: &[f32], out: &mut Vec<f32>) {
    out.clear();
    for sample in mono {
        for _ in 0..MIX_CHANNELS {
            out.push(*sample);
        }
    }
}

pub struct Talker {
    decoder: Decoder,
    speex: Option<SpeexLine>,
    speex_playing: bool,
    queue: BTreeMap<u32, (u8, Vec<u8>)>,
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
    out: Vec<f32>,
    heard: bool,
    whisper: bool,
    priority: bool,
    lowered: f32,
    pub volume: f32,
    pub stats: TalkerStats,
}

impl Talker {
    pub fn new() -> Result<Self, String> {
        Ok(Self {
            decoder: Decoder::new(MIX_CHANNELS)?,
            speex: None,
            speex_playing: false,
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
            out: Vec::with_capacity(BLOCK),
            heard: false,
            whisper: false,
            priority: false,
            lowered: 1.0,
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

    fn begin(&mut self, seq: u16, codec: u8, data: &[u8]) {
        self.decoder.reset();
        if let Some(line) = &mut self.speex {
            line.reset();
        }
        self.speex_playing = false;
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
        self.queue.insert(self.last_ext, (codec, data.to_vec()));
        self.state = StreamState::Buffering;
    }

    pub fn push(&mut self, seq: u16, data: &[u8]) {
        self.push_coded(seq, CODEC_OPUS_VOICE, data);
    }

    pub fn push_coded(&mut self, seq: u16, codec: u8, data: &[u8]) {
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
                self.begin(seq, codec, data);
            }
            return;
        }
        if diff.abs() > RESYNC_DISTANCE {
            self.begin(seq, codec, data);
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
        self.queue.entry(ext).or_insert_with(|| (codec, data.to_vec()));
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
        if self.speex_playing {
            let decoded = match self.speex.as_mut() {
                Some(line) => {
                    let decoded = line.conceal();
                    spread(&line.wide, &mut self.scratch);
                    decoded
                }
                None => 0,
            };
            self.finish_concealment(decoded);
            return;
        }
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

    fn decode_opus(&mut self, data: &[u8]) -> Option<usize> {
        self.speex_playing = false;
        let samples = Decoder::packet_samples(data).unwrap_or(FRAME_SAMPLES);
        self.celt_only = data[0] >> 3 >= 16;
        self.scratch.clear();
        self.scratch.resize(samples * MIX_CHANNELS, 0.0);
        self.decoder.decode(data, false, &mut self.scratch).ok().filter(|decoded| *decoded > 0)
    }

    fn decode_speex(&mut self, band: Band, data: &[u8]) -> Option<usize> {
        self.speex_playing = true;
        self.celt_only = false;
        if self.speex.as_ref().map(|line| line.band) != Some(band) {
            self.speex = Some(SpeexLine::new(band));
        }
        let line = self.speex.as_mut()?;
        let decoded = line.decode(data).filter(|decoded| *decoded > 0)?;
        spread(&line.wide, &mut self.scratch);
        (frame_rms(&self.scratch) <= MAX_PLAUSIBLE_RMS).then_some(decoded)
    }

    fn decode_next(&mut self) -> bool {
        while self.markers.first().is_some_and(|marker| *marker <= self.next_seq) {
            if self.markers.pop_first() == Some(self.next_seq) {
                self.next_seq += 1;
            }
        }
        if let Some((codec, data)) = self.queue.remove(&self.next_seq) {
            self.draining = false;
            let decoded = match speex_band(codec) {
                Some(band) => self.decode_speex(band, &data),
                None => self.decode_opus(&data),
            };
            match decoded {
                Some(n) => {
                    self.frame_samples = n;
                    let last = self.ended && self.queue.is_empty();
                    self.accept_good_frame(n, last);
                    self.stats.decoded += 1;
                }
                None => self.conceal(),
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

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Adjustment {
    pub leveled_db: f32,
    pub lowered_db: f32,
}

fn slew(from: f32, goal: f32, dt: f32) -> f32 {
    if goal < from {
        goal.max(from - dt / DIM_ATTACK_SECONDS)
    } else {
        goal.min(from + dt / DIM_RELEASE_SECONDS)
    }
}

#[derive(Debug, Clone, Copy)]
struct Dim {
    depth: f32,
    now: f32,
    busy: bool,
    quiet: f32,
}

impl Dim {
    fn step(&mut self, dt: f32) {
        self.quiet = if self.busy { 0.0 } else { (self.quiet + dt).min(DIM_HOLD_SECONDS) };
        let target = if self.quiet < DIM_HOLD_SECONDS { self.depth } else { 1.0 };
        self.now = slew(self.now, target, dt);
    }
}

#[derive(Default)]
pub struct Playback {
    talkers: HashMap<u32, Talker>,
    volumes: HashMap<u32, f32>,
    priority: HashSet<u32>,
    levelers: HashMap<u32, Leveler>,
    dims: HashMap<u16, Dim>,
    leveling: bool,
    unsupported_codec: Option<u8>,
}

impl Playback {
    pub fn new() -> Self {
        Self::default()
    }

    fn key(session: u16, client_id: u16) -> u32 {
        (u32::from(session) << 16) | u32::from(client_id)
    }

    fn session_of(key: u32) -> u16 {
        (key >> 16) as u16
    }

    pub fn push(&mut self, session: u16, client_id: u16, voice_id: u16, codec: u8, data: &[u8]) {
        self.push_from(session, client_id, voice_id, codec, data, false);
    }

    pub fn push_from(&mut self, session: u16, client_id: u16, voice_id: u16, codec: u8, data: &[u8], whisper: bool) {
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
                    talker.priority = self.priority.contains(&key);
                    self.talkers.insert(key, talker);
                    if session != LOOPBACK_SESSION {
                        self.levelers.entry(key).or_default();
                    }
                }
                Err(_) => return,
            }
        }
        if let Some(talker) = self.talkers.get_mut(&key) {
            if !is_end_marker(data) {
                talker.whisper = whisper;
            }
            talker.push_coded(voice_id, codec, data);
        }
    }

    pub fn set_priority(&mut self, session: u16, client_id: u16, on: bool) {
        let key = Self::key(session, client_id);
        if on {
            self.priority.insert(key);
        } else {
            self.priority.remove(&key);
        }
        if let Some(talker) = self.talkers.get_mut(&key) {
            talker.priority = on;
        }
    }

    pub fn set_priority_dim(&mut self, session: u16, db: Option<f32>) {
        let depth = db.filter(|db| db.is_finite() && *db < 0.0).map_or(1.0, |db| 10f32.powf(db.max(DEEPEST_DIM_DB) / 20.0));
        match self.dims.get_mut(&session) {
            Some(dim) => dim.depth = depth,
            None if depth < 1.0 => {
                self.dims.insert(session, Dim { depth, now: 1.0, busy: false, quiet: DIM_HOLD_SECONDS });
            }
            None => {}
        }
    }

    pub fn set_leveling(&mut self, on: bool) {
        self.leveling = on;
    }

    pub fn leveling(&self) -> bool {
        self.leveling
    }

    pub fn adjustment(&self, session: u16, client_id: u16) -> Adjustment {
        let key = Self::key(session, client_id);
        let leveled_db = if self.leveling { self.levelers.get(&key).map_or(0.0, Leveler::gain_db) } else { 0.0 };
        let lowered = match (self.talkers.get(&key), self.dims.get(&session)) {
            (Some(talker), _) => talker.lowered,
            (None, Some(dim)) if !self.priority.contains(&key) => dim.now,
            _ => 1.0,
        };
        let lowered_db = if lowered < 1.0 { 20.0 * lowered.max(1e-3).log10() } else { 0.0 };
        Adjustment { leveled_db, lowered_db }
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
        let key = Self::key(session, client_id);
        self.talkers.remove(&key);
        self.levelers.remove(&key);
    }

    pub fn clear_session(&mut self, session: u16) {
        self.talkers.retain(|key, _| Self::session_of(*key) != session);
        self.volumes.retain(|key, _| Self::session_of(*key) != session);
        self.priority.retain(|key| Self::session_of(*key) != session);
        self.levelers.retain(|key, _| Self::session_of(*key) != session);
        self.dims.remove(&session);
    }

    pub fn clear(&mut self) {
        self.talkers.clear();
        self.levelers.clear();
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
        let frames = out.len() / MIX_CHANNELS;
        let dt = frames as f32 / SAMPLE_RATE as f32;
        for dim in self.dims.values_mut() {
            dim.busy = false;
        }
        let leveling = self.leveling;
        let mut active = 0;
        for (key, talker) in self.talkers.iter_mut() {
            let mut heard = std::mem::take(&mut talker.out);
            heard.clear();
            heard.resize(out.len(), 0.0);
            talker.heard = talker.pull(&mut heard);
            if talker.heard {
                active += 1;
                let session = Self::session_of(*key);
                if let Some(leveler) = self.levelers.get_mut(key).filter(|leveler| leveling || !leveler.is_idle()) {
                    leveler.process(&mut heard, MIX_CHANNELS, leveling);
                }
                if talker.priority && !talker.whisper {
                    if let Some(dim) = self.dims.get_mut(&session) {
                        dim.busy = true;
                    }
                }
            }
            talker.out = heard;
        }
        for dim in self.dims.values_mut() {
            dim.step(dt);
        }
        for (key, talker) in self.talkers.iter_mut() {
            let goal = match self.dims.get(&Self::session_of(*key)) {
                Some(dim) if !talker.priority && !talker.whisper => dim.now,
                _ => 1.0,
            };
            if !talker.heard {
                talker.lowered = goal;
                if !talker.is_active() {
                    talker.whisper = false;
                }
                if let Some(leveler) = self.levelers.get_mut(key) {
                    leveler.rest(leveling);
                }
                continue;
            }
            let (from, to) = (talker.lowered, slew(talker.lowered, goal, dt));
            talker.lowered = to;
            let volume = talker.volume;
            if from == 1.0 && to == 1.0 {
                for (o, s) in out.iter_mut().zip(talker.out.iter()) {
                    *o += *s * volume;
                }
            } else {
                let pairs = out.chunks_mut(MIX_CHANNELS).zip(talker.out.chunks(MIX_CHANNELS));
                for (index, (mixed, heard)) in pairs.enumerate() {
                    let gain = volume * (from + (to - from) * (index as f32 + 1.0) / frames.max(1) as f32);
                    for (o, s) in mixed.iter_mut().zip(heard) {
                        *o += *s * gain;
                    }
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
    use crate::codec::{
        Encoder, CODEC_OPUS_MUSIC, CODEC_OPUS_VOICE, CODEC_SPEEX_NARROW, CODEC_SPEEX_ULTRA_WIDE, CODEC_SPEEX_WIDE,
        MAX_PACKET_BYTES, SAMPLE_RATE,
    };

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

    fn speex_stream(packets: &[u8], reference: &[u8]) -> (Vec<Vec<u8>>, Vec<f32>) {
        let mut out = Vec::new();
        let mut at = 0;
        while at + 2 <= packets.len() {
            let length = usize::from(packets[at]) | usize::from(packets[at + 1]) << 8;
            at += 2;
            out.push(packets[at..at + length].to_vec());
            at += length;
        }
        let sound = reference.chunks_exact(2).map(|pair| f32::from(i16::from_le_bytes([pair[0], pair[1]])) / 32768.0).collect();
        (out, sound)
    }

    fn wide_speex() -> (Vec<Vec<u8>>, Vec<f32>) {
        speex_stream(
            include_bytes!("../../ps-oldcodecs/tests/data/speex/wb_q6.pkt"),
            include_bytes!("../../ps-oldcodecs/tests/data/speex/wb_q6.s16"),
        )
    }

    fn play_speex(codec: u8, data: &[Vec<u8>], skip: Option<usize>) -> (Vec<f32>, Talker) {
        let mut t = Talker::new().unwrap();
        for (i, p) in data.iter().enumerate() {
            if Some(i) != skip {
                t.push_coded(i as u16, codec, p);
            }
        }
        t.push_coded(data.len() as u16, codec, &[]);
        let mut all = Vec::new();
        let mut block = vec![0f32; BLOCK];
        while t.pull(&mut block) {
            all.extend_from_slice(&block);
            assert!(all.len() < BLOCK * 400, "the stream never ends");
        }
        (all, t)
    }

    fn loudness_by_block(samples: &[f32], block: usize) -> Vec<f32> {
        samples.chunks_exact(block).map(rms).collect()
    }

    fn alike(a: &[f32], b: &[f32]) -> f32 {
        let n = a.len().min(b.len());
        let (a, b) = (&a[..n], &b[..n]);
        let mean = |x: &[f32]| x.iter().sum::<f32>() / n as f32;
        let (ma, mb) = (mean(a), mean(b));
        let top: f32 = a.iter().zip(b).map(|(x, y)| (x - ma) * (y - mb)).sum();
        let spread = |x: &[f32], m: f32| x.iter().map(|v| (v - m) * (v - m)).sum::<f32>().sqrt();
        top / (spread(a, ma) * spread(b, mb)).max(1e-9)
    }

    #[test]
    fn speex_is_played_at_the_mixers_rate() {
        let (data, reference) = wide_speex();
        let (played, t) = play_speex(CODEC_SPEEX_WIDE, &data, None);
        assert_eq!(t.stats.decoded, data.len() as u64);
        assert_eq!(t.stats.concealed, 0);
        let frames = played.len() / MIX_CHANNELS;
        let wanted = data.len() * FRAME_SAMPLES;
        assert!(frames + FRAME_SAMPLES >= wanted && frames <= wanted + 2 * FRAME_SAMPLES, "{frames} frames for {wanted}");
        assert!(played.chunks_exact(MIX_CHANNELS).all(|pair| pair[0] == pair[1]), "one voice, the same on both sides");
        assert!(played.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
        let left: Vec<f32> = played.iter().step_by(MIX_CHANNELS).copied().collect();
        let level = rms(&left[FRAME_SAMPLES..left.len() - 2 * FRAME_SAMPLES]);
        let source = rms(&reference[320..reference.len() - 640]);
        assert!((level / source - 1.0).abs() < 0.12, "level {level} against {source}");
        let ours = loudness_by_block(&left, FRAME_SAMPLES);
        let theirs = loudness_by_block(&reference, 320);
        let shape = alike(&ours[1..ours.len() - 2], &theirs[1..theirs.len() - 2]);
        assert!(shape > 0.97, "the loudness over time only matches the reference by {shape}");
    }

    #[test]
    fn narrow_and_ultra_wide_speex_reach_the_mixer_too() {
        for (codec, block, (data, reference)) in [
            (
                CODEC_SPEEX_NARROW,
                160,
                speex_stream(
                    include_bytes!("../../ps-oldcodecs/tests/data/speex/nb_q6.pkt"),
                    include_bytes!("../../ps-oldcodecs/tests/data/speex/nb_q6.s16"),
                ),
            ),
            (
                CODEC_SPEEX_ULTRA_WIDE,
                640,
                speex_stream(
                    include_bytes!("../../ps-oldcodecs/tests/data/speex/uwb_q5.pkt"),
                    include_bytes!("../../ps-oldcodecs/tests/data/speex/uwb_q5.s16"),
                ),
            ),
        ] {
            let (played, t) = play_speex(codec, &data, None);
            assert_eq!(t.stats.decoded, data.len() as u64, "codec {codec}");
            let left: Vec<f32> = played.iter().step_by(MIX_CHANNELS).copied().collect();
            let wanted = data.len() * FRAME_SAMPLES;
            assert!(left.len() + FRAME_SAMPLES >= wanted && left.len() <= wanted + 2 * FRAME_SAMPLES, "codec {codec}");
            let ours = loudness_by_block(&left, FRAME_SAMPLES);
            let theirs = loudness_by_block(&reference, block);
            let shape = alike(&ours[1..ours.len() - 2], &theirs[1..theirs.len() - 2]);
            assert!(shape > 0.95, "codec {codec}: the loudness over time only matches the reference by {shape}");
        }
    }

    #[test]
    fn several_speex_frames_in_one_packet_play_as_that_much_sound() {
        let (data, _) = speex_stream(
            include_bytes!("../../ps-oldcodecs/tests/data/speex/nb_q6_x3.pkt"),
            include_bytes!("../../ps-oldcodecs/tests/data/speex/nb_q6_x3.s16"),
        );
        let (played, t) = play_speex(CODEC_SPEEX_NARROW, &data, None);
        assert_eq!(t.stats.decoded, data.len() as u64);
        let frames = played.len() / MIX_CHANNELS;
        let wanted = data.len() * 3 * FRAME_SAMPLES;
        assert!(frames + FRAME_SAMPLES >= wanted && frames <= wanted + 2 * FRAME_SAMPLES, "{frames} frames for {wanted}");
    }

    #[test]
    fn a_lost_speex_packet_is_filled_in_without_a_step() {
        let (data, _) = wide_speex();
        let (whole, _) = play_speex(CODEC_SPEEX_WIDE, &data, None);
        let (played, t) = play_speex(CODEC_SPEEX_WIDE, &data, Some(12));
        assert_eq!(t.stats.lost, 1);
        assert_eq!(t.stats.concealed, 1);
        assert_eq!(t.stats.decoded, data.len() as u64 - 1);
        assert_eq!(played.len(), whole.len(), "the gap is filled, not closed up");
        assert!(largest_step(&played) <= largest_step(&whole) * 1.5 + 0.02, "{} against {}", largest_step(&played), largest_step(&whole));
        assert!(played.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
    }

    #[test]
    fn a_talker_can_change_between_opus_and_speex() {
        let opus = packets(30, 440.0);
        let (speex, _) = wide_speex();
        let mut t = Talker::new().unwrap();
        for i in 0..10 {
            t.push_coded(i as u16, CODEC_OPUS_VOICE, &opus[i]);
        }
        for i in 10..20 {
            t.push_coded(i as u16, CODEC_SPEEX_WIDE, &speex[i]);
        }
        for i in 20..30 {
            t.push_coded(i as u16, CODEC_OPUS_VOICE, &opus[i]);
        }
        t.push_coded(30, CODEC_OPUS_VOICE, &[]);
        let mut block = vec![0f32; BLOCK];
        let mut all = Vec::new();
        while t.pull(&mut block) {
            all.extend_from_slice(&block);
            assert!(all.len() < BLOCK * 200);
        }
        assert_eq!(t.stats.decoded, 30);
        assert!(all.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
        assert!(rms(&all) > 0.01);
    }

    #[test]
    fn rubbish_marked_as_speex_is_survived() {
        let mut seed = 0x9E37_79B9u32;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed
        };
        for codec in [CODEC_SPEEX_NARROW, CODEC_SPEEX_WIDE, CODEC_SPEEX_ULTRA_WIDE] {
            let mut p = Playback::new();
            let mut out = vec![0f32; BLOCK];
            for i in 0..400u16 {
                let length = 2 + (next() % 120) as usize;
                let packet: Vec<u8> = (0..length).map(|_| (next() >> 13) as u8).collect();
                p.push(0, 9, i, codec, &packet);
                p.mix(&mut out);
                assert!(out.iter().all(|s| s.is_finite() && s.abs() <= 1.0), "codec {codec}, packet {i}");
            }
            assert_eq!(p.take_unsupported_codec(), None);
        }
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
        p.push(0, 3, 0, 3, &[1, 2, 3]);
        assert_eq!(p.take_unsupported_codec(), Some(3));
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

    fn tone_at(count: usize, freq: f32, amplitude: f32) -> Vec<Vec<u8>> {
        let mut enc = Encoder::new(CODEC_OPUS_VOICE, 6).unwrap();
        let channels = enc.channels();
        let mut buf = [0u8; MAX_PACKET_BYTES];
        (0..count)
            .map(|f| {
                let mut pcm = Vec::with_capacity(FRAME_SAMPLES * channels);
                for i in 0..FRAME_SAMPLES {
                    let t = (f * FRAME_SAMPLES + i) as f32 / SAMPLE_RATE as f32;
                    let sample = amplitude * (2.0 * std::f32::consts::PI * freq * t).sin();
                    for _ in 0..channels {
                        pcm.push(sample);
                    }
                }
                let n = enc.encode(&pcm, &mut buf).unwrap();
                buf[..n].to_vec()
            })
            .collect()
    }

    fn strength(samples: &[f32], freq: f32) -> f32 {
        let (mut sin, mut cos) = (0.0f64, 0.0f64);
        for (n, sample) in samples.iter().enumerate() {
            let angle = 2.0 * std::f64::consts::PI * f64::from(freq) * n as f64 / f64::from(SAMPLE_RATE);
            sin += f64::from(*sample) * angle.sin();
            cos += f64::from(*sample) * angle.cos();
        }
        (2.0 * (sin * sin + cos * cos).sqrt() / samples.len().max(1) as f64) as f32
    }

    fn db(ratio: f32) -> f32 {
        20.0 * ratio.max(1e-9).log10()
    }

    struct Part<'a> {
        session: u16,
        client: u16,
        packets: &'a [Vec<u8>],
        from: usize,
        whisper: bool,
    }

    fn says<'a>(session: u16, client: u16, packets: &'a [Vec<u8>], from: usize) -> Part<'a> {
        Part { session, client, packets, from, whisper: false }
    }

    fn play(p: &mut Playback, parts: &[Part], blocks: usize) -> Vec<f32> {
        let mut out = vec![0f32; BLOCK];
        let mut left = Vec::with_capacity(blocks * FRAME_SAMPLES);
        for block in 0..blocks {
            for part in parts {
                let end = part.from + part.packets.len();
                if block >= part.from && block < end {
                    let at = block - part.from;
                    p.push_from(part.session, part.client, at as u16, CODEC_OPUS_VOICE, &part.packets[at], part.whisper);
                } else if block == end {
                    p.push_from(part.session, part.client, part.packets.len() as u16, CODEC_OPUS_VOICE, &[], part.whisper);
                }
            }
            p.mix(&mut out);
            left.extend(out.chunks(MIX_CHANNELS).map(|frame| frame[0]));
        }
        left
    }

    fn span(left: &[f32], from_block: usize, to_block: usize) -> &[f32] {
        &left[from_block * FRAME_SAMPLES..to_block * FRAME_SAMPLES]
    }

    #[test]
    fn everyone_else_is_lowered_while_a_priority_speaker_talks() {
        let normal = tone_at(240, 440.0, 0.2);
        let chief = tone_at(60, 1000.0, 0.2);
        let mut p = Playback::new();
        p.set_priority(1, 10, true);
        p.set_priority_dim(1, Some(-18.0));
        let left = play(&mut p, &[says(1, 20, &normal, 0), says(1, 10, &chief, 70)], 255);
        let alone = strength(span(&left, 20, 60), 440.0);
        let under = strength(span(&left, 85, 125), 440.0);
        let after = strength(span(&left, 200, 235), 440.0);
        assert!(alone > 0.1, "the test tone itself came through at {alone}");
        assert!((db(under / alone) + 18.0).abs() < 1.0, "lowered by {:.1} dB", db(under / alone));
        assert!(db(after / alone).abs() < 0.5, "afterwards it was {:.1} dB off", db(after / alone));
        let chief_level = strength(span(&left, 85, 125), 1000.0);
        assert!(db(chief_level / alone).abs() < 2.0, "the priority speaker came through {:.1} dB off", db(chief_level / alone));
        assert_eq!(p.adjustment(1, 20).lowered_db, 0.0);
        assert_eq!(p.adjustment(1, 10), Adjustment::default());
    }

    #[test]
    fn what_is_shown_about_a_lowered_person_matches_what_is_played() {
        let normal = tone_at(120, 440.0, 0.2);
        let chief = tone_at(80, 1000.0, 0.2);
        let mut p = Playback::new();
        p.set_priority(1, 10, true);
        p.set_priority_dim(1, Some(-18.0));
        let left = play(&mut p, &[says(1, 20, &normal, 0), says(1, 10, &chief, 30)], 100);
        let played = db(strength(span(&left, 60, 100), 440.0) / strength(span(&left, 8, 28), 440.0));
        let shown = p.adjustment(1, 20).lowered_db;
        assert!((shown - played).abs() < 1.0, "shown {shown:.1} dB, played {played:.1} dB");
        assert!((p.adjustment(1, 20).lowered_db + 18.0).abs() < 0.1, "{:?}", p.adjustment(1, 20));
        assert!((p.adjustment(1, 21).lowered_db + 18.0).abs() < 0.1, "someone who has not spoken yet would be lowered as well");
        assert_eq!(p.adjustment(1, 10).lowered_db, 0.0, "the priority speaker is not lowered");
        assert_eq!(p.adjustment(2, 20).lowered_db, 0.0, "another connection is not touched");
    }

    #[test]
    fn the_lowering_comes_and_goes_without_a_step() {
        let normal = tone_at(130, 440.0, 0.3);
        let hush = tone_at(40, 440.0, 0.0);
        let plain = play(&mut Playback::new(), &[says(1, 20, &normal, 0)], 135);
        let mut p = Playback::new();
        p.set_priority(1, 10, true);
        p.set_priority_dim(1, Some(-18.0));
        let left = play(&mut p, &[says(1, 20, &normal, 0), says(1, 10, &hush, 40)], 135);
        assert!(
            largest_step(&left) <= largest_step(&plain) * 1.02 + 1e-4,
            "largest step {} against {} without lowering",
            largest_step(&left),
            largest_step(&plain)
        );
        let full = rms(span(&plain, 20, 35));
        let levels: Vec<f32> = (20..130).map(|block| rms(span(&left, block, block + 1)) / full).collect();
        let lowest = levels.iter().copied().fold(f32::MAX, f32::min);
        assert!((db(lowest) + 18.0).abs() < 1.0, "the lowest point was {:.1} dB", db(lowest));
        let down_from = levels.iter().position(|level| *level < 0.9).unwrap();
        let down_to = levels.iter().position(|level| *level < 0.2).unwrap();
        assert!(down_to - down_from <= 4, "going down took {} blocks", down_to - down_from);
        let up_from = down_to + levels[down_to..].iter().position(|level| *level > 0.2).unwrap();
        let up_to = down_to + levels[down_to..].iter().position(|level| *level > 0.9).unwrap();
        assert!((6..=20).contains(&(up_to - up_from)), "coming back took {} blocks", up_to - up_from);
    }

    #[test]
    fn priority_speakers_are_not_lowered_by_each_other() {
        let first = tone_at(80, 440.0, 0.2);
        let second = tone_at(80, 1000.0, 0.2);
        let lone_first = strength(span(&play(&mut Playback::new(), &[says(1, 10, &first, 0)], 80), 20, 70), 440.0);
        let lone_second = strength(span(&play(&mut Playback::new(), &[says(1, 11, &second, 0)], 80), 20, 70), 1000.0);
        let mut p = Playback::new();
        p.set_priority(1, 10, true);
        p.set_priority(1, 11, true);
        p.set_priority_dim(1, Some(-18.0));
        let left = play(&mut p, &[says(1, 10, &first, 0), says(1, 11, &second, 0)], 80);
        assert!(db(strength(span(&left, 20, 70), 440.0) / lone_first).abs() < 0.5);
        assert!(db(strength(span(&left, 20, 70), 1000.0) / lone_second).abs() < 0.5);
    }

    #[test]
    fn whispers_lower_nobody_and_are_not_lowered() {
        let normal = tone_at(90, 440.0, 0.2);
        let chief = tone_at(90, 1000.0, 0.2);
        let third = tone_at(90, 1500.0, 0.2);
        let lone = strength(span(&play(&mut Playback::new(), &[says(1, 20, &normal, 0)], 90), 30, 80), 440.0);
        let lone_third = strength(span(&play(&mut Playback::new(), &[says(1, 30, &third, 0)], 90), 30, 80), 1500.0);

        let mut p = Playback::new();
        p.set_priority(1, 10, true);
        p.set_priority_dim(1, Some(-18.0));
        let whispering_chief = Part { session: 1, client: 10, packets: &chief, from: 0, whisper: true };
        let left = play(&mut p, &[says(1, 20, &normal, 0), whispering_chief], 90);
        assert!(db(strength(span(&left, 30, 80), 440.0) / lone).abs() < 0.5, "a whispering priority speaker lowered the channel");

        let mut p = Playback::new();
        p.set_priority(1, 10, true);
        p.set_priority_dim(1, Some(-18.0));
        let whisperer = Part { session: 1, client: 30, packets: &third, from: 0, whisper: true };
        let left = play(&mut p, &[says(1, 20, &normal, 0), says(1, 10, &chief, 0), whisperer], 90);
        assert!((db(strength(span(&left, 30, 80), 440.0) / lone) + 18.0).abs() < 1.0);
        assert!(db(strength(span(&left, 30, 80), 1500.0) / lone_third).abs() < 0.5, "a whisper to me was lowered");
        assert_eq!(p.adjustment(1, 30).lowered_db, 0.0);
    }

    #[test]
    fn a_persons_own_volume_still_counts_while_they_are_lowered() {
        let normal = tone_at(90, 440.0, 0.2);
        let chief = tone_at(90, 1000.0, 0.2);
        let lone = strength(span(&play(&mut Playback::new(), &[says(1, 20, &normal, 0)], 90), 30, 80), 440.0);
        let mut p = Playback::new();
        p.set_priority(1, 10, true);
        p.set_priority_dim(1, Some(-18.0));
        p.set_volume(1, 20, 0.5);
        let left = play(&mut p, &[says(1, 20, &normal, 0), says(1, 10, &chief, 0)], 90);
        let heard = db(strength(span(&left, 30, 80), 440.0) / lone);
        assert!((heard + 24.0).abs() < 1.0, "at half volume and lowered by 18 dB it came out {heard:.1} dB down");
    }

    #[test]
    fn someone_who_whispered_earlier_counts_as_anyone_else_once_they_stopped() {
        let whisper = tone_at(30, 1500.0, 0.2);
        let chief = tone_at(60, 1000.0, 0.2);
        let mut p = Playback::new();
        p.set_priority(1, 10, true);
        p.set_priority_dim(1, Some(-18.0));
        let whisperer = Part { session: 1, client: 30, packets: &whisper, from: 0, whisper: true };
        play(&mut p, &[whisperer, says(1, 10, &chief, 50)], 90);
        assert!(p.talker(1, 30).is_some(), "the person who whispered is still known to the mixer");
        let shown = p.adjustment(1, 30).lowered_db;
        assert!((shown + 18.0).abs() < 0.1, "shown as lowered by {shown:.1} dB while the priority speaker talks");
    }

    #[test]
    fn lowering_stays_inside_one_connection() {
        let normal = tone_at(90, 440.0, 0.2);
        let chief = tone_at(90, 1000.0, 0.2);
        let lone = strength(span(&play(&mut Playback::new(), &[says(2, 20, &normal, 0)], 90), 30, 80), 440.0);
        let mut p = Playback::new();
        p.set_priority(1, 10, true);
        p.set_priority_dim(1, Some(-18.0));
        p.set_priority_dim(2, Some(-18.0));
        let left = play(&mut p, &[says(2, 20, &normal, 0), says(1, 10, &chief, 0)], 90);
        assert!(db(strength(span(&left, 30, 80), 440.0) / lone).abs() < 0.5);
    }

    #[test]
    fn nobody_is_lowered_without_a_value_or_with_none() {
        let normal = tone_at(90, 440.0, 0.2);
        let chief = tone_at(90, 1000.0, 0.2);
        let lone = strength(span(&play(&mut Playback::new(), &[says(1, 20, &normal, 0)], 90), 30, 80), 440.0);
        for value in [None, Some(0.0), Some(3.0), Some(f32::NAN)] {
            let mut p = Playback::new();
            p.set_priority(1, 10, true);
            p.set_priority_dim(1, value);
            let left = play(&mut p, &[says(1, 20, &normal, 0), says(1, 10, &chief, 0)], 90);
            assert!(db(strength(span(&left, 30, 80), 440.0) / lone).abs() < 0.5, "lowered with {value:?}");
        }
        let mut p = Playback::new();
        p.set_priority(1, 10, true);
        p.set_priority_dim(1, Some(-18.0));
        p.set_priority_dim(1, None);
        let left = play(&mut p, &[says(1, 20, &normal, 0), says(1, 10, &chief, 0)], 90);
        assert!(db(strength(span(&left, 30, 80), 440.0) / lone).abs() < 0.5, "lowered after the value was taken away");
        let mut p = Playback::new();
        p.set_priority(1, 10, true);
        p.set_priority_dim(1, Some(-500.0));
        let left = play(&mut p, &[says(1, 20, &normal, 0), says(1, 10, &chief, 0)], 90);
        let lowered = db(strength(span(&left, 40, 80), 440.0) / lone);
        assert!((lowered - DEEPEST_DIM_DB).abs() < 3.0, "an absurd value lowered by {lowered:.1} dB");
    }

    #[test]
    fn a_person_who_stops_being_a_priority_speaker_stops_lowering_others() {
        let normal = tone_at(160, 440.0, 0.2);
        let chief = tone_at(160, 1000.0, 0.2);
        let mut p = Playback::new();
        p.set_priority(1, 10, true);
        p.set_priority_dim(1, Some(-18.0));
        let parts = [says(1, 20, &normal, 0), says(1, 10, &chief, 0)];
        let mut out = vec![0f32; BLOCK];
        let mut left = Vec::new();
        for block in 0..160 {
            if block == 80 {
                p.set_priority(1, 10, false);
            }
            for part in &parts {
                p.push(part.session, part.client, block as u16, CODEC_OPUS_VOICE, &part.packets[block]);
            }
            p.mix(&mut out);
            left.extend(out.chunks(MIX_CHANNELS).map(|frame| frame[0]));
        }
        let lowered = strength(span(&left, 30, 70), 440.0);
        let restored = strength(span(&left, 125, 158), 440.0);
        assert!((db(lowered / restored) + 18.0).abs() < 1.0, "{:.1} dB", db(lowered / restored));
        assert!(largest_step(&left) < 0.05, "a step of {} when the flag was taken away", largest_step(&left));
    }

    #[test]
    fn becoming_a_priority_speaker_in_mid_sentence_brings_no_step() {
        let first = tone_at(160, 440.0, 0.2);
        let second = tone_at(160, 1000.0, 0.2);
        let mut p = Playback::new();
        p.set_priority(1, 10, true);
        p.set_priority_dim(1, Some(-18.0));
        let mut out = vec![0f32; BLOCK];
        let mut left = Vec::new();
        for block in 0..160 {
            if block == 80 {
                p.set_priority(1, 20, true);
            }
            p.push(1, 10, block as u16, CODEC_OPUS_VOICE, &second[block]);
            p.push(1, 20, block as u16, CODEC_OPUS_VOICE, &first[block]);
            p.mix(&mut out);
            left.extend(out.chunks(MIX_CHANNELS).map(|frame| frame[0]));
        }
        let before = strength(span(&left, 30, 70), 440.0);
        let after = strength(span(&left, 110, 150), 440.0);
        assert!((db(before / after) + 18.0).abs() < 1.0, "{:.1} dB", db(before / after));
        assert!(largest_step(&left) < 0.05, "a step of {} when the flag was given", largest_step(&left));
    }

    #[test]
    fn a_whisper_that_turns_into_talk_is_lowered_without_a_step() {
        let normal = tone_at(160, 440.0, 0.2);
        let chief = tone_at(160, 1000.0, 0.2);
        let mut p = Playback::new();
        p.set_priority(1, 10, true);
        p.set_priority_dim(1, Some(-18.0));
        let mut out = vec![0f32; BLOCK];
        let mut left = Vec::new();
        for block in 0..160 {
            p.push(1, 10, block as u16, CODEC_OPUS_VOICE, &chief[block]);
            p.push_from(1, 20, block as u16, CODEC_OPUS_VOICE, &normal[block], block < 80);
            p.mix(&mut out);
            left.extend(out.chunks(MIX_CHANNELS).map(|frame| frame[0]));
        }
        let whispered = strength(span(&left, 30, 70), 440.0);
        let talked = strength(span(&left, 110, 150), 440.0);
        assert!((db(talked / whispered) + 18.0).abs() < 1.0, "{:.1} dB", db(talked / whispered));
        assert!(largest_step(&left) < 0.05, "a step of {} when the whisper became talk", largest_step(&left));
    }

    #[test]
    fn a_priority_speakers_short_pauses_do_not_let_the_others_swell() {
        let normal = tone_at(150, 440.0, 0.3);
        let hush = tone_at(30, 440.0, 0.0);
        let plain = play(&mut Playback::new(), &[says(1, 20, &normal, 0)], 150);
        let full = rms(span(&plain, 20, 35));
        let mut p = Playback::new();
        p.set_priority(1, 10, true);
        p.set_priority_dim(1, Some(-18.0));
        let mut out = vec![0f32; BLOCK];
        let mut left = Vec::new();
        let mut sent = 0u16;
        for block in 0..150 {
            p.push(1, 20, block as u16, CODEC_OPUS_VOICE, &normal[block]);
            if (20..50).contains(&block) || (62..92).contains(&block) {
                p.push(1, 10, sent, CODEC_OPUS_VOICE, &hush[block % hush.len()]);
                sent += 1;
            } else if block == 50 || block == 92 {
                p.push(1, 10, sent, CODEC_OPUS_VOICE, &[]);
                sent += 1;
            }
            p.mix(&mut out);
            left.extend(out.chunks(MIX_CHANNELS).map(|frame| frame[0]));
        }
        for block in 30..92 {
            let level = rms(span(&left, block, block + 1)) / full;
            assert!(level < 0.2, "block {block}, between or during the two sentences, came out at {:.1} dB", db(level));
        }
        let later = rms(span(&left, 135, 148)) / full;
        assert!(db(later).abs() < 0.5, "afterwards it stood at {:.1} dB", db(later));
        let mut held = Playback::new();
        held.set_priority(1, 10, true);
        held.set_priority_dim(1, Some(-18.0));
        let long = play(&mut held, &[says(1, 20, &normal, 0), says(1, 10, &hush, 20)], 150);
        let stopped = 20 + hush.len();
        let back = (stopped..150).find(|block| rms(span(&long, *block, block + 1)) / full > 0.9).unwrap();
        let waited = (back - stopped) as f32 * 0.02;
        assert!((0.5..=1.0).contains(&waited), "everyone was back {waited:.2} s after the priority speaker stopped");
    }

    #[test]
    fn levelling_and_lowering_add_up() {
        let loud = tone_at(260, 440.0, 0.8);
        let chief = tone_at(60, 1000.0, 0.1);
        let mut p = Playback::new();
        p.set_leveling(true);
        p.set_priority(0, 10, true);
        p.set_priority_dim(0, Some(-18.0));
        let left = play(&mut p, &[says(0, 1, &loud, 0), says(0, 10, &chief, 150)], 262);
        let levelled = strength(span(&left, 105, 145), 440.0);
        let both = strength(span(&left, 165, 205), 440.0);
        let target = 10f32.powf(crate::level::TARGET_DB / 20.0) * std::f32::consts::SQRT_2;
        assert!(db(levelled / target).abs() < 2.0, "levelled it sat {:.1} dB from the target", db(levelled / target));
        assert!((db(both / levelled) + 18.0).abs() < 1.0, "lowered by {:.1} dB on top", db(both / levelled));
        assert!(largest_step(&left) < 0.05, "a step of {}", largest_step(&left));
    }

    #[test]
    fn the_first_words_after_a_shout_are_not_held_down() {
        let shout = tone_at(12, 440.0, 0.9);
        let usual = tone_at(60, 440.0, 0.14);
        let mut p = Playback::new();
        p.set_leveling(true);
        play(&mut p, &[says(0, 1, &shout, 0)], 70);
        let again = play(&mut p, &[says(0, 1, &usual, 0)], 62);
        let plain = play(&mut Playback::new(), &[says(0, 1, &usual, 0)], 62);
        let opening = |left: &[f32]| {
            let first = (0..40).find(|block| rms(span(left, *block, block + 1)) > 0.02).unwrap();
            rms(span(left, first + 1, first + 4))
        };
        let down = db(opening(&again) / opening(&plain));
        assert!(down > -5.5, "the first words came out {down:.1} dB down");
        assert!(down < -1.0, "the shout was not forgotten altogether either: {down:.1} dB");
    }

    #[test]
    fn blocks_of_half_the_length_give_the_same_lowering() {
        let normal = tone_at(160, 440.0, 0.3);
        let hush = tone_at(40, 440.0, 0.0);
        let mut p = Playback::new();
        p.set_priority(1, 10, true);
        p.set_priority_dim(1, Some(-18.0));
        let mut out = vec![0f32; BLOCK / 2];
        let mut left = Vec::new();
        for half in 0..320 {
            let block = half / 2;
            if half % 2 == 0 {
                p.push(1, 20, block as u16, CODEC_OPUS_VOICE, &normal[block]);
                if (40..80).contains(&block) {
                    p.push(1, 10, (block - 40) as u16, CODEC_OPUS_VOICE, &hush[block - 40]);
                } else if block == 80 {
                    p.push(1, 10, 40, CODEC_OPUS_VOICE, &[]);
                }
            }
            p.mix(&mut out);
            left.extend(out.chunks(MIX_CHANNELS).map(|frame| frame[0]));
        }
        let full = rms(span(&left, 20, 35));
        let levels: Vec<f32> = (20..155).map(|block| rms(span(&left, block, block + 1)) / full).collect();
        let lowest = levels.iter().copied().fold(f32::MAX, f32::min);
        assert!((db(lowest) + 18.0).abs() < 1.0, "the lowest point was {:.1} dB", db(lowest));
        let down_from = levels.iter().position(|level| *level < 0.9).unwrap();
        let down_to = levels.iter().position(|level| *level < 0.2).unwrap();
        assert!(down_to - down_from <= 4, "going down took {} blocks", down_to - down_from);
        let up_from = down_to + levels[down_to..].iter().position(|level| *level > 0.2).unwrap();
        let up_to = down_to + levels[down_to..].iter().position(|level| *level > 0.9).unwrap();
        assert!((6..=20).contains(&(up_to - up_from)), "coming back took {} blocks", up_to - up_from);
    }

    #[test]
    fn a_loud_person_is_turned_down_in_the_mix_when_levelling_is_on() {
        let loud = tone_at(160, 440.0, 0.8);
        let raw = play(&mut Playback::new(), &[says(0, 1, &loud, 0)], 160);
        let mut p = Playback::new();
        assert!(!p.leveling());
        p.set_leveling(true);
        let even = play(&mut p, &[says(0, 1, &loud, 0)], 160);
        let before = strength(span(&raw, 110, 150), 440.0);
        let after = strength(span(&even, 110, 150), 440.0);
        assert!(before > 0.6, "unlevelled it came through at {before}");
        let target = 10f32.powf(crate::level::TARGET_DB / 20.0) * std::f32::consts::SQRT_2;
        assert!(db(after / target).abs() < 2.0, "levelled it sat {:.1} dB from the target", db(after / target));
        let shown = p.adjustment(0, 1).leveled_db;
        assert!((shown - db(after / before)).abs() < 1.5, "shown {shown:.1} dB, measured {:.1} dB", db(after / before));
        let cap = 10f32.powf((crate::level::CAP_DB + 1.5) / 20.0);
        for block in 0..160 {
            let level = rms(span(&even, block, block + 1));
            assert!(level <= cap, "block {block} came out at {:.1} dB", db(level));
        }
        p.set_volume(0, 1, 0.5);
        let more = tone_at(60, 440.0, 0.8);
        let quieter = play(&mut p, &[says(0, 1, &more, 0)], 60);
        let halved = strength(span(&quieter, 15, 55), 440.0);
        assert!((db(halved / after) + 6.0).abs() < 1.0, "with the person's own volume at half: {:.1} dB", db(halved / after));
    }

    #[test]
    fn people_are_levelled_one_by_one() {
        let loud = tone_at(160, 440.0, 0.8);
        let usual = tone_at(160, 1000.0, 0.1);
        let lone_usual = strength(span(&play(&mut Playback::new(), &[says(0, 2, &usual, 0)], 160), 110, 150), 1000.0);
        let mut p = Playback::new();
        p.set_leveling(true);
        let left = play(&mut p, &[says(0, 1, &loud, 0), says(0, 2, &usual, 0)], 160);
        let (first, second) = (strength(span(&left, 110, 150), 440.0), strength(span(&left, 110, 150), 1000.0));
        assert!(db(second / lone_usual).abs() < 1.0, "the person at a usual level moved by {:.1} dB", db(second / lone_usual));
        assert!(db(first / second) < 5.0, "they were 18 dB apart and came out {:.1} dB apart", db(first / second));
        assert!(p.adjustment(0, 1).leveled_db < -10.0 && p.adjustment(0, 2).leveled_db.abs() < 2.5);
    }

    #[test]
    fn what_was_learned_about_a_person_is_kept_while_they_are_quiet() {
        let loud = tone_at(100, 440.0, 0.8);
        let mut p = Playback::new();
        p.set_leveling(true);
        play(&mut p, &[says(0, 1, &loud, 0)], 110);
        let learned = p.adjustment(0, 1).leveled_db;
        assert!(learned < -10.0, "{learned}");
        let mut out = vec![0f32; BLOCK];
        for _ in 0..IDLE_PULLS_BEFORE_DROP + 5 {
            p.mix(&mut out);
        }
        assert!(p.talker(0, 1).is_none(), "the stream itself was dropped after a minute of quiet");
        assert_eq!(p.adjustment(0, 1).leveled_db, learned);
        let again = play(&mut p, &[says(0, 1, &loud, 0)], 30);
        let limit = 10f32.powf((crate::level::TARGET_DB + 4.0) / 20.0);
        for block in 0..30 {
            let level = rms(span(&again, block, block + 1));
            assert!(level <= limit, "block {block} of the second time came out at {:.1} dB", db(level));
        }
    }

    #[test]
    fn switching_levelling_off_gives_the_plain_sound_back() {
        let loud = tone_at(200, 440.0, 0.6);
        let raw = play(&mut Playback::new(), &[says(0, 1, &loud, 0)], 200);
        let mut p = Playback::new();
        p.set_leveling(true);
        let mut out = vec![0f32; BLOCK];
        let mut left = Vec::new();
        for block in 0..200 {
            if block == 100 {
                p.set_leveling(false);
            }
            p.push(0, 1, block as u16, CODEC_OPUS_VOICE, &loud[block]);
            p.mix(&mut out);
            left.extend(out.chunks(MIX_CHANNELS).map(|frame| frame[0]));
        }
        let plain = strength(span(&raw, 140, 190), 440.0);
        let back = strength(span(&left, 140, 190), 440.0);
        let levelled = strength(span(&left, 60, 95), 440.0);
        assert!(db(back / plain).abs() < 0.3, "{:.2} dB from the plain sound", db(back / plain));
        assert!(db(levelled / plain) < -6.0);
        assert_eq!(p.adjustment(0, 1).leveled_db, 0.0);
        assert!(largest_step(&left) <= largest_step(&raw) * 1.05 + 1e-4);
    }

    #[test]
    fn the_microphone_test_is_not_levelled() {
        let loud = tone_at(120, 440.0, 0.7);
        let raw = play(&mut Playback::new(), &[says(LOOPBACK_SESSION, crate::state::LOOPBACK_CLIENT_ID, &loud, 0)], 120);
        let mut p = Playback::new();
        p.set_leveling(true);
        let heard = play(&mut p, &[says(LOOPBACK_SESSION, crate::state::LOOPBACK_CLIENT_ID, &loud, 0)], 120);
        let (plain, kept) = (strength(span(&raw, 70, 110), 440.0), strength(span(&heard, 70, 110), 440.0));
        assert!(db(kept / plain).abs() < 0.2, "{:.2} dB", db(kept / plain));
    }

    #[test]
    fn leaving_or_a_new_connection_forgets_what_was_set_and_learned() {
        let loud = tone_at(100, 440.0, 0.8);
        let chief = tone_at(100, 1000.0, 0.2);
        let mut p = Playback::new();
        p.set_leveling(true);
        p.set_priority(0, 10, true);
        p.set_priority_dim(0, Some(-18.0));
        play(&mut p, &[says(0, 1, &loud, 0), says(0, 10, &chief, 0)], 90);
        assert!(p.adjustment(0, 1).leveled_db < -10.0 && p.adjustment(0, 1).lowered_db < -17.0);
        p.remove(0, 1);
        assert_eq!(p.adjustment(0, 1).leveled_db, 0.0, "a person who left is not remembered");
        p.clear_session(0);
        p.set_leveling(false);
        p.set_priority_dim(0, Some(-18.0));
        let usual = tone_at(90, 440.0, 0.2);
        let lone = strength(span(&play(&mut Playback::new(), &[says(0, 1, &usual, 0)], 90), 30, 80), 440.0);
        let left = play(&mut p, &[says(0, 1, &usual, 0), says(0, 10, &chief, 0)], 90);
        assert!(db(strength(span(&left, 30, 80), 440.0) / lone).abs() < 0.5, "the old connection's priority speaker still counted");
        assert_eq!(p.adjustment(0, 1), Adjustment::default());
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
