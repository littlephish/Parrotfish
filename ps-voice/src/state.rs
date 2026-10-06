use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::Mutex;

use crate::capture::SILENCE_DB;
use crate::codec::CODEC_OPUS_VOICE;
use crate::playback::Playback;

pub const LOOPBACK_CLIENT_ID: u16 = 0xffff;
pub const LOOPBACK_SESSION: u16 = 0xffff;

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

pub type FrameSink = Box<dyn FnMut(u8, &[u8]) + Send>;

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
    pub loopback: AtomicBool,
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
            loopback: AtomicBool::new(false),
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
    fn defaults_and_round_trips() {
        let s = Shared::default();
        assert_eq!(s.tx_mode(), TxMode::VoiceActivation);
        assert_eq!(s.vad_threshold(), -40.0);
        assert_eq!(s.input_gain(), 1.0);
        assert_eq!(s.output_volume(), 1.0);
        assert_eq!(s.input_level(), SILENCE_DB);
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
