use std::collections::VecDeque;
use std::sync::atomic::Ordering;

use crate::agc::AutoGain;
use crate::codec::{Encoder, CODEC_OPUS_MUSIC, CODEC_OPUS_VOICE, FRAME_SAMPLES, MAX_PACKET_BYTES, SAMPLE_RATE};
use crate::denoise::Denoiser;
use crate::echo::EchoCanceller;
use crate::resample::Resampler;
use crate::state::{Shared, TxMode};

pub const HANGOVER_FRAMES: u32 = 20;
pub const LOOKBACK_FRAMES: usize = 2;
pub const SILENCE_DB: f32 = -96.0;

pub fn level_db(frame: &[f32]) -> f32 {
    if frame.is_empty() {
        return SILENCE_DB;
    }
    let mean_square = frame.iter().map(|s| s * s).sum::<f32>() / frame.len() as f32;
    if mean_square <= 1e-10 {
        SILENCE_DB
    } else {
        (10.0 * mean_square.log10()).max(SILENCE_DB)
    }
}

struct EchoPath {
    canceller: EchoCanceller,
    resampler: Resampler,
    raw: Vec<f32>,
    ready: Vec<f32>,
}

pub struct Transmitter {
    resampler: Resampler,
    input_rate: u32,
    pending: Vec<f32>,
    encoder: Option<Encoder>,
    codec: u8,
    quality: u8,
    lookback: VecDeque<Vec<f32>>,
    hangover: u32,
    transmitting: bool,
    lane: u8,
    packet: [u8; MAX_PACKET_BYTES],
    stereo: Vec<f32>,
    resampled: Vec<f32>,
    far: Option<(rtrb::Consumer<f32>, u32)>,
    echo: Option<Box<EchoPath>>,
    denoiser: Option<Box<Denoiser>>,
    agc: Option<Box<AutoGain>>,
}

impl Transmitter {
    pub fn new(input_rate: u32) -> Self {
        Self {
            resampler: Resampler::new(input_rate, SAMPLE_RATE, 1),
            input_rate,
            pending: Vec::new(),
            encoder: None,
            codec: 0,
            quality: 0,
            lookback: VecDeque::new(),
            hangover: 0,
            transmitting: false,
            lane: 0,
            packet: [0; MAX_PACKET_BYTES],
            stereo: Vec::new(),
            resampled: Vec::new(),
            far: None,
            echo: None,
            denoiser: None,
            agc: None,
        }
    }

    pub fn set_far(&mut self, consumer: rtrb::Consumer<f32>, rate: u32) {
        self.far = Some((consumer, rate));
        self.echo = None;
    }

    pub fn restart_echo(&mut self) {
        self.echo = None;
    }

    fn cancel_echo(&mut self, frame: &mut [f32], shared: &Shared) {
        let Some((consumer, rate)) = self.far.as_mut() else {
            return;
        };
        let path = self.echo.get_or_insert_with(|| {
            while consumer.pop().is_ok() {}
            Box::new(EchoPath {
                canceller: EchoCanceller::new(),
                resampler: Resampler::new(*rate, SAMPLE_RATE, 1),
                raw: Vec::new(),
                ready: Vec::new(),
            })
        });
        path.raw.clear();
        while let Ok(sample) = consumer.pop() {
            path.raw.push(sample);
        }
        path.ready.clear();
        path.resampler.process(&path.raw, &mut path.ready);
        path.canceller.push_far(&path.ready);
        path.canceller.process(frame);
        shared.set_echo_reduction(path.canceller.reduction_db());
        shared.set_echo_details(path.canceller.echo_delay_ms(), path.canceller.clock_drift_ppm());
    }

    pub fn set_input_rate(&mut self, input_rate: u32) {
        if input_rate != self.input_rate {
            self.input_rate = input_rate;
            self.resampler = Resampler::new(input_rate, SAMPLE_RATE, 1);
            self.pending.clear();
            self.lookback.clear();
            self.echo = None;
        }
    }

    fn ensure_encoder(&mut self, codec: u8, quality: u8) -> bool {
        let codec = if codec == CODEC_OPUS_MUSIC { CODEC_OPUS_MUSIC } else { CODEC_OPUS_VOICE };
        if self.encoder.is_none() || self.codec != codec || self.quality != quality {
            match Encoder::new(codec, quality) {
                Ok(enc) => {
                    self.encoder = Some(enc);
                    self.codec = codec;
                    self.quality = quality;
                }
                Err(_) => {
                    self.encoder = None;
                    return false;
                }
            }
        }
        true
    }

    fn encode_and_emit(&mut self, frame: &[f32], lane: u8, room: usize, sink: &mut dyn FnMut(u8, u8, &[u8])) {
        let room = room.clamp(1, MAX_PACKET_BYTES);
        let codec = self.codec;
        let Some(encoder) = self.encoder.as_mut() else {
            return;
        };
        let result = if encoder.channels() == 2 {
            self.stereo.clear();
            for s in frame {
                self.stereo.push(*s);
                self.stereo.push(*s);
            }
            encoder.encode(&self.stereo, &mut self.packet[..room])
        } else {
            encoder.encode(frame, &mut self.packet[..room])
        };
        if let Ok(n) = result {
            if n > 0 {
                sink(lane, codec, &self.packet[..n]);
            }
        }
    }

    pub fn process(&mut self, input: &[f32], shared: &Shared, sink: &mut dyn FnMut(u8, u8, &[u8])) {
        self.resampled.clear();
        self.resampler.process(input, &mut self.resampled);
        self.pending.extend_from_slice(&self.resampled);
        let mut offset = 0;
        while self.pending.len() - offset >= FRAME_SAMPLES {
            let mut frame = self.pending[offset..offset + FRAME_SAMPLES].to_vec();
            offset += FRAME_SAMPLES;
            self.process_frame(&mut frame, shared, sink);
        }
        self.pending.drain(..offset);
    }

    fn process_frame(&mut self, frame: &mut [f32], shared: &Shared, sink: &mut dyn FnMut(u8, u8, &[u8])) {
        if shared.echo_cancel() {
            self.cancel_echo(frame, shared);
        } else if self.echo.take().is_some() {
            shared.set_echo_reduction(None);
        }
        if shared.noise_suppression() {
            self.denoiser.get_or_insert_with(|| Box::new(Denoiser::new())).process(frame);
        } else {
            self.denoiser = None;
        }
        if shared.auto_gain() {
            self.agc.get_or_insert_with(|| Box::new(AutoGain::new())).process(frame);
        } else {
            self.agc = None;
        }
        let gain = shared.input_gain();
        if (gain - 1.0).abs() > 1e-3 {
            for s in frame.iter_mut() {
                *s = (*s * gain).clamp(-1.0, 1.0);
            }
        }
        let level = level_db(frame);
        shared.set_input_level(level);

        let lane = shared.whisper_lane();
        let mode = shared.tx_mode();
        let gate_open = lane != 0
            || match mode {
                TxMode::Continuous => true,
                TxMode::PushToTalk => shared.ptt.load(Ordering::Relaxed),
                TxMode::VoiceActivation => {
                    if level >= shared.vad_threshold() {
                        self.hangover = HANGOVER_FRAMES;
                        true
                    } else if self.hangover > 0 {
                        self.hangover -= 1;
                        true
                    } else {
                        false
                    }
                }
            };
        let allowed = shared.tx_enabled.load(Ordering::Relaxed)
            && !shared.mic_muted.load(Ordering::Relaxed)
            && !shared.speaker_muted.load(Ordering::Relaxed);
        let active = gate_open && allowed;

        if self.transmitting && (!active || lane != self.lane) {
            self.transmitting = false;
            self.hangover = 0;
            let ended = if self.codec == 0 { CODEC_OPUS_VOICE } else { self.codec };
            sink(self.lane, ended, &[]);
        }
        if active {
            let codec = if lane == 0 { shared.codec.load(Ordering::Relaxed) } else { CODEC_OPUS_VOICE };
            let quality = shared.codec_quality.load(Ordering::Relaxed);
            if !self.ensure_encoder(codec, quality) {
                shared.transmitting.store(false, Ordering::Relaxed);
                shared.set_on_air(None);
                return;
            }
            if !self.transmitting {
                self.transmitting = true;
                self.lane = lane;
                if let Some(encoder) = self.encoder.as_mut() {
                    encoder.reset();
                }
                if lane == 0 && mode == TxMode::VoiceActivation {
                    let earlier: Vec<Vec<f32>> = self.lookback.drain(..).collect();
                    for old in &earlier {
                        self.encode_and_emit(old, 0, MAX_PACKET_BYTES, sink);
                    }
                }
            }
            self.lookback.clear();
            let room = if lane == 0 { MAX_PACKET_BYTES } else { shared.lane_room(lane) };
            self.encode_and_emit(frame, lane, room, sink);
        } else {
            self.lookback.push_back(frame.to_vec());
            while self.lookback.len() > LOOKBACK_FRAMES {
                self.lookback.pop_front();
            }
        }
        shared.transmitting.store(active, Ordering::Relaxed);
        shared.set_on_air(if active { Some(lane) } else { None });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::Decoder;

    fn tone(frames: usize, amplitude: f32, rate: u32, start: usize) -> Vec<f32> {
        (0..frames * rate as usize / 50)
            .map(|i| {
                amplitude
                    * (2.0 * std::f32::consts::PI * 440.0 * (start + i) as f32 / rate as f32).sin()
            })
            .collect()
    }

    fn collect(tx: &mut Transmitter, shared: &Shared, input: &[f32]) -> Vec<(u8, Vec<u8>)> {
        let mut out = Vec::new();
        tx.process(input, shared, &mut |_lane, codec, data| out.push((codec, data.to_vec())));
        out
    }

    fn lanes(tx: &mut Transmitter, shared: &Shared, input: &[f32]) -> Vec<(u8, u8, usize)> {
        let mut out = Vec::new();
        tx.process(input, shared, &mut |lane, codec, data| out.push((lane, codec, data.len())));
        out
    }

    fn shape(frames: &[(u8, u8, usize)]) -> Vec<(u8, bool)> {
        frames.iter().map(|frame| (frame.0, frame.2 > 0)).collect()
    }

    #[test]
    fn a_whisper_key_opens_the_microphone_on_its_own_lane() {
        let shared = Shared::default();
        shared.tx_enabled.store(true, Ordering::Relaxed);
        let mut tx = Transmitter::new(48_000);
        let silence = vec![0.0f32; 960 * 3];
        assert!(lanes(&mut tx, &shared, &silence).is_empty());
        assert_eq!(shared.on_air_lane(), None);
        shared.set_keys(false, 2);
        let frames = lanes(&mut tx, &shared, &silence);
        assert_eq!(shape(&frames), vec![(2, true), (2, true), (2, true)]);
        assert!(frames.iter().all(|frame| frame.1 == CODEC_OPUS_VOICE));
        assert_eq!(shared.on_air_lane(), Some(2));
        shared.set_keys(false, 0);
        assert_eq!(shape(&lanes(&mut tx, &shared, &silence)), vec![(2, false)]);
        assert_eq!(shared.on_air_lane(), None);
    }

    #[test]
    fn changing_lane_mid_sentence_ends_the_old_one_first() {
        let shared = Shared::default();
        shared.set_tx_mode(TxMode::PushToTalk);
        shared.tx_enabled.store(true, Ordering::Relaxed);
        let mut tx = Transmitter::new(48_000);
        let two = vec![0.0f32; 960 * 2];
        shared.set_keys(true, 0);
        assert_eq!(shape(&lanes(&mut tx, &shared, &two)), vec![(0, true), (0, true)]);
        shared.set_keys(true, 1);
        assert_eq!(shape(&lanes(&mut tx, &shared, &two)), vec![(0, false), (1, true), (1, true)]);
        shared.set_keys(true, 5);
        assert_eq!(shape(&lanes(&mut tx, &shared, &two)), vec![(1, false), (5, true), (5, true)]);
        shared.set_keys(true, 0);
        assert_eq!(shape(&lanes(&mut tx, &shared, &two)), vec![(5, false), (0, true), (0, true)]);
        shared.set_keys(false, 0);
        assert_eq!(shape(&lanes(&mut tx, &shared, &two)), vec![(0, false)]);
        assert!(lanes(&mut tx, &shared, &two).is_empty());
    }

    #[test]
    fn whisper_frames_fit_their_lane_and_use_the_voice_codec() {
        let shared = Shared::default();
        shared.tx_enabled.store(true, Ordering::Relaxed);
        shared.codec.store(CODEC_OPUS_MUSIC, Ordering::Relaxed);
        shared.codec_quality.store(10, Ordering::Relaxed);
        assert_eq!(shared.lane_room(3), MAX_PACKET_BYTES);
        shared.set_lane_room(3, 40);
        shared.set_lane_room(4, 1);
        assert_eq!((shared.lane_room(3), shared.lane_room(4)), (40, 24));
        shared.set_keys(false, 3);
        let mut tx = Transmitter::new(48_000);
        let frames = lanes(&mut tx, &shared, &tone(10, 0.5, 48_000, 0));
        assert_eq!(frames.len(), 10);
        assert!(frames.iter().all(|frame| frame.0 == 3 && frame.1 == CODEC_OPUS_VOICE && frame.2 > 0 && frame.2 <= 40));
        shared.set_keys(false, 0);
        shared.set_tx_mode(TxMode::Continuous);
        let talk = lanes(&mut tx, &shared, &tone(3, 0.5, 48_000, 0));
        assert_eq!(talk[0], (3, CODEC_OPUS_VOICE, 0));
        assert!(talk[1..].iter().all(|frame| frame.0 == 0 && frame.1 == CODEC_OPUS_MUSIC && frame.2 > 0));
    }

    #[test]
    fn muting_stops_whispers_too() {
        let shared = Shared::default();
        shared.tx_enabled.store(true, Ordering::Relaxed);
        shared.set_keys(false, 1);
        let mut tx = Transmitter::new(48_000);
        let one = vec![0.0f32; 960];
        assert_eq!(shape(&lanes(&mut tx, &shared, &one)), vec![(1, true)]);
        shared.mic_muted.store(true, Ordering::Relaxed);
        assert_eq!(shape(&lanes(&mut tx, &shared, &one)), vec![(1, false)]);
        assert!(lanes(&mut tx, &shared, &one).is_empty());
        shared.mic_muted.store(false, Ordering::Relaxed);
        shared.tx_enabled.store(false, Ordering::Relaxed);
        assert!(lanes(&mut tx, &shared, &one).is_empty());
    }

    #[test]
    fn echo_of_the_speakers_is_taken_out_of_the_microphone() {
        let shared = Shared::default();
        shared.set_tx_mode(TxMode::Continuous);
        shared.tx_enabled.store(true, Ordering::Relaxed);
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let mut noise = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            ((seed >> 40) as f32 / (1u64 << 23) as f32) - 1.0
        };
        let mut low = 0.0f32;
        let played: Vec<f32> = (0..48_000 * 6)
            .map(|_| {
                low += 0.15 * (noise() - low);
                0.5 * low
            })
            .collect();
        let heard: Vec<f32> = (0..played.len()).map(|n| if n >= 2400 { 0.6 * played[n - 2400] } else { 0.0 }).collect();

        let mut plain = Transmitter::new(48_000);
        let mut loudest = SILENCE_DB;
        for frame in heard.chunks(960) {
            plain.process(frame, &shared, &mut |_, _, _| {});
            loudest = loudest.max(shared.input_level());
        }
        assert!(shared.echo_reduction().is_none());

        shared.set_echo_cancel(true);
        let (mut speaker, consumer) = rtrb::RingBuffer::<f32>::new(48_000);
        let mut tx = Transmitter::new(48_000);
        tx.set_far(consumer, 48_000);
        let mut sent = 0;
        let mut late = Vec::new();
        for (index, (out, frame)) in played.chunks(960).zip(heard.chunks(960)).enumerate() {
            for sample in out {
                speaker.push(*sample).unwrap();
            }
            tx.process(frame, &shared, &mut |_, _, data| sent += usize::from(!data.is_empty()));
            if index >= 250 {
                late.push(shared.input_level());
            }
        }
        let quietest = late.iter().copied().fold(0.0f32, f32::min);
        let typical = late.iter().sum::<f32>() / late.len() as f32;
        assert_eq!(sent, 300);
        assert!(typical < loudest - 25.0, "{typical:.1} dB with cancelling against {loudest:.1} dB without");
        assert!(quietest < typical + 1.0);
        assert!(shared.echo_reduction().is_some_and(|db| db > 20.0));

        shared.set_echo_cancel(false);
        tx.process(&heard[48_000..48_960], &shared, &mut |_, _, _| {});
        assert!(shared.input_level() > loudest - 6.0);
        assert!(shared.echo_reduction().is_none());
    }

    fn voice_like(samples: usize, level: f32) -> Vec<f32> {
        let mut out = Vec::with_capacity(samples);
        let mut phase = 0.0f32;
        for n in 0..samples {
            let t = n as f32 / 48_000.0;
            let pitch = 130.0 + 30.0 * (2.0 * std::f32::consts::PI * 0.6 * t).sin();
            phase += 2.0 * std::f32::consts::PI * pitch / 48_000.0;
            let syllable = 0.5 - 0.5 * (2.0 * std::f32::consts::PI * 3.0 * t).cos();
            let gate = if (0.6 * t).fract() < 0.6 { 1.0 } else { 0.0 };
            let voiced: f32 = (1..=14).map(|h| (h as f32 * phase).sin() / h as f32).sum();
            out.push(gate * syllable * voiced);
        }
        let rms = (out.iter().map(|s| s * s).sum::<f32>() / out.len() as f32).sqrt();
        let scale = if rms > 0.0 { level / rms } else { 0.0 };
        out.iter_mut().for_each(|s| *s *= scale);
        out
    }

    fn hiss(seed: u64, samples: usize, level: f32) -> Vec<f32> {
        let mut seed = seed | 1;
        (0..samples)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                level * (((seed >> 40) as f32 / (1u64 << 23) as f32) - 1.0)
            })
            .collect()
    }

    #[test]
    fn with_the_switches_off_a_frame_is_not_touched() {
        let shared = Shared::default();
        shared.set_tx_mode(TxMode::Continuous);
        shared.tx_enabled.store(true, Ordering::Relaxed);
        assert!(!shared.noise_suppression() && !shared.auto_gain());
        let mut tx = Transmitter::new(48_000);
        let mut frame = tone(1, 0.3, 48_000, 0);
        let before = frame.clone();
        tx.process_frame(&mut frame, &shared, &mut |_, _, _| {});
        assert_eq!(frame, before);

        shared.set_noise_suppression(true);
        shared.set_auto_gain(true);
        let mut changed = before.clone();
        tx.process_frame(&mut changed, &shared, &mut |_, _, _| {});
        assert_ne!(changed, before, "with them on the frame does change");

        shared.set_noise_suppression(false);
        shared.set_auto_gain(false);
        let mut again = before.clone();
        tx.process_frame(&mut again, &shared, &mut |_, _, _| {});
        assert_eq!(again, before, "switching them off brings back the plain path");
    }

    #[test]
    fn noise_suppression_keeps_the_gate_shut_but_lets_speech_through() {
        let shared = Shared::default();
        shared.set_tx_mode(TxMode::VoiceActivation);
        shared.set_vad_threshold(-40.0);
        shared.tx_enabled.store(true, Ordering::Relaxed);
        shared.set_noise_suppression(true);
        let quiet = hiss(5, 48_000, 0.011);
        assert!(level_db(&quiet) < -40.0 && level_db(&quiet) > -46.0, "{:.1} dB", level_db(&quiet));
        let mut shut = Transmitter::new(48_000);
        let mut sent = 0;
        let mut loudest = SILENCE_DB;
        for frame in quiet.chunks(960) {
            shut.process(frame, &shared, &mut |_, _, data| sent += usize::from(!data.is_empty()));
            loudest = loudest.max(shared.input_level());
        }
        assert!(loudest < -40.0, "the quiet noise peaked at {loudest:.1} dB");
        assert_eq!(sent, 0, "the gate never opened");

        let background = hiss(7, 48_000 * 2, 0.05);
        assert!(level_db(&background) > -40.0, "this noise alone would open an unhelped gate");
        shared.set_noise_suppression(false);
        let mut plain = Transmitter::new(48_000);
        let mut opened = 0;
        for frame in background.chunks(960) {
            plain.process(frame, &shared, &mut |_, _, data| opened += usize::from(!data.is_empty()));
        }
        assert!(opened > 90, "without help the noise keeps the gate open, {opened} frames");

        shared.set_noise_suppression(true);
        let mut tx = Transmitter::new(48_000);
        let mut late = 0;
        let mut after = SILENCE_DB;
        for (index, frame) in background.chunks(960).enumerate() {
            tx.process(frame, &shared, &mut |_, _, data| {
                if index >= 50 {
                    late += usize::from(!data.is_empty());
                }
            });
            if index >= 50 {
                after = after.max(shared.input_level());
            }
        }
        assert!(after < -40.0, "once settled the suppressed noise peaked at {after:.1} dB");
        assert_eq!(late, 0, "the gate stayed shut");

        let speech = voice_like(48_000 * 3, 0.1);
        let more = hiss(9, speech.len(), 0.05);
        let noisy: Vec<f32> = speech.iter().zip(more.iter()).map(|(s, n)| s + n).collect();
        let mut talk = 0;
        for frame in noisy.chunks(960) {
            tx.process(frame, &shared, &mut |_, _, data| talk += usize::from(!data.is_empty()));
        }
        assert!(talk > 80, "speech still opens the gate, {talk} frames went out");
    }

    #[test]
    fn level_meter_reads_dbfs() {
        assert_eq!(level_db(&[0.0; 960]), SILENCE_DB);
        assert_eq!(level_db(&[]), SILENCE_DB);
        assert!((level_db(&[1.0; 960]) - 0.0).abs() < 0.01);
        assert!((level_db(&[0.1; 960]) + 20.0).abs() < 0.01);
        let sine = tone(1, 1.0, 48_000, 0);
        assert!((level_db(&sine) + 3.01).abs() < 0.1);
    }

    #[test]
    fn continuous_mode_sends_every_frame() {
        let shared = Shared::default();
        shared.set_tx_mode(TxMode::Continuous);
        shared.tx_enabled.store(true, Ordering::Relaxed);
        let mut tx = Transmitter::new(48_000);
        let frames = collect(&mut tx, &shared, &vec![0.0; 960 * 5 + 100]);
        assert_eq!(frames.len(), 5);
        assert!(frames.iter().all(|(c, d)| *c == CODEC_OPUS_VOICE && !d.is_empty()));
        assert!(shared.transmitting.load(Ordering::Relaxed));
        let more = collect(&mut tx, &shared, &vec![0.0; 860]);
        assert_eq!(more.len(), 1, "leftover samples carry over between calls");
    }

    #[test]
    fn nothing_is_sent_while_disabled_or_muted() {
        let shared = Shared::default();
        shared.set_tx_mode(TxMode::Continuous);
        let mut tx = Transmitter::new(48_000);
        assert!(collect(&mut tx, &shared, &tone(5, 0.5, 48_000, 0)).is_empty());
        assert!(shared.input_level() > -20.0, "the meter still works while not transmitting");

        shared.tx_enabled.store(true, Ordering::Relaxed);
        assert_eq!(collect(&mut tx, &shared, &tone(3, 0.5, 48_000, 0)).len(), 3);

        shared.mic_muted.store(true, Ordering::Relaxed);
        let muted = collect(&mut tx, &shared, &tone(3, 0.5, 48_000, 0));
        assert_eq!(muted.len(), 1);
        assert!(muted[0].1.is_empty(), "muting ends the stream with an empty packet");
        assert!(!shared.transmitting.load(Ordering::Relaxed));

        shared.mic_muted.store(false, Ordering::Relaxed);
        shared.speaker_muted.store(true, Ordering::Relaxed);
        assert!(collect(&mut tx, &shared, &tone(3, 0.5, 48_000, 0)).is_empty());
    }

    #[test]
    fn voice_activation_gates_with_lookback_and_hangover() {
        let shared = Shared::default();
        shared.set_tx_mode(TxMode::VoiceActivation);
        shared.set_vad_threshold(-40.0);
        shared.tx_enabled.store(true, Ordering::Relaxed);
        let mut tx = Transmitter::new(48_000);

        assert!(collect(&mut tx, &shared, &tone(10, 0.0005, 48_000, 0)).is_empty());
        assert!(!shared.transmitting.load(Ordering::Relaxed));

        let speech = collect(&mut tx, &shared, &tone(5, 0.3, 48_000, 0));
        assert_eq!(speech.len(), 5 + LOOKBACK_FRAMES, "the frames just before the onset are sent too");
        assert!(speech.iter().all(|(_, d)| !d.is_empty()));
        assert!(shared.transmitting.load(Ordering::Relaxed));

        let tail = collect(&mut tx, &shared, &tone(HANGOVER_FRAMES as usize + 5, 0.0005, 48_000, 0));
        assert_eq!(tail.len(), HANGOVER_FRAMES as usize + 1);
        assert!(tail[..HANGOVER_FRAMES as usize].iter().all(|(_, d)| !d.is_empty()));
        assert!(tail[HANGOVER_FRAMES as usize].1.is_empty(), "stream ends with an empty packet");
        assert!(!shared.transmitting.load(Ordering::Relaxed));
    }

    #[test]
    fn push_to_talk_follows_the_key() {
        let shared = Shared::default();
        shared.set_tx_mode(TxMode::PushToTalk);
        shared.tx_enabled.store(true, Ordering::Relaxed);
        let mut tx = Transmitter::new(48_000);
        assert!(collect(&mut tx, &shared, &tone(3, 0.5, 48_000, 0)).is_empty());
        shared.ptt.store(true, Ordering::Relaxed);
        assert_eq!(collect(&mut tx, &shared, &tone(3, 0.5, 48_000, 0)).len(), 3);
        shared.ptt.store(false, Ordering::Relaxed);
        let end = collect(&mut tx, &shared, &tone(2, 0.5, 48_000, 0));
        assert_eq!(end.len(), 1);
        assert!(end[0].1.is_empty());
    }

    #[test]
    fn resamples_device_rate_and_follows_channel_codec() {
        let shared = Shared::default();
        shared.set_tx_mode(TxMode::Continuous);
        shared.tx_enabled.store(true, Ordering::Relaxed);
        let mut tx = Transmitter::new(44_100);
        let voice = collect(&mut tx, &shared, &tone(50, 0.4, 44_100, 0));
        assert!((48..=50).contains(&voice.len()), "{} frames", voice.len());

        let mut dec = Decoder::new(2).unwrap();
        let mut pcm = vec![0f32; FRAME_SAMPLES * 2];
        let mut energy = 0.0;
        for (_, data) in &voice[10..] {
            assert_eq!(dec.decode(data, false, &mut pcm).unwrap(), FRAME_SAMPLES);
            energy += pcm.iter().map(|s| s * s).sum::<f32>() / pcm.len() as f32;
        }
        let rms = (energy / (voice.len() - 10) as f32).sqrt();
        assert!((rms - 0.2828).abs() < 0.06, "rms {rms}");

        shared.codec.store(CODEC_OPUS_MUSIC, Ordering::Relaxed);
        shared.codec_quality.store(10, Ordering::Relaxed);
        tx.set_input_rate(48_000);
        let music = collect(&mut tx, &shared, &tone(5, 0.4, 48_000, 0));
        assert_eq!(music.len(), 5);
        assert!(music.iter().all(|(c, _)| *c == CODEC_OPUS_MUSIC));

        shared.set_input_gain(0.0);
        collect(&mut tx, &shared, &tone(2, 0.4, 48_000, 0));
        assert_eq!(shared.input_level(), SILENCE_DB);
    }
}
