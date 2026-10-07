use crate::capture::level_db;
use crate::playback::soft_limit;

const TARGET_DB: f32 = -20.0;
const MIN_GAIN_DB: f32 = -6.0;
const MAX_GAIN_DB: f32 = 24.0;
const RISE_DB: f32 = 0.25;
const FALL_DB: f32 = 0.5;
const SPEECH_MARGIN_DB: f32 = 8.0;
const SPEECH_MIN_DB: f32 = -55.0;
const SPEECH_TRACK: f32 = 0.08;
const FLOOR_FALL: f32 = 0.3;
const FLOOR_RISE: f32 = 0.0008;
const PEAK_ROOM_DB: f32 = -3.0;
const CEILING: f32 = 0.98;
const KNEE: f32 = 0.8;

fn gain_of(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

fn limited(sample: f32) -> f32 {
    if sample.abs() <= CEILING * KNEE {
        sample
    } else {
        CEILING * soft_limit(sample / CEILING)
    }
}

pub struct AutoGain {
    gain_db: f32,
    applied: f32,
    floor_db: f32,
    speech_db: f32,
}

impl Default for AutoGain {
    fn default() -> Self {
        Self::new()
    }
}

impl AutoGain {
    pub fn new() -> Self {
        Self {
            gain_db: 0.0,
            applied: 1.0,
            floor_db: 0.0,
            speech_db: TARGET_DB,
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    pub fn gain_db(&self) -> f32 {
        self.gain_db
    }

    pub fn process(&mut self, frame: &mut [f32]) {
        if frame.is_empty() {
            return;
        }
        let level = level_db(frame);
        let follow = if level < self.floor_db { FLOOR_FALL } else { FLOOR_RISE };
        self.floor_db += follow * (level - self.floor_db);
        let heard = level > SPEECH_MIN_DB;
        let mut wanted = self.gain_db;
        if heard && level > self.floor_db + SPEECH_MARGIN_DB {
            self.speech_db += SPEECH_TRACK * (level - self.speech_db);
            wanted = TARGET_DB - self.speech_db;
        }
        if heard {
            wanted = wanted.min(PEAK_ROOM_DB - level);
        }
        let wanted = wanted.clamp(MIN_GAIN_DB, MAX_GAIN_DB);
        self.gain_db += (wanted - self.gain_db).clamp(-FALL_DB, RISE_DB);

        let from = self.applied;
        let to = gain_of(self.gain_db);
        let span = frame.len() as f32;
        for (i, sample) in frame.iter_mut().enumerate() {
            let gain = from + (to - from) * (i as f32 + 1.0) / span;
            *sample = limited(*sample * gain);
        }
        self.applied = to;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

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

    fn gate(t: f32) -> f32 {
        let within = (0.6 * t).fract() / 0.6;
        if within > 1.0 {
            return 0.0;
        }
        let edge = (within / 0.02).min((1.0 - within) / 0.02).clamp(0.0, 1.0);
        0.5 - 0.5 * (PI * edge).cos()
    }

    fn speech(samples: usize, level_db: f32) -> Vec<f32> {
        let mut out = Vec::with_capacity(samples);
        let mut phase = 0.0f32;
        for n in 0..samples {
            let t = n as f32 / RATE;
            let pitch = 130.0 + 30.0 * (2.0 * PI * 0.6 * t).sin();
            phase += 2.0 * PI * pitch / RATE;
            let syllable = 0.86 + 0.14 * (2.0 * PI * 3.0 * t).sin();
            let voiced = phase.sin() + 0.5 * (2.0 * phase).sin() + 0.3 * (3.0 * phase).sin();
            out.push(gate(t) * syllable * voiced.clamp(-1.0, 1.0));
        }
        let want = 10f32.powf(level_db / 20.0);
        let rms = speaking_rms(&out, 0);
        let scale = if rms > 0.0 { want / rms } else { 0.0 };
        out.iter_mut().for_each(|s| *s *= scale);
        out
    }

    fn speaking_rms(signal: &[f32], from: usize) -> f32 {
        let (mut sum, mut count) = (0.0f32, 0usize);
        for (n, sample) in signal.iter().enumerate() {
            if gate((from + n) as f32 / RATE) > 0.9 {
                sum += sample * sample;
                count += 1;
            }
        }
        (sum / count.max(1) as f32).sqrt()
    }

    fn speaking_db(signal: &[f32], from: usize) -> f32 {
        20.0 * speaking_rms(signal, from).max(1e-12).log10()
    }

    fn noise(seed: u64, samples: usize, level_db: f32) -> Vec<f32> {
        let mut hiss = Hiss(seed | 1);
        let raw: Vec<f32> = (0..samples).map(|_| hiss.next()).collect();
        let rms = (raw.iter().map(|s| s * s).sum::<f32>() / raw.len() as f32).sqrt();
        let want = 10f32.powf(level_db / 20.0);
        raw.iter().map(|s| s * want / rms).collect()
    }

    fn run(agc: &mut AutoGain, signal: &[f32]) -> Vec<f32> {
        let mut out = Vec::with_capacity(signal.len());
        for frame in signal.chunks(FRAME) {
            let mut work = frame.to_vec();
            agc.process(&mut work);
            out.extend_from_slice(&work);
        }
        out
    }

    #[test]
    fn quiet_speech_reaches_the_target_within_a_few_seconds() {
        let voice = speech(8 * SECOND, -40.0);
        assert!((speaking_db(&voice, 0) + 40.0).abs() < 0.1);
        let mut agc = AutoGain::new();
        let out = run(&mut agc, &voice);
        let early = speaking_db(&out[3 * SECOND..4 * SECOND], 3 * SECOND);
        assert!((early - TARGET_DB).abs() < 3.0, "inside four seconds it sat at {early:.1} dB");
        let settled = speaking_db(&out[6 * SECOND..], 6 * SECOND);
        assert!((settled - TARGET_DB).abs() < 3.0, "settled at {settled:.1} dB");
        assert!(agc.gain_db() > 15.0, "the gain reached {:.1} dB", agc.gain_db());
    }

    #[test]
    fn loud_speech_is_brought_down_and_kept_under_the_ceiling() {
        let voice = speech(6 * SECOND, -6.0);
        assert!(voice.iter().all(|s| s.abs() <= 1.0), "the test signal itself fits");
        let mut agc = AutoGain::new();
        let out = run(&mut agc, &voice);
        assert!(out.iter().all(|s| s.abs() <= 0.98), "every sample stays under the ceiling");
        let settled = speaking_db(&out[4 * SECOND..], 4 * SECOND);
        assert!(settled < -10.0, "loud speech came out at {settled:.1} dB");
        assert!(agc.gain_db() <= MIN_GAIN_DB + 0.01, "the gain went to {:.1} dB", agc.gain_db());
    }

    #[test]
    fn background_noise_alone_is_not_turned_up() {
        let hiss = noise(13, 10 * SECOND, -50.0);
        let mut agc = AutoGain::new();
        let out = run(&mut agc, &hiss);
        assert!(agc.gain_db() < 3.0, "the gain crept to {:.1} dB", agc.gain_db());
        let before = 20.0 * (hiss.iter().map(|s| s * s).sum::<f32>() / hiss.len() as f32).sqrt().log10();
        let after = 20.0 * (out.iter().map(|s| s * s).sum::<f32>() / out.len() as f32).sqrt().log10();
        assert!(after < before + 3.0, "{before:.1} dB in became {after:.1} dB out");
    }

    #[test]
    fn the_gain_moves_by_small_steps_only() {
        let mut signal = speech(2 * SECOND, -45.0);
        signal.extend(speech(2 * SECOND, -4.0));
        signal.extend(vec![0.0; SECOND]);
        signal.extend(speech(2 * SECOND, -30.0));
        let mut agc = AutoGain::new();
        let mut worst = 0.0f32;
        for frame in signal.chunks(FRAME) {
            let before = agc.gain_db();
            let mut work = frame.to_vec();
            agc.process(&mut work);
            worst = worst.max((agc.gain_db() - before).abs());
            assert!(agc.gain_db() >= MIN_GAIN_DB - 0.01 && agc.gain_db() <= MAX_GAIN_DB + 0.01);
            assert!(work.iter().all(|s| s.abs() <= 0.98));
        }
        assert!(worst <= 0.5 + 1e-4, "the biggest step was {worst:.3} dB");
        assert!(worst > 0.2, "the gain did move, biggest step {worst:.3} dB");
    }

    #[test]
    fn nothing_is_moved_in_time() {
        let mut agc = AutoGain::new();
        let mut frame = vec![0.0f32; FRAME];
        frame[17] = 0.4;
        agc.process(&mut frame);
        assert!(frame[17] > 0.0);
        assert!(frame.iter().enumerate().all(|(i, s)| i == 17 || *s == 0.0));
    }

    #[test]
    fn silence_leaves_the_gain_alone() {
        let voice = speech(3 * SECOND, -35.0);
        let mut agc = AutoGain::new();
        run(&mut agc, &voice);
        let held = agc.gain_db();
        assert!(held > 1.0, "the gain had moved up to {held:.1} dB");
        let quiet = vec![0.0f32; 2 * SECOND];
        let out = run(&mut agc, &quiet);
        assert_eq!(agc.gain_db(), held);
        assert!(out.iter().all(|s| *s == 0.0));
    }

    #[test]
    fn reset_puts_it_back_to_the_state_of_a_new_one() {
        let voice = speech(2 * SECOND, -38.0);
        let mut agc = AutoGain::new();
        let first = run(&mut agc, &voice);
        assert_ne!(agc.gain_db(), 0.0);
        agc.reset();
        assert_eq!(agc.gain_db(), 0.0);
        let again = run(&mut agc, &voice);
        assert_eq!(first, again);
    }
}
