use std::f32::consts::PI;

use crate::codec::SAMPLE_RATE;

const PEAK: f32 = 0.2;
const FADE_MS: f32 = 12.0;
const MAX_CUES: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cue {
    Connected,
    ConnectionLost,
    Joined,
    Left,
    Moved,
    Message,
    Poke,
    MicOff,
    MicOn,
    SoundOff,
    SoundOn,
}

fn tune(cue: Cue) -> (&'static [f32], f32, f32) {
    match cue {
        Cue::Connected => (&[494.0, 988.0], 100.0, 8.0),
        Cue::ConnectionLost => (&[988.0, 494.0], 130.0, 8.0),
        Cue::Joined => (&[659.0, 1318.0], 70.0, 0.0),
        Cue::Left => (&[1318.0, 659.0], 70.0, 0.0),
        Cue::Moved => (&[740.0, 554.0, 740.0], 60.0, 0.0),
        Cue::Message => (&[988.0], 110.0, 0.0),
        Cue::Poke => (&[1760.0, 1760.0, 1760.0], 50.0, 14.0),
        Cue::MicOff => (&[220.0], 150.0, 0.0),
        Cue::MicOn => (&[440.0], 150.0, 0.0),
        Cue::SoundOff => (&[330.0, 165.0], 110.0, 0.0),
        Cue::SoundOn => (&[165.0, 330.0], 110.0, 0.0),
    }
}

fn samples_of(ms: f32) -> usize {
    (ms * SAMPLE_RATE as f32 / 1000.0) as usize
}

fn note(out: &mut Vec<f32>, freq: f32, ms: f32) {
    let len = samples_of(ms);
    let fade = samples_of(FADE_MS).clamp(1, (len / 2).max(1));
    let step = 2.0 * PI * freq / SAMPLE_RATE as f32;
    for i in 0..len {
        let phase = step * i as f32;
        let edge = i.min(len - 1 - i);
        let shape = if edge >= fade {
            1.0
        } else {
            0.5 - 0.5 * (PI * edge as f32 / fade as f32).cos()
        };
        out.push(PEAK * shape * (phase.sin() + (3.0 * phase).sin() / 9.0));
    }
}

fn render(cue: Cue, out: &mut Vec<f32>) {
    let (freqs, ms, gap) = tune(cue);
    out.clear();
    for (index, freq) in freqs.iter().enumerate() {
        if index > 0 {
            out.extend(std::iter::repeat(0.0).take(samples_of(gap)));
        }
        note(out, *freq, ms);
    }
}

pub fn cue_samples(cue: Cue) -> Vec<f32> {
    let mut out = Vec::new();
    render(cue, &mut out);
    out
}

struct Slot {
    samples: Vec<f32>,
    at: usize,
}

impl Slot {
    fn free(&self) -> bool {
        self.at >= self.samples.len()
    }
}

pub struct CueMixer {
    slots: [Slot; MAX_CUES],
}

impl Default for CueMixer {
    fn default() -> Self {
        Self::new()
    }
}

impl CueMixer {
    pub fn new() -> Self {
        Self {
            slots: std::array::from_fn(|_| Slot { samples: Vec::new(), at: 0 }),
        }
    }

    pub fn start(&mut self, cue: Cue) {
        let Some(slot) = self.slots.iter_mut().find(|slot| slot.free()) else {
            return;
        };
        render(cue, &mut slot.samples);
        slot.at = 0;
    }

    pub fn mix(&mut self, block: &mut [f32], channels: usize, gain: f32) -> bool {
        let channels = channels.max(1);
        let frames = block.len() / channels;
        if frames == 0 {
            return false;
        }
        let mut played = false;
        for slot in self.slots.iter_mut() {
            if slot.free() {
                continue;
            }
            played = true;
            let take = frames.min(slot.samples.len() - slot.at);
            for i in 0..take {
                let value = slot.samples[slot.at + i] * gain;
                for sample in &mut block[i * channels..(i + 1) * channels] {
                    *sample += value;
                }
            }
            slot.at += take;
        }
        played
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Cue; 11] = [
        Cue::Connected,
        Cue::ConnectionLost,
        Cue::Joined,
        Cue::Left,
        Cue::Moved,
        Cue::Message,
        Cue::Poke,
        Cue::MicOff,
        Cue::MicOn,
        Cue::SoundOff,
        Cue::SoundOn,
    ];

    const SLICES: usize = 8;
    const BANDS: usize = 24;

    fn fingerprint(samples: &[f32]) -> Vec<f32> {
        let mut out = vec![0.0; SLICES * BANDS];
        let span = samples.len() / SLICES;
        for slice in 0..SLICES {
            let part = &samples[slice * span..(slice + 1) * span];
            for band in 0..BANDS {
                let freq = 120.0 * 1.17f32.powi(band as i32);
                let step = 2.0 * PI * freq / SAMPLE_RATE as f32;
                let (mut re, mut im) = (0.0f32, 0.0f32);
                for (n, sample) in part.iter().enumerate() {
                    let phase = step * n as f32;
                    re += sample * phase.cos();
                    im += sample * phase.sin();
                }
                out[slice * BANDS + band] = (re * re + im * im).sqrt() / span.max(1) as f32;
            }
        }
        let size = out.iter().map(|v| v * v).sum::<f32>().sqrt();
        if size > 0.0 {
            out.iter_mut().for_each(|v| *v /= size);
        }
        out
    }

    fn alike(a: &[f32], b: &[f32]) -> f32 {
        a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
    }

    #[test]
    fn every_cue_is_a_short_soft_sound() {
        for cue in ALL {
            let samples = cue_samples(cue);
            let ms = samples.len() as f32 * 1000.0 / SAMPLE_RATE as f32;
            assert!((80.0..=450.0).contains(&ms), "{cue:?} lasts {ms:.0} ms");
            assert!(samples.iter().all(|s| s.is_finite()), "{cue:?} has broken samples");
            let peak = samples.iter().fold(0.0f32, |most, s| most.max(s.abs()));
            assert!(peak <= 0.25, "{cue:?} peaks at {peak:.3}");
            assert!(peak > 0.05, "{cue:?} is audible, peak {peak:.3}");
            assert!(samples[0].abs() < 0.001, "{cue:?} starts at {}", samples[0]);
            assert!(samples[samples.len() - 1].abs() < 0.001, "{cue:?} ends at {}", samples[samples.len() - 1]);
        }
    }

    #[test]
    fn no_two_cues_sound_the_same() {
        let marks: Vec<Vec<f32>> = ALL.iter().map(|cue| fingerprint(&cue_samples(*cue))).collect();
        let mut worst = 0.0f32;
        for (i, first) in marks.iter().enumerate() {
            assert!((alike(first, first) - 1.0).abs() < 1e-3);
            for (j, second) in marks.iter().enumerate().skip(i + 1) {
                let score = alike(first, second);
                worst = worst.max(score);
                assert!(score < 0.9, "{:?} and {:?} are {score:.2} alike", ALL[i], ALL[j]);
            }
        }
        assert!(worst > 0.0, "the closest pair scored {worst:.2}");
    }

    #[test]
    fn a_cue_crosses_block_boundaries_in_one_piece() {
        let want = cue_samples(Cue::Joined);
        let mut mixer = CueMixer::new();
        mixer.start(Cue::Joined);
        let mut heard = Vec::new();
        let mut blocks = 0;
        loop {
            let mut block = vec![0.0f32; 300 * 2];
            let played = mixer.mix(&mut block, 2, 0.5);
            blocks += 1;
            if !played {
                assert!(block.iter().all(|s| *s == 0.0));
                break;
            }
            for frame in block.chunks(2) {
                assert_eq!(frame[0], frame[1]);
                heard.push(frame[0]);
            }
        }
        assert!(blocks > 4, "the cue spanned {blocks} blocks");
        assert!(heard.len() >= want.len());
        for (n, wanted) in want.iter().enumerate() {
            assert_eq!(heard[n], wanted * 0.5, "sample {n} of the cue");
        }
        assert!(heard[want.len()..].iter().all(|s| *s == 0.0));
    }

    #[test]
    fn a_fifth_cue_at_once_is_dropped() {
        let one = cue_samples(Cue::MicOn);
        let mut mixer = CueMixer::new();
        let mut block = vec![0.0f32; 960 * 2];
        assert!(!mixer.mix(&mut block, 2, 1.0));
        assert!(block.iter().all(|s| *s == 0.0), "nothing playing leaves the block alone");
        for _ in 0..5 {
            mixer.start(Cue::MicOn);
        }
        assert!(mixer.mix(&mut block, 2, 1.0));
        let mut loudest = 0.0f32;
        for (n, frame) in block.chunks(2).enumerate() {
            assert!((frame[0] - 4.0 * one[n]).abs() < 1e-6, "sample {n} is not four cues deep");
            loudest = loudest.max(frame[0].abs());
        }
        assert!(loudest > 0.5);
    }

    #[test]
    fn a_finished_cue_leaves_room_for_the_next_one() {
        let one = cue_samples(Cue::Poke);
        let mut mixer = CueMixer::new();
        for _ in 0..4 {
            mixer.start(Cue::Poke);
        }
        let mut block = vec![0.0f32; one.len() * 2];
        assert!(mixer.mix(&mut block, 2, 1.0));
        assert!(!mixer.mix(&mut vec![0.0f32; 2], 2, 1.0), "all four ran out");
        mixer.start(Cue::Poke);
        let mut after = vec![0.0f32; 480 * 2];
        assert!(mixer.mix(&mut after, 2, 1.0));
        assert!((after[0] - one[0]).abs() < 1e-6);
    }

    #[test]
    fn mono_blocks_get_the_cue_too() {
        let one = cue_samples(Cue::Message);
        let mut mixer = CueMixer::new();
        mixer.start(Cue::Message);
        let mut block = vec![0.0f32; 480];
        assert!(mixer.mix(&mut block, 1, 1.0));
        for (n, sample) in block.iter().enumerate() {
            assert_eq!(*sample, one[n]);
        }
    }
}
