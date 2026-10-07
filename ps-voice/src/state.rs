use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::Mutex;

use crate::capture::SILENCE_DB;
use crate::codec::{CODEC_OPUS_VOICE, MAX_PACKET_BYTES};
use crate::playback::Playback;

pub const LOOPBACK_CLIENT_ID: u16 = 0xffff;
pub const LOOPBACK_SESSION: u16 = 0xffff;
pub const LANES: usize = 16;
const NO_LANE: u8 = 255;
const MIN_LANE_ROOM: usize = 24;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxMode {
    VoiceActivation,
    PushToTalk,
    Continuous,
}

impl TxMode {
    pub fn from_index(index: u8) -> Self {
        match index {
            1 => TxMode::PushToTalk,
            2 => TxMode::Continuous,
            _ => TxMode::VoiceActivation,
        }
    }

    pub fn index(self) -> u8 {
        match self {
            TxMode::VoiceActivation => 0,
            TxMode::PushToTalk => 1,
            TxMode::Continuous => 2,
        }
    }
}

pub type FrameSink = Box<dyn FnMut(u8, u8, &[u8]) + Send>;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct DeviceStatus {
    pub input: String,
    pub output: String,
    pub input_ok: bool,
    pub output_ok: bool,
}

pub struct Shared {
    pub tx_enabled: AtomicBool,
    pub mic_muted: AtomicBool,
    pub speaker_muted: AtomicBool,
    pub ptt: AtomicBool,
    whisper_lane: AtomicU8,
    on_air: AtomicU8,
    lane_room: [AtomicU16; LANES],
    pub loopback: AtomicBool,
    echo_cancel: AtomicBool,
    echo_reduction: AtomicU32,
    echo_delay: AtomicU32,
    echo_drift: AtomicU32,
    pub transmitting: AtomicBool,
    pub codec: AtomicU8,
    pub codec_quality: AtomicU8,
    tx_mode: AtomicU8,
    vad_threshold: AtomicU32,
    input_gain: AtomicU32,
    output_volume: AtomicU32,
    input_level: AtomicU32,
    output_level: AtomicU32,
    pub frames_captured: AtomicU64,
    pub frames_played: AtomicU64,
    pub capture_overruns: AtomicU64,
    pub playback: Mutex<Playback>,
    pub sink: Mutex<Option<FrameSink>>,
    pub status: Mutex<DeviceStatus>,
}

impl Default for Shared {
    fn default() -> Self {
        Self {
            tx_enabled: AtomicBool::new(false),
            mic_muted: AtomicBool::new(false),
            speaker_muted: AtomicBool::new(false),
            ptt: AtomicBool::new(false),
            whisper_lane: AtomicU8::new(0),
            on_air: AtomicU8::new(NO_LANE),
            lane_room: std::array::from_fn(|_| AtomicU16::new(0)),
            loopback: AtomicBool::new(false),
            echo_cancel: AtomicBool::new(false),
            echo_reduction: AtomicU32::new(f32::NAN.to_bits()),
            echo_delay: AtomicU32::new(f32::NAN.to_bits()),
            echo_drift: AtomicU32::new(0f32.to_bits()),
            transmitting: AtomicBool::new(false),
            codec: AtomicU8::new(CODEC_OPUS_VOICE),
            codec_quality: AtomicU8::new(6),
            tx_mode: AtomicU8::new(TxMode::VoiceActivation.index()),
            vad_threshold: AtomicU32::new((-40.0f32).to_bits()),
            input_gain: AtomicU32::new(1.0f32.to_bits()),
            output_volume: AtomicU32::new(1.0f32.to_bits()),
            input_level: AtomicU32::new(SILENCE_DB.to_bits()),
            output_level: AtomicU32::new(SILENCE_DB.to_bits()),
            frames_captured: AtomicU64::new(0),
            frames_played: AtomicU64::new(0),
            capture_overruns: AtomicU64::new(0),
            playback: Mutex::new(Playback::new()),
            sink: Mutex::new(None),
            status: Mutex::new(DeviceStatus::default()),
        }
    }
}

fn load(a: &AtomicU32) -> f32 {
    f32::from_bits(a.load(Ordering::Relaxed))
}

fn store(a: &AtomicU32, v: f32) {
    a.store(v.to_bits(), Ordering::Relaxed);
}

impl Shared {
    pub fn set_keys(&self, talk: bool, lane: u8) {
        self.ptt.store(talk, Ordering::Relaxed);
        self.whisper_lane.store(if usize::from(lane) < LANES { lane } else { 0 }, Ordering::Relaxed);
    }

    pub fn whisper_lane(&self) -> u8 {
        self.whisper_lane.load(Ordering::Relaxed)
    }

    pub fn set_lane_room(&self, lane: u8, bytes: usize) {
        if let Some(slot) = self.lane_room.get(usize::from(lane)) {
            slot.store(bytes.clamp(MIN_LANE_ROOM, MAX_PACKET_BYTES) as u16, Ordering::Relaxed);
        }
    }

    pub fn lane_room(&self, lane: u8) -> usize {
        match self.lane_room.get(usize::from(lane)).map(|slot| slot.load(Ordering::Relaxed)) {
            Some(bytes) if bytes > 0 => usize::from(bytes),
            _ => MAX_PACKET_BYTES,
        }
    }

    pub fn on_air_lane(&self) -> Option<u8> {
        match self.on_air.load(Ordering::Relaxed) {
            NO_LANE => None,
            lane => Some(lane),
        }
    }

    pub(crate) fn set_on_air(&self, lane: Option<u8>) {
        self.on_air.store(lane.unwrap_or(NO_LANE), Ordering::Relaxed);
    }

    pub fn echo_cancel(&self) -> bool {
        self.echo_cancel.load(Ordering::Relaxed)
    }

    pub fn set_echo_cancel(&self, on: bool) {
        self.echo_cancel.store(on, Ordering::Relaxed);
        if !on {
            self.set_echo_reduction(None);
        }
    }

    pub fn echo_reduction(&self) -> Option<f32> {
        Some(load(&self.echo_reduction)).filter(|db| db.is_finite())
    }

    pub fn set_echo_reduction(&self, db: Option<f32>) {
        store(&self.echo_reduction, db.unwrap_or(f32::NAN));
    }

    pub fn echo_delay_ms(&self) -> Option<f32> {
        Some(load(&self.echo_delay)).filter(|ms| ms.is_finite())
    }

    pub fn echo_drift_ppm(&self) -> f32 {
        load(&self.echo_drift)
    }

    pub fn set_echo_details(&self, delay_ms: Option<f32>, drift_ppm: f32) {
        store(&self.echo_delay, delay_ms.unwrap_or(f32::NAN));
        store(&self.echo_drift, drift_ppm);
    }

    pub fn tx_mode(&self) -> TxMode {
        TxMode::from_index(self.tx_mode.load(Ordering::Relaxed))
    }

    pub fn set_tx_mode(&self, mode: TxMode) {
        self.tx_mode.store(mode.index(), Ordering::Relaxed);
    }

    pub fn vad_threshold(&self) -> f32 {
        load(&self.vad_threshold)
    }

    pub fn set_vad_threshold(&self, db: f32) {
        store(&self.vad_threshold, db);
    }

    pub fn input_gain(&self) -> f32 {
        load(&self.input_gain)
    }

    pub fn set_input_gain(&self, gain: f32) {
        store(&self.input_gain, gain.clamp(0.0, 8.0));
    }

    pub fn output_volume(&self) -> f32 {
        load(&self.output_volume)
    }

    pub fn set_output_volume(&self, volume: f32) {
        store(&self.output_volume, volume.clamp(0.0, 4.0));
    }

    pub fn input_level(&self) -> f32 {
        load(&self.input_level)
    }

    pub fn set_input_level(&self, db: f32) {
        store(&self.input_level, db);
    }

    pub fn output_level(&self) -> f32 {
        load(&self.output_level)
    }

    pub fn set_output_level(&self, db: f32) {
        store(&self.output_level, db);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn held_keys_are_stored_for_the_transmitter() {
        let s = Shared::default();
        assert_eq!(s.whisper_lane(), 0);
        s.set_keys(true, 3);
        assert!(s.ptt.load(Ordering::Relaxed));
        assert_eq!(s.whisper_lane(), 3);
        s.set_keys(false, 200);
        assert!(!s.ptt.load(Ordering::Relaxed));
        assert_eq!(s.whisper_lane(), 0);
    }

    #[test]
    fn defaults_and_round_trips() {
        let s = Shared::default();
        assert_eq!(s.tx_mode(), TxMode::VoiceActivation);
        assert_eq!(s.vad_threshold(), -40.0);
        assert_eq!(s.input_gain(), 1.0);
        assert_eq!(s.output_volume(), 1.0);
        assert_eq!(s.input_level(), SILENCE_DB);
        assert!(!s.echo_cancel() && s.echo_reduction().is_none());
        s.set_echo_cancel(true);
        s.set_echo_reduction(Some(23.5));
        assert_eq!((s.echo_cancel(), s.echo_reduction()), (true, Some(23.5)));
        s.set_echo_cancel(false);
        assert_eq!(s.echo_reduction(), None);
        s.set_tx_mode(TxMode::Continuous);
        assert_eq!(s.tx_mode(), TxMode::Continuous);
        s.set_input_gain(100.0);
        assert_eq!(s.input_gain(), 8.0);
        s.set_output_volume(-1.0);
        assert_eq!(s.output_volume(), 0.0);
        for m in [TxMode::VoiceActivation, TxMode::PushToTalk, TxMode::Continuous] {
            assert_eq!(TxMode::from_index(m.index()), m);
        }
    }
}
