use crate::capture::level_db;
use crate::codec::SAMPLE_RATE;

pub const TARGET_DB: f32 = -20.0;
pub const MAX_BOOST_DB: f32 = 12.0;
pub const MAX_CUT_DB: f32 = -40.0;
pub const CAP_DB: f32 = TARGET_DB + 6.0;
const PEAK_CEILING: f32 = 0.9;
const RISE_DB_PER_SECOND: f32 = 3.0;
const FALL_DB_PER_SECOND: f32 = 12.0;
const LIMIT_RELEASE_DB_PER_SECOND: f32 = 40.0;
const SPEECH_MIN_DB: f32 = -55.0;
const BOOST_MIN_DB: f32 = -45.0;
const SPEECH_MARGIN_DB: f32 = 8.0;
const SPEECH_UP_SECONDS: f32 = 0.3;
const SPEECH_DOWN_SECONDS: f32 = 1.5;
const FLOOR_FALL_SECONDS: f32 = 0.06;
const FLOOR_RISE_SECONDS: f32 = 25.0;
const FLOOR_MIN_DB: f32 = -65.0;
const ATTACK_SECONDS: f32 = 0.002;
const LOUDEST_DB: f32 = 20.0;

fn gain_of(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

fn follow(seconds: f32, dt: f32) -> f32 {
    1.0 - (-dt / seconds).exp()
}

#[derive(Debug, Clone, PartialEq)]
pub struct Leveler {
    gain_db: f32,
    limit_db: f32,
    applied: f32,
    floor_db: Option<f32>,
    speech_db: f32,
}

impl Default for Leveler {
    fn default() -> Self {
        Self::new()
    }
}

impl Leveler {
    pub fn new() -> Self {
        Self { gain_db: 0.0, limit_db: 0.0, applied: 1.0, floor_db: None, speech_db: TARGET_DB }
    }

    pub fn gain_db(&self) -> f32 {
        self.gain_db + self.limit_db
    }

    pub fn is_idle(&self) -> bool {
        self.applied == 1.0
    }

    pub fn rest(&mut self, enabled: bool) {
        self.limit_db = 0.0;
        self.applied = if enabled { gain_of(self.gain_db) } else { 1.0 };
    }

    pub fn process(&mut self, block: &mut [f32], channels: usize, enabled: bool) {
        let frames = block.len() / channels.max(1);
        if frames == 0 {
            return;
        }
        let to = if enabled { self.measure(block, frames) } else { 1.0 };
        let from = self.applied;
        if from == 1.0 && to == 1.0 {
            return;
        }
        let width = channels.max(1);
        let span = if to < from { ((ATTACK_SECONDS * SAMPLE_RATE as f32) as usize).clamp(1, frames) } else { frames };
        for (index, frame) in block.chunks_mut(width).enumerate() {
            let gain = if index < span { from + (to - from) * (index as f32 + 1.0) / span as f32 } else { to };
            for sample in frame {
                *sample *= gain;
            }
        }
        self.applied = to;
    }

    fn measure(&mut self, block: &[f32], frames: usize) -> f32 {
        let dt = frames as f32 / SAMPLE_RATE as f32;
        let level = level_db(block).min(LOUDEST_DB);
        let peak = block.iter().fold(0.0f32, |most, sample| most.max(sample.abs())).min(gain_of(LOUDEST_DB));
        let floor = match self.floor_db {
            Some(floor) => {
                let time = if level < floor { FLOOR_FALL_SECONDS } else { FLOOR_RISE_SECONDS };
                floor + follow(time, dt) * (level - floor)
            }
            None => level,
        }
        .max(FLOOR_MIN_DB);
        self.floor_db = Some(floor);
        let audible = level > SPEECH_MIN_DB;
        let louder = level > self.speech_db;
        let counts = audible && (louder || (level > BOOST_MIN_DB && level > floor + SPEECH_MARGIN_DB));
        if counts {
            let time = if louder { SPEECH_UP_SECONDS } else { SPEECH_DOWN_SECONDS };
            self.speech_db += follow(time, dt) * (level - self.speech_db);
        }
        let wanted = (TARGET_DB - self.speech_db).clamp(MAX_CUT_DB, MAX_BOOST_DB);
        if wanted < self.gain_db && audible {
            self.gain_db = wanted.max(self.gain_db - FALL_DB_PER_SECOND * dt);
        } else if wanted > self.gain_db && counts {
            self.gain_db = wanted.min(self.gain_db + RISE_DB_PER_SECOND * dt);
        }
        let over_level = level + self.gain_db - CAP_DB;
        let over_peak = if peak > 0.0 { 20.0 * (peak / PEAK_CEILING).log10() + self.gain_db } else { f32::MIN };
        let needed = -over_level.max(over_peak).max(0.0);
        self.limit_db = (self.limit_db + LIMIT_RELEASE_DB_PER_SECOND * dt).min(0.0).min(needed);
        gain_of(self.gain_db + self.limit_db)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    const RATE: f32 = 48_000.0;
    const SECOND: usize = 48_000;
    const BLOCK: usize = 960;

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

    fn run(leveler: &mut Leveler, signal: &[f32]) -> Vec<f32> {
        let mut out = Vec::with_capacity(signal.len());
        for block in signal.chunks(BLOCK) {
            let mut work = block.to_vec();
            leveler.process(&mut work, 1, true);
            out.extend_from_slice(&work);
        }
        out
    }

    fn block_db(samples: &[f32]) -> f32 {
        level_db(samples)
    }

    #[test]
    fn a_very_loud_voice_is_turned_down_from_its_first_block() {
        let voice = speech(6 * SECOND, -4.0);
        let mut leveler = Leveler::new();
        let out = run(&mut leveler, &voice);
        for (index, (raw, heard)) in voice.chunks(BLOCK).zip(out.chunks(BLOCK)).enumerate() {
            let (before, after) = (block_db(raw), block_db(heard));
            assert!(after <= CAP_DB + 1.5, "block {index} came out at {after:.1} dB (went in at {before:.1})");
        }
        let early = speaking_db(&out[SECOND..2 * SECOND], SECOND);
        assert!(early < TARGET_DB + 5.0, "after one second it sat at {early:.1} dB");
        let settled = speaking_db(&out[4 * SECOND..], 4 * SECOND);
        assert!((settled - TARGET_DB).abs() < 3.0, "settled at {settled:.1} dB");
        assert!(leveler.gain_db() < -12.0, "the gain went to {:.1} dB", leveler.gain_db());
    }

    #[test]
    fn a_quiet_voice_is_brought_up_slowly_and_not_past_the_limit() {
        let voice = speech(10 * SECOND, -38.0);
        let mut leveler = Leveler::new();
        let mut most = 0.0f32;
        let mut out = Vec::new();
        for block in voice.chunks(BLOCK) {
            let before = leveler.gain_db();
            let mut work = block.to_vec();
            leveler.process(&mut work, 1, true);
            most = most.max(leveler.gain_db() - before);
            out.extend_from_slice(&work);
        }
        assert!(most <= RISE_DB_PER_SECOND * 0.02 + 1e-3, "the biggest step up was {most:.3} dB in one block");
        assert!((leveler.gain_db() - MAX_BOOST_DB).abs() < 0.5, "the gain ended at {:.1} dB", leveler.gain_db());
        let settled = speaking_db(&out[8 * SECOND..], 8 * SECOND);
        assert!((settled - (-38.0 + MAX_BOOST_DB)).abs() < 1.5, "settled at {settled:.1} dB");
    }

    #[test]
    fn a_voice_at_the_usual_level_is_left_almost_alone() {
        let voice = speech(6 * SECOND, TARGET_DB);
        let mut leveler = Leveler::new();
        let out = run(&mut leveler, &voice);
        let settled = speaking_db(&out[3 * SECOND..], 3 * SECOND);
        assert!((settled - TARGET_DB).abs() < 2.5, "settled at {settled:.1} dB");
        assert!(leveler.gain_db().abs() < 2.5, "the gain is {:.1} dB", leveler.gain_db());
    }

    #[test]
    fn loud_and_quiet_people_end_up_close_together() {
        let mut levels = Vec::new();
        for level in [-3.0f32, -12.0, -20.0, -28.0] {
            let voice = speech(8 * SECOND, level);
            let out = run(&mut Leveler::new(), &voice);
            levels.push(speaking_db(&out[6 * SECOND..], 6 * SECOND));
        }
        let (low, high) = levels.iter().fold((f32::MAX, f32::MIN), |(low, high), level| (low.min(*level), high.max(*level)));
        assert!(high - low < 4.0, "25 dB apart going in, they came out at {levels:?}");
    }

    #[test]
    fn background_noise_alone_is_not_turned_up() {
        let hiss = noise(13, 10 * SECOND, -50.0);
        let mut leveler = Leveler::new();
        let out = run(&mut leveler, &hiss);
        assert!(leveler.gain_db() < 1.0, "the gain crept to {:.1} dB", leveler.gain_db());
        assert!(block_db(&out) < block_db(&hiss) + 1.0);

        let mut after_silence = vec![0.0f32; SECOND];
        after_silence.extend(noise(17, 20 * SECOND, -50.0));
        let mut leveler = Leveler::new();
        run(&mut leveler, &after_silence);
        assert!(leveler.gain_db() < 1.0, "after a silent start the gain crept to {:.1} dB", leveler.gain_db());

        let steady = noise(19, 30 * SECOND, -34.0);
        let mut leveler = Leveler::new();
        run(&mut leveler, &steady);
        assert!(leveler.gain_db() < 1.0, "steady louder noise was turned up by {:.1} dB", leveler.gain_db());
    }

    #[test]
    fn loud_steady_sound_is_turned_down_too() {
        let roar = noise(23, 6 * SECOND, -6.0);
        let mut leveler = Leveler::new();
        let out = run(&mut leveler, &roar);
        let settled = block_db(&out[4 * SECOND..]);
        assert!((settled - TARGET_DB).abs() < 2.0, "it settled at {settled:.1} dB");
    }

    #[test]
    fn a_pause_leaves_the_gain_where_it_was() {
        let voice = speech(4 * SECOND, -38.0);
        let mut leveler = Leveler::new();
        run(&mut leveler, &voice);
        let held = leveler.gain_db();
        assert!(held > 3.0 && held < MAX_BOOST_DB - 3.0, "the gain was on its way up at {held:.1} dB");
        let quiet = noise(5, 3 * SECOND, -62.0);
        run(&mut leveler, &quiet);
        assert!((leveler.gain_db() - held).abs() < 0.05, "it moved to {:.2} dB during the pause", leveler.gain_db());
    }

    #[test]
    fn a_shout_is_held_down_while_it_lasts_and_soon_forgotten() {
        let mut signal = speech(4 * SECOND, TARGET_DB);
        let shout_at = signal.len();
        signal.extend(noise(3, SECOND / 5, -4.0));
        let after_at = signal.len();
        signal.extend(speech(4 * SECOND, TARGET_DB));
        let mut leveler = Leveler::new();
        let out = run(&mut leveler, &signal);
        for block in out[shout_at..after_at].chunks(BLOCK) {
            assert!(block_db(block) <= CAP_DB + 1.5, "a block of the shout came out at {:.1} dB", block_db(block));
        }
        let soon = speaking_db(&out[after_at + SECOND..after_at + 3 * SECOND], SECOND);
        assert!(soon > TARGET_DB - 6.0, "one to three seconds after the shout the voice sat at {soon:.1} dB");
        let later = speaking_db(&out[after_at + 3 * SECOND..], 3 * SECOND);
        assert!((later - TARGET_DB).abs() < 3.0, "later it sat at {later:.1} dB");
    }

    #[test]
    fn a_boosted_voice_does_not_clip() {
        let mut voice = speech(6 * SECOND, -34.0);
        for index in (BLOCK / 2..voice.len()).step_by(4_800) {
            voice[index] = 0.5;
        }
        let out = run(&mut Leveler::new(), &voice);
        let peak = out.iter().fold(0.0f32, |most, sample| most.max(sample.abs()));
        assert!(peak <= PEAK_CEILING + 0.01, "the highest sample was {peak:.3}");
    }

    #[test]
    fn turning_down_is_always_a_short_ramp_never_a_step() {
        let voice = speech(8 * SECOND, -36.0);
        let mut leveler = Leveler::new();
        run(&mut leveler, &voice);
        let boosted = 10f32.powf(leveler.gain_db() / 20.0);
        assert!(boosted > 2.0, "the gain stood at {boosted}");
        let mut block = vec![0.5f32; BLOCK];
        leveler.process(&mut block, 1, true);
        let (first, last) = (block[0] / 0.5, block[BLOCK - 1] / 0.5);
        assert!(first > boosted * 0.95, "the first sample already had the gain {first} instead of about {boosted}");
        assert!(last < 0.5, "the block ended at the gain {last}");
        let largest = block.windows(2).map(|pair| (pair[1] - pair[0]).abs()).fold(0.0f32, f32::max);
        assert!(largest < 0.02, "the largest step in the block was {largest}");
        let reached = block.iter().position(|sample| (*sample / 0.5 - last).abs() < 1e-6).unwrap();
        assert!((48..=144).contains(&reached), "the new gain was reached after {reached} samples");
    }

    #[test]
    fn the_limiter_lets_go_while_the_person_is_quiet() {
        let mut signal = speech(4 * SECOND, TARGET_DB);
        signal.extend(noise(3, SECOND / 10, -3.0));
        let mut leveler = Leveler::new();
        run(&mut leveler, &signal);
        let held = leveler.gain_db();
        assert!(held < -8.0, "right after a shout the gain stood at {held:.1} dB");
        leveler.rest(true);
        let rested = leveler.gain_db();
        assert!(rested > held + 6.0 && rested > -4.0, "after the pause it stood at {rested:.1} dB");
        let next = speech(SECOND, TARGET_DB);
        let out = run(&mut leveler, &next);
        let start = speaking_db(&out[..SECOND / 4], 0);
        assert!(start > TARGET_DB - 4.0, "the next words began at {start:.1} dB");
        leveler.rest(false);
        assert!(leveler.is_idle());
    }

    #[test]
    fn odd_input_does_not_break_it() {
        let mut leveler = Leveler::new();
        leveler.process(&mut [], 1, true);
        leveler.process(&mut [], 0, true);
        leveler.process(&mut [0.3], 1, true);
        leveler.process(&mut [0.3, -0.3], 2, true);
        leveler.process(&mut vec![f32::INFINITY; BLOCK], 1, true);
        leveler.process(&mut vec![f32::NAN; BLOCK], 1, true);
        leveler.process(&mut vec![0.0; BLOCK], 1, true);
        leveler.process(&mut vec![1.0e30; BLOCK], 1, true);
        leveler.process(&mut vec![1.0e15; BLOCK], 1, true);
        assert!(leveler.gain_db().is_finite(), "the gain became {}", leveler.gain_db());
        assert!(leveler.gain_db() > -80.0, "one absurd block pushed the gain to {} dB", leveler.gain_db());
        let voice = speech(8 * SECOND, -6.0);
        let out = run(&mut leveler, &voice);
        assert!(out.iter().all(|sample| sample.is_finite()));
        let settled = speaking_db(&out[6 * SECOND..], 6 * SECOND);
        assert!((settled - TARGET_DB).abs() < 3.0, "afterwards it settled at {settled:.1} dB");
        let mut short = Leveler::new();
        for sample in &voice[..SECOND] {
            let mut one = [*sample];
            short.process(&mut one, 1, true);
            assert!(one[0].is_finite());
        }
        assert!(short.gain_db().is_finite());
    }

    #[test]
    fn nothing_is_moved_in_time() {
        let mut leveler = Leveler::new();
        let mut block = vec![0.0f32; BLOCK];
        block[17] = 0.4;
        leveler.process(&mut block, 1, true);
        assert!(block[17] > 0.0);
        assert!(block.iter().enumerate().all(|(index, sample)| index == 17 || *sample == 0.0));
    }

    #[test]
    fn both_channels_get_the_same_gain() {
        let voice = speech(2 * SECOND, -6.0);
        let stereo: Vec<f32> = voice.iter().flat_map(|sample| [*sample, *sample * 0.5]).collect();
        let mut leveler = Leveler::new();
        for block in stereo.chunks(BLOCK * 2) {
            let mut work = block.to_vec();
            leveler.process(&mut work, 2, true);
            for (before, after) in block.chunks(2).zip(work.chunks(2)) {
                if before[0].abs() > 1e-4 {
                    let (left, right) = (after[0] / before[0], after[1] / before[1]);
                    assert!((left - right).abs() < 1e-4, "left {left} right {right}");
                }
            }
        }
        assert!(leveler.gain_db() < -6.0);
    }

    #[test]
    fn switching_it_off_goes_back_to_the_plain_sound_without_a_step() {
        let voice = speech(3 * SECOND, -4.0);
        let mut leveler = Leveler::new();
        run(&mut leveler, &voice);
        assert!(!leveler.is_idle());
        let mut block = vec![0.25f32; BLOCK];
        leveler.process(&mut block, 1, false);
        let largest = block.windows(2).map(|pair| (pair[1] - pair[0]).abs()).fold(0.0f32, f32::max);
        assert!(largest < 0.01, "the largest step was {largest}");
        assert!((block[BLOCK - 1] - 0.25).abs() < 1e-6);
        assert!(leveler.is_idle());
        let mut plain = vec![0.25f32; BLOCK];
        leveler.process(&mut plain, 1, false);
        assert!(plain.iter().all(|sample| *sample == 0.25));
    }

    #[test]
    fn blocks_of_another_length_behave_the_same() {
        let voice = speech(6 * SECOND, -6.0);
        let mut by_twenty = Leveler::new();
        let mut by_ten = Leveler::new();
        for block in voice.chunks(960) {
            by_twenty.process(&mut block.to_vec(), 1, true);
        }
        for block in voice.chunks(480) {
            by_ten.process(&mut block.to_vec(), 1, true);
        }
        assert!((by_twenty.gain_db() - by_ten.gain_db()).abs() < 1.5, "{} and {}", by_twenty.gain_db(), by_ten.gain_db());
    }
}
