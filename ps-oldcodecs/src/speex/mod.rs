use crate::Corrupt;

mod bits;
mod cb;
mod filters;
mod lsp;
mod ltp;
mod math;
mod modes;
mod nb;
mod sb;
mod tables;

use bits::Bits;
use nb::Nb;
use sb::{Low, Sb};

pub const MAX_FRAMES_PER_PACKET: usize = 10;

const MAX_FRAME: usize = 640;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Band {
    Narrow,
    Wide,
    UltraWide,
}

impl Band {
    pub fn sample_rate(self) -> u32 {
        match self {
            Band::Narrow => 8_000,
            Band::Wide => 16_000,
            Band::UltraWide => 32_000,
        }
    }

    pub fn frame_samples(self) -> usize {
        match self {
            Band::Narrow => 160,
            Band::Wide => 320,
            Band::UltraWide => 640,
        }
    }
}

#[derive(Clone)]
enum Layers {
    Narrow(Nb),
    Wide(Sb<Nb>),
    UltraWide(Sb<Sb<Nb>>),
}

impl Layers {
    fn new(band: Band) -> Self {
        match band {
            Band::Narrow => Layers::Narrow(Nb::new(false)),
            Band::Wide => Layers::Wide(Sb::new(Nb::new(true), &modes::WB_MODE)),
            Band::UltraWide => Layers::UltraWide(Sb::new(Sb::new(Nb::new(true), &modes::WB_MODE), &modes::UWB_MODE)),
        }
    }

    fn decode(&mut self, bits: &mut Bits, out: &mut [f32]) -> Result<(), Corrupt> {
        match self {
            Layers::Narrow(low) => low.decode(bits, out, false),
            Layers::Wide(low) => low.decode(bits, out, false),
            Layers::UltraWide(low) => low.decode(bits, out, false),
        }
    }

    fn conceal(&mut self, out: &mut [f32]) {
        match self {
            Layers::Narrow(low) => low.conceal(out, false),
            Layers::Wide(low) => low.conceal(out, false),
            Layers::UltraWide(low) => low.conceal(out, false),
        }
    }

    fn sane(&self) -> bool {
        match self {
            Layers::Narrow(low) => low.sane(),
            Layers::Wide(low) => low.sane(),
            Layers::UltraWide(low) => low.sane(),
        }
    }

    fn restart(&mut self) {
        match self {
            Layers::Narrow(low) => low.restart(),
            Layers::Wide(low) => low.restart(),
            Layers::UltraWide(low) => low.restart(),
        }
    }

    fn set_enhancement(&mut self, on: bool) {
        match self {
            Layers::Narrow(low) => low.set_enhancement(on),
            Layers::Wide(low) => low.set_enhancement(on),
            Layers::UltraWide(low) => low.set_enhancement(on),
        }
    }
}

pub struct Decoder {
    band: Band,
    enhance: bool,
    layers: Layers,
    frame: [f32; MAX_FRAME],
}

fn append(out: &mut Vec<f32>, frame: &[f32]) {
    out.extend(frame.iter().map(|v| {
        let sample = v / 32768.0;
        if sample.is_finite() {
            sample.clamp(-1.0, 1.0)
        } else {
            0.0
        }
    }));
}

impl Decoder {
    pub fn new(band: Band) -> Self {
        Self { band, enhance: true, layers: Layers::new(band), frame: [0.0; MAX_FRAME] }
    }

    pub fn band(&self) -> Band {
        self.band
    }

    pub fn set_enhancement(&mut self, on: bool) {
        self.enhance = on;
        self.layers.set_enhancement(on);
    }

    pub fn decode(&mut self, packet: &[u8], out: &mut Vec<f32>) -> Result<usize, Corrupt> {
        let samples = self.band.frame_samples();
        let mut bits = Bits::new(packet);
        let Some(head) = bits.peek_at(0, 5) else { return Err(Corrupt) };

        let mut layers = self.layers.clone();
        let mut frame = self.frame;
        layers.decode(&mut bits, &mut frame[..samples])?;
        let size = bits.pos();
        if size < 5 {
            return Err(Corrupt);
        }
        self.layers = layers;
        self.frame = frame;
        self.settle();
        append(out, &self.frame[..samples]);

        let total = bits.total();
        let whole_bytes = size.div_ceil(8);
        let bit_to_bit = total >= 2 * size && bits.peek_at(size, 5) == Some(head);
        let byte_to_byte = size % 8 != 0
            && packet.len() as u64 % whole_bytes == 0
            && bits.peek_at(8 * whole_bytes, 5) == Some(head);
        let stride = if bit_to_bit {
            size
        } else if byte_to_byte {
            8 * whole_bytes
        } else {
            return Ok(1);
        };

        let mut frames = 1;
        while frames < MAX_FRAMES_PER_PACKET {
            let Some(start) = stride.checked_mul(frames as u64) else { break };
            if start + size > total || bits.peek_at(start, 5) != Some(head) {
                break;
            }
            let kept_layers = self.layers.clone();
            let kept_frame = self.frame;
            bits.seek(start);
            let done = self.layers.decode(&mut bits, &mut self.frame[..samples]).is_ok();
            if !done || bits.pos() != start + size {
                self.layers = kept_layers;
                self.frame = kept_frame;
                break;
            }
            self.settle();
            append(out, &self.frame[..samples]);
            frames += 1;
        }
        Ok(frames)
    }

    pub fn conceal(&mut self, out: &mut Vec<f32>) {
        let samples = self.band.frame_samples();
        self.layers.conceal(&mut self.frame[..samples]);
        self.settle();
        append(out, &self.frame[..samples]);
    }

    pub fn reset(&mut self) {
        self.layers = Layers::new(self.band);
        self.frame = [0.0; MAX_FRAME];
        self.layers.set_enhancement(self.enhance);
    }

    fn settle(&mut self) {
        let samples = self.band.frame_samples();
        let sound_is_sane = self.frame[..samples].iter().all(|v| v.is_finite());
        if !sound_is_sane || !self.layers.sane() {
            self.layers.restart();
            self.frame = [0.0; MAX_FRAME];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOLERANCE: f32 = 2.0;
    const FULL_SET_TOLERANCE: f32 = 1.0;

    macro_rules! stream {
        ($name:literal) => {
            (
                $name,
                &include_bytes!(concat!("../../tests/data/speex/", $name, ".pkt"))[..],
                &include_bytes!(concat!("../../tests/data/speex/", $name, ".s16"))[..],
            )
        };
    }

    fn records(data: &[u8]) -> Vec<Option<&[u8]>> {
        let mut out = Vec::new();
        let mut at = 0;
        while at + 2 <= data.len() {
            let length = usize::from(data[at]) | usize::from(data[at + 1]) << 8;
            at += 2;
            if length == 0xFFFF {
                out.push(None);
            } else {
                out.push(Some(&data[at..at + length]));
                at += length;
            }
        }
        out
    }

    fn wanted(data: &[u8]) -> Vec<f32> {
        data.chunks_exact(2).map(|pair| f32::from(i16::from_le_bytes([pair[0], pair[1]]))).collect()
    }

    fn decode_all(band: Band, enhance: bool, stream: &[u8]) -> Vec<f32> {
        let mut decoder = Decoder::new(band);
        decoder.set_enhancement(enhance);
        let mut out = Vec::new();
        let mut frames = 1;
        for record in records(stream) {
            match record {
                Some(packet) => frames = decoder.decode(packet, &mut out).expect("a reference packet decodes"),
                None => (0..frames).for_each(|_| decoder.conceal(&mut out)),
            }
        }
        out
    }

    fn worst_difference(name: &str, got: &[f32], want: &[f32]) -> f32 {
        assert_eq!(got.len(), want.len(), "{name}: number of samples");
        let mut worst = 0.0f32;
        for (g, w) in got.iter().zip(want) {
            assert!(g.is_finite() && g.abs() <= 1.0, "{name}: a sample of {g}");
            worst = worst.max((g * 32768.0 - w).abs());
        }
        worst
    }

    fn check(band: Band, enhance: bool, (name, stream, want): (&str, &[u8], &[u8])) {
        let got = decode_all(band, enhance, stream);
        let worst = worst_difference(name, &got, &wanted(want));
        assert!(worst <= TOLERANCE, "{name}: off by up to {worst} of 32768");
    }

    #[test]
    fn rates_and_frame_lengths() {
        assert_eq!((Band::Narrow.sample_rate(), Band::Narrow.frame_samples()), (8_000, 160));
        assert_eq!((Band::Wide.sample_rate(), Band::Wide.frame_samples()), (16_000, 320));
        assert_eq!((Band::UltraWide.sample_rate(), Band::UltraWide.frame_samples()), (32_000, 640));
        assert_eq!(Decoder::new(Band::Wide).band(), Band::Wide);
    }

    #[test]
    fn narrowband_matches_the_reference_decoder() {
        for vector in [
            stream!("nb_q0"),
            stream!("nb_q1"),
            stream!("nb_q2"),
            stream!("nb_q3"),
            stream!("nb_q6"),
            stream!("nb_q8"),
            stream!("nb_q10"),
        ] {
            check(Band::Narrow, true, vector);
        }
    }

    #[test]
    fn wideband_matches_the_reference_decoder() {
        for vector in [stream!("wb_q0"), stream!("wb_q4"), stream!("wb_q6"), stream!("wb_q10")] {
            check(Band::Wide, true, vector);
        }
    }

    #[test]
    fn ultra_wideband_matches_the_reference_decoder() {
        for vector in [stream!("uwb_q1"), stream!("uwb_q5"), stream!("uwb_q10")] {
            check(Band::UltraWide, true, vector);
        }
    }

    #[test]
    fn streams_whose_bit_rate_varies_match_the_reference_decoder() {
        check(Band::Narrow, true, stream!("nb_vbr8"));
        check(Band::Wide, true, stream!("wb_vbr8"));
        check(Band::UltraWide, true, stream!("uwb_vbr8"));
        check(Band::Narrow, true, stream!("nb_vbr3_quiet"));
        check(Band::Wide, true, stream!("wb_vbr3_quiet"));
        check(Band::Narrow, true, stream!("nb_q5_quiet"));
    }

    #[test]
    fn frames_packed_bit_to_bit_in_one_packet_are_all_decoded() {
        check(Band::Narrow, true, stream!("nb_q4_x2"));
        check(Band::Narrow, true, stream!("nb_q4_x5"));
        check(Band::Narrow, true, stream!("nb_q6_x3"));
        check(Band::Narrow, true, stream!("nb_q2_x2"));
        check(Band::Wide, true, stream!("wb_q6_x3"));
        check(Band::UltraWide, true, stream!("uwb_q5_x2"));
        let (_, packets, _) = stream!("nb_q4_x5");
        let first = records(packets)[0].expect("a packet");
        let mut out = Vec::new();
        assert_eq!(Decoder::new(Band::Narrow).decode(first, &mut out), Ok(5));
        assert_eq!(out.len(), 5 * 160);
    }

    #[test]
    fn frames_packed_byte_to_byte_in_one_packet_are_all_decoded() {
        for (band, group_size, (name, packets, want)) in [
            (Band::Narrow, 3, stream!("nb_q6")),
            (Band::Narrow, 2, stream!("nb_q2")),
            (Band::Narrow, 4, stream!("nb_q1")),
            (Band::Narrow, 2, stream!("nb_q3")),
            (Band::Wide, 2, stream!("wb_q6")),
            (Band::UltraWide, 2, stream!("uwb_q5")),
        ] {
            let singles: Vec<&[u8]> = records(packets).into_iter().flatten().collect();
            let mut decoder = Decoder::new(band);
            let mut out = Vec::new();
            for group in singles.chunks(group_size) {
                let joined: Vec<u8> = group.concat();
                assert_eq!(decoder.decode(&joined, &mut out), Ok(group.len()), "{name}");
            }
            let worst = worst_difference(name, &out, &wanted(want));
            assert!(worst <= TOLERANCE, "{name}: off by up to {worst} of 32768");
        }
    }

    #[test]
    fn a_frame_of_another_kind_ends_the_packet_and_leaves_no_trace() {
        let (name, packets, want) = stream!("nb_q6");
        let (_, others, _) = stream!("nb_q10");
        let singles: Vec<&[u8]> = records(packets).into_iter().flatten().collect();
        let stray = records(others)[5].expect("a packet");
        let mut decoder = Decoder::new(Band::Narrow);
        let mut out = Vec::new();
        let mixed: Vec<u8> = [singles[0], stray].concat();
        assert_eq!(decoder.decode(&mixed, &mut out), Ok(1));
        for packet in &singles[1..] {
            assert_eq!(decoder.decode(packet, &mut out), Ok(1));
        }
        let worst = worst_difference(name, &out, &wanted(want));
        assert!(worst <= TOLERANCE, "{name}: off by up to {worst} of 32768");
    }

    #[test]
    fn lost_packets_are_filled_in_like_the_reference_decoder_does() {
        check(Band::Narrow, true, stream!("nb_q6_loss"));
        check(Band::Wide, true, stream!("wb_q6_loss"));
        check(Band::UltraWide, true, stream!("uwb_q6_loss"));
    }

    #[test]
    fn the_enhancer_can_be_turned_off() {
        let (_, packets, _) = stream!("nb_q6");
        let want = include_bytes!("../../tests/data/speex/nb_q6_plain.s16");
        check(Band::Narrow, false, ("nb_q6_plain", packets, want));
        let (_, packets, _) = stream!("wb_q6");
        let want = include_bytes!("../../tests/data/speex/wb_q6_plain.s16");
        check(Band::Wide, false, ("wb_q6_plain", packets, want));
    }

    #[test]
    fn starting_over_gives_the_same_sound_again() {
        let (_, packets, _) = stream!("wb_q6");
        let mut decoder = Decoder::new(Band::Wide);
        let mut first = Vec::new();
        for packet in records(packets).into_iter().flatten() {
            decoder.decode(packet, &mut first).expect("a reference packet decodes");
        }
        decoder.reset();
        let mut second = Vec::new();
        for packet in records(packets).into_iter().flatten() {
            decoder.decode(packet, &mut second).expect("a reference packet decodes");
        }
        assert_eq!(first, second);
        assert_eq!(first, decode_all(Band::Wide, true, packets));
    }

    #[test]
    fn a_packet_with_nothing_usable_in_it_is_refused_and_adds_nothing() {
        for band in [Band::Narrow, Band::Wide, Band::UltraWide] {
            let mut decoder = Decoder::new(band);
            let mut out = vec![0.25f32];
            assert_eq!(decoder.decode(&[], &mut out), Err(Corrupt));
            assert_eq!(decoder.decode(&[0x7F], &mut out), Err(Corrupt), "a packet that only says it is over");
            assert_eq!(decoder.decode(&[0x70, 0, 0, 0, 0, 0, 0, 0], &mut out), Err(Corrupt), "a request inside the stream");
            assert_eq!(decoder.decode(&[0x68, 0, 0, 0, 0, 0, 0, 0], &mut out), Err(Corrupt), "somebody's own data");
            assert_eq!(decoder.decode(&[0x48, 0, 0, 0, 0, 0, 0, 0], &mut out), Err(Corrupt), "a kind of frame that does not exist");
            assert_eq!(out, vec![0.25f32]);
        }
    }

    struct Noise(u32);

    impl Noise {
        fn next(&mut self) -> u32 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 17;
            self.0 ^= self.0 << 5;
            self.0
        }
    }

    #[test]
    fn rubbish_never_panics_never_hangs_and_never_gets_loud() {
        let mut noise = Noise(0x2545_F491);
        for band in [Band::Narrow, Band::Wide, Band::UltraWide] {
            let mut decoder = Decoder::new(band);
            for round in 0..3000 {
                let length = (noise.next() % 140) as usize;
                let packet: Vec<u8> = (0..length).map(|_| (noise.next() >> 11) as u8).collect();
                let mut out = Vec::new();
                match decoder.decode(&packet, &mut out) {
                    Ok(frames) => {
                        assert!((1..=MAX_FRAMES_PER_PACKET).contains(&frames), "round {round}: {frames} frames");
                        assert_eq!(out.len(), frames * band.frame_samples());
                    }
                    Err(Corrupt) => assert!(out.is_empty(), "round {round}: a refused packet added sound"),
                }
                assert!(out.iter().all(|s| s.is_finite() && s.abs() <= 1.0), "round {round}");
                if round % 7 == 0 {
                    decoder.conceal(&mut out);
                    assert!(out.iter().all(|s| s.is_finite() && s.abs() <= 1.0), "round {round}, filling in");
                }
            }
        }
    }

    #[test]
    fn a_real_stream_still_decodes_after_rubbish() {
        let (name, packets, want) = stream!("nb_q6");
        let mut noise = Noise(77);
        let mut decoder = Decoder::new(Band::Narrow);
        for _ in 0..200 {
            let packet: Vec<u8> = (0..38).map(|_| (noise.next() >> 9) as u8).collect();
            let _ = decoder.decode(&packet, &mut Vec::new());
        }
        decoder.reset();
        let mut out = Vec::new();
        for packet in records(packets).into_iter().flatten() {
            decoder.decode(packet, &mut out).expect("a reference packet decodes");
        }
        let worst = worst_difference(name, &out, &wanted(want));
        assert!(worst <= TOLERANCE, "{name}: off by up to {worst} of 32768");
    }

    #[test]
    fn every_cut_off_piece_of_a_real_packet_is_handled() {
        for (band, (_, packets, _)) in
            [(Band::Narrow, stream!("nb_q10")), (Band::Wide, stream!("wb_q10")), (Band::UltraWide, stream!("uwb_q10"))]
        {
            let whole = records(packets)[3].expect("a packet");
            let mut decoder = Decoder::new(band);
            for length in 0..=whole.len() {
                let mut out = Vec::new();
                let _ = decoder.decode(&whole[..length], &mut out);
                assert!(out.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
                assert!(out.len() <= MAX_FRAMES_PER_PACKET * band.frame_samples());
            }
        }
    }

    #[test]
    fn a_real_packet_with_bits_flipped_in_it_is_handled() {
        let mut noise = Noise(0x9E37_79B9);
        for (band, (_, packets, _)) in
            [(Band::Narrow, stream!("nb_q10")), (Band::Wide, stream!("wb_q10")), (Band::UltraWide, stream!("uwb_q10"))]
        {
            let whole: Vec<&[u8]> = records(packets).into_iter().flatten().collect();
            let mut decoder = Decoder::new(band);
            for round in 0..3000 {
                let mut packet = whole[round % whole.len()].to_vec();
                for _ in 0..1 + noise.next() % 8 {
                    let at = noise.next() as usize % packet.len().max(1);
                    if let Some(byte) = packet.get_mut(at) {
                        *byte ^= 1 << (noise.next() % 8);
                    }
                }
                let keep = noise.next() as usize % (packet.len() + 1);
                let mut out = Vec::new();
                match decoder.decode(&packet[..keep], &mut out) {
                    Ok(frames) => assert_eq!(out.len(), frames * band.frame_samples(), "round {round}"),
                    Err(Corrupt) => assert!(out.is_empty(), "round {round}"),
                }
                assert!(out.iter().all(|s| s.is_finite() && s.abs() <= 1.0), "round {round}");
                decoder.conceal(&mut out);
                assert!(out.iter().all(|s| s.is_finite() && s.abs() <= 1.0), "round {round}, filling in");
            }
        }
    }

    #[test]
    fn rubbish_repeated_to_look_like_several_frames_is_handled() {
        let mut noise = Noise(0x0BAD_F00D);
        for band in [Band::Narrow, Band::Wide, Band::UltraWide] {
            let mut decoder = Decoder::new(band);
            for _ in 0..2000 {
                let unit = 1 + (noise.next() % 60) as usize;
                let copies = 1 + (noise.next() % 12) as usize;
                let seed: Vec<u8> = (0..unit).map(|_| (noise.next() >> 13) as u8).collect();
                let packet: Vec<u8> = std::iter::repeat(seed).take(copies).flatten().collect();
                let mut out = Vec::new();
                let _ = decoder.decode(&packet, &mut out);
                assert!(out.len() <= MAX_FRAMES_PER_PACKET * band.frame_samples());
                assert!(out.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
            }
        }
    }

    #[test]
    #[ignore = "needs the full reference set, which is not kept in the repository"]
    fn the_full_reference_set_matches() {
        let folder = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("full").join("speex");
        let mut names: Vec<String> = std::fs::read_dir(&folder)
            .expect("the full reference set is in tests/full/speex")
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| entry.file_name().to_str().and_then(|name| name.strip_suffix(".pkt")).map(String::from))
            .collect();
        names.sort();
        assert!(names.len() >= 40, "only {} streams found", names.len());
        let mut worst_of_all = 0.0f32;
        for name in &names {
            let band = match name.split('_').next() {
                Some("nb") => Band::Narrow,
                Some("wb") => Band::Wide,
                _ => Band::UltraWide,
            };
            let stream = std::fs::read(folder.join(format!("{name}.pkt"))).expect("a stream");
            let reference = std::fs::read(folder.join(format!("{name}.f32"))).expect("its reference sound");
            let want: Vec<f32> =
                reference.chunks_exact(4).map(|four| f32::from_le_bytes([four[0], four[1], four[2], four[3]])).collect();
            let got = decode_all(band, true, &stream);
            assert_eq!(got.len(), want.len(), "{name}: number of samples");
            let mut worst = 0.0f32;
            for (g, w) in got.iter().zip(&want) {
                worst = worst.max((g * 32768.0 - w.clamp(-32768.0, 32767.0)).abs());
            }
            println!("{name}: off by up to {worst:.4} of 32768");
            assert!(worst <= FULL_SET_TOLERANCE, "{name}: off by up to {worst} of 32768");
            worst_of_all = worst_of_all.max(worst);
        }
        println!("worst of all: {worst_of_all:.4} of 32768 over {} streams", names.len());
    }

    #[test]
    #[ignore = "takes half a minute; run it by hand after changing the decoder"]
    fn a_long_run_of_broken_input_is_survived() {
        let mut noise = Noise(0x9E37_79B9);
        let mut below = move |limit: usize| (noise.next() as usize) % limit.max(1);
        let mut decoded = 0u64;
        let mut refused = 0u64;
        for (band, (_, stream, _)) in
            [(Band::Narrow, stream!("nb_q8")), (Band::Wide, stream!("wb_q10")), (Band::UltraWide, stream!("uwb_q10"))]
        {
            let real: Vec<&[u8]> = records(stream).into_iter().flatten().collect();
            let mut decoder = Decoder::new(band);
            for round in 0..400_000usize {
                let mut packet: Vec<u8> = match below(6) {
                    0 => (0..below(200)).map(|_| below(256) as u8).collect(),
                    1 => {
                        let mut packet = real[below(real.len())].to_vec();
                        for _ in 0..1 + below(6) {
                            let at = below(packet.len());
                            packet[at] ^= 1 << below(8);
                        }
                        packet
                    }
                    2 => {
                        let packet = real[below(real.len())];
                        packet[..below(packet.len() + 1)].to_vec()
                    }
                    3 => (0..1 + below(12)).flat_map(|_| real[below(real.len())].to_vec()).collect(),
                    4 => {
                        let mut packet = real[below(real.len())].to_vec();
                        let at = below(packet.len());
                        for _ in 0..below(40) {
                            packet.insert(at, below(256) as u8);
                        }
                        packet
                    }
                    _ => real[below(real.len())].to_vec(),
                };
                if below(50) == 0 {
                    packet.iter_mut().for_each(|byte| *byte = 0xFF);
                }
                let mut out = Vec::new();
                match decoder.decode(&packet, &mut out) {
                    Ok(frames) => {
                        decoded += 1;
                        assert!((1..=MAX_FRAMES_PER_PACKET).contains(&frames), "round {round}: {frames} frames");
                        assert_eq!(out.len(), frames * band.frame_samples(), "round {round}");
                    }
                    Err(Corrupt) => {
                        refused += 1;
                        assert!(out.is_empty(), "round {round}: a refused packet added sound");
                    }
                }
                assert!(out.iter().all(|s| s.is_finite() && s.abs() <= 1.0), "round {round}");
                match below(40) {
                    0 => {
                        let mut filler = Vec::new();
                        (0..1 + below(30)).for_each(|_| decoder.conceal(&mut filler));
                        assert!(filler.iter().all(|s| s.is_finite() && s.abs() <= 1.0), "round {round}, filling in");
                    }
                    1 => decoder.reset(),
                    2 => decoder.set_enhancement(below(2) == 0),
                    _ => {}
                }
            }
        }
        println!("{decoded} packets decoded, {refused} refused");
    }

    #[test]
    #[ignore = "a measurement, not a check"]
    fn how_fast_it_is() {
        for (name, band, (_, stream, _)) in [
            ("narrowband", Band::Narrow, stream!("nb_q8")),
            ("wideband", Band::Wide, stream!("wb_q10")),
            ("ultra-wideband", Band::UltraWide, stream!("uwb_q10")),
        ] {
            let packets: Vec<&[u8]> = records(stream).into_iter().flatten().collect();
            let mut decoder = Decoder::new(band);
            let mut out = Vec::new();
            let started = std::time::Instant::now();
            let mut frames = 0u64;
            for _ in 0..200 {
                for packet in &packets {
                    out.clear();
                    frames += decoder.decode(packet, &mut out).unwrap_or(0) as u64;
                }
            }
            let spent = started.elapsed().as_secs_f64();
            let sound = frames as f64 * 0.02;
            println!("{name}: {sound:.0} s of sound in {spent:.2} s, {:.2} % of one processor", 100.0 * spent / sound);
        }
    }
}
