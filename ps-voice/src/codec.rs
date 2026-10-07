use ps_oldcodecs::speex::Band;
use unsafe_libopus as opus;
use unsafe_libopus::varargs::{VarArg, VarArgs};

pub const SAMPLE_RATE: u32 = 48_000;
pub const FRAME_SAMPLES: usize = 960;
pub const MAX_FRAME_SAMPLES: usize = 5760;
pub const MAX_PACKET_BYTES: usize = 480;

pub const CODEC_SPEEX_NARROW: u8 = 0;
pub const CODEC_SPEEX_WIDE: u8 = 1;
pub const CODEC_SPEEX_ULTRA_WIDE: u8 = 2;
pub const CODEC_CELT_MONO: u8 = 3;
pub const CODEC_OPUS_VOICE: u8 = 4;
pub const CODEC_OPUS_MUSIC: u8 = 5;

pub fn speex_band(codec: u8) -> Option<Band> {
    match codec {
        CODEC_SPEEX_NARROW => Some(Band::Narrow),
        CODEC_SPEEX_WIDE => Some(Band::Wide),
        CODEC_SPEEX_ULTRA_WIDE => Some(Band::UltraWide),
        _ => None,
    }
}

pub fn is_supported_codec(codec: u8) -> bool {
    codec == CODEC_OPUS_VOICE || codec == CODEC_OPUS_MUSIC || speex_band(codec).is_some()
}

pub fn is_end_marker(data: &[u8]) -> bool {
    data.len() <= 1
}

pub fn bitrate_for(codec: u8, quality: u8) -> i32 {
    let q = quality.min(10) as i32;
    if codec == CODEC_OPUS_MUSIC {
        16_000 + 8_000 * q
    } else {
        6_000 + 4_000 * q
    }
}

fn error_text(code: i32) -> String {
    match code {
        opus::OPUS_BAD_ARG => "bad argument".into(),
        opus::OPUS_BUFFER_TOO_SMALL => "buffer too small".into(),
        opus::OPUS_INTERNAL_ERROR => "internal error".into(),
        opus::OPUS_INVALID_PACKET => "invalid packet".into(),
        opus::OPUS_UNIMPLEMENTED => "unimplemented".into(),
        opus::OPUS_INVALID_STATE => "invalid state".into(),
        opus::OPUS_ALLOC_FAIL => "allocation failed".into(),
        other => format!("opus error {other}"),
    }
}

pub struct Encoder {
    st: *mut opus::OpusEncoder,
    channels: usize,
}

unsafe impl Send for Encoder {}

impl Encoder {
    pub fn new(codec: u8, quality: u8) -> Result<Self, String> {
        let music = codec == CODEC_OPUS_MUSIC;
        let channels = if music { 2 } else { 1 };
        let application = if music { opus::OPUS_APPLICATION_AUDIO } else { opus::OPUS_APPLICATION_VOIP };
        let mut error = 0i32;
        let st = unsafe {
            opus::opus_encoder_create(SAMPLE_RATE as i32, channels as i32, application, &mut error)
        };
        if st.is_null() || error != opus::OPUS_OK {
            return Err(format!("cannot create opus encoder: {}", error_text(error)));
        }
        let mut enc = Self { st, channels };
        enc.set(opus::OPUS_SET_BITRATE_REQUEST, bitrate_for(codec, quality))?;
        enc.set(opus::OPUS_SET_VBR_REQUEST, 1)?;
        enc.set(opus::OPUS_SET_COMPLEXITY_REQUEST, 8)?;
        enc.set(opus::OPUS_SET_DTX_REQUEST, 0)?;
        if music {
            enc.set(opus::OPUS_SET_SIGNAL_REQUEST, opus::OPUS_SIGNAL_MUSIC)?;
        } else {
            enc.set(opus::OPUS_SET_SIGNAL_REQUEST, opus::OPUS_SIGNAL_VOICE)?;
            enc.set(opus::OPUS_SET_INBAND_FEC_REQUEST, 1)?;
            enc.set(opus::OPUS_SET_PACKET_LOSS_PERC_REQUEST, 10)?;
        }
        Ok(enc)
    }

    fn set(&mut self, request: i32, value: i32) -> Result<(), String> {
        let rc = unsafe {
            opus::opus_encoder_ctl_impl(self.st, request, VarArgs::new(vec![VarArg::I32(value)]))
        };
        if rc == opus::OPUS_OK {
            Ok(())
        } else {
            Err(format!("opus encoder ctl {request} failed: {}", error_text(rc)))
        }
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    pub fn reset(&mut self) {
        unsafe {
            opus::opus_encoder_ctl_impl(self.st, opus::OPUS_RESET_STATE, VarArgs::new(Vec::new()));
        }
    }

    pub fn encode(&mut self, pcm: &[f32], out: &mut [u8]) -> Result<usize, String> {
        if pcm.len() != FRAME_SAMPLES * self.channels {
            return Err(format!("expected {} samples, got {}", FRAME_SAMPLES * self.channels, pcm.len()));
        }
        let cap = out.len().min(MAX_PACKET_BYTES) as i32;
        let n = unsafe {
            opus::opus_encode_float(self.st, pcm.as_ptr(), FRAME_SAMPLES as i32, out.as_mut_ptr(), cap)
        };
        if n < 0 {
            Err(format!("opus encode failed: {}", error_text(n)))
        } else {
            Ok(n as usize)
        }
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        unsafe { opus::opus_encoder_destroy(self.st) }
    }
}

pub struct Decoder {
    st: *mut opus::OpusDecoder,
    channels: usize,
}

unsafe impl Send for Decoder {}

impl Decoder {
    pub fn new(channels: usize) -> Result<Self, String> {
        let mut error = 0i32;
        let st = unsafe { opus::opus_decoder_create(SAMPLE_RATE as i32, channels as i32, &mut error) };
        if st.is_null() || error != opus::OPUS_OK {
            return Err(format!("cannot create opus decoder: {}", error_text(error)));
        }
        Ok(Self { st, channels })
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    pub fn reset(&mut self) {
        unsafe {
            opus::opus_decoder_ctl_impl(self.st, opus::OPUS_RESET_STATE, VarArgs::new(Vec::new()));
        }
    }

    pub fn packet_samples(data: &[u8]) -> Option<usize> {
        if data.is_empty() {
            return None;
        }
        let n = unsafe {
            opus::opus_packet_get_nb_samples(data.as_ptr(), data.len() as i32, SAMPLE_RATE as i32)
        };
        if n > 0 && n as usize <= MAX_FRAME_SAMPLES {
            Some(n as usize)
        } else {
            None
        }
    }

    pub fn decode(&mut self, data: &[u8], fec: bool, out: &mut [f32]) -> Result<usize, String> {
        let frame = out.len() / self.channels;
        let n = unsafe {
            opus::opus_decode_float(
                self.st,
                data.as_ptr(),
                data.len() as i32,
                out.as_mut_ptr(),
                frame as i32,
                i32::from(fec),
            )
        };
        if n < 0 {
            Err(format!("opus decode failed: {}", error_text(n)))
        } else {
            Ok(n as usize)
        }
    }

    pub fn conceal(&mut self, out: &mut [f32]) -> Result<usize, String> {
        let frame = out.len() / self.channels;
        let n = unsafe {
            opus::opus_decode_float(self.st, std::ptr::null(), 0, out.as_mut_ptr(), frame as i32, 0)
        };
        if n < 0 {
            Err(format!("opus concealment failed: {}", error_text(n)))
        } else {
            Ok(n as usize)
        }
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        unsafe { opus::opus_decoder_destroy(self.st) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(freq: f32, frames: usize, channels: usize, offset: usize) -> Vec<f32> {
        let mut out = Vec::with_capacity(frames * channels);
        for i in 0..frames {
            let t = (offset + i) as f32 / SAMPLE_RATE as f32;
            let s = 0.4 * (2.0 * std::f32::consts::PI * freq * t).sin();
            for _ in 0..channels {
                out.push(s);
            }
        }
        out
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
    }

    fn dominant_frequency(mono: &[f32]) -> f32 {
        let mut crossings = 0usize;
        for w in mono.windows(2) {
            if w[0] <= 0.0 && w[1] > 0.0 {
                crossings += 1;
            }
        }
        crossings as f32 * SAMPLE_RATE as f32 / mono.len() as f32
    }

    #[test]
    fn bitrates_scale_with_quality() {
        assert_eq!(bitrate_for(CODEC_OPUS_VOICE, 0), 6_000);
        assert_eq!(bitrate_for(CODEC_OPUS_VOICE, 6), 30_000);
        assert_eq!(bitrate_for(CODEC_OPUS_VOICE, 10), 46_000);
        assert_eq!(bitrate_for(CODEC_OPUS_VOICE, 200), 46_000);
        assert_eq!(bitrate_for(CODEC_OPUS_MUSIC, 10), 96_000);
        assert!(is_supported_codec(4) && is_supported_codec(5) && !is_supported_codec(3));
        assert!(is_supported_codec(0) && is_supported_codec(1) && is_supported_codec(2) && !is_supported_codec(6));
        assert_eq!(speex_band(CODEC_SPEEX_WIDE).map(|band| band.sample_rate()), Some(16_000));
        assert_eq!(speex_band(CODEC_CELT_MONO), None);
        assert!(is_end_marker(&[]) && is_end_marker(&[0x78]) && !is_end_marker(&[0x78, 0]));
    }

    #[test]
    fn voice_round_trip_preserves_tone() {
        let mut enc = Encoder::new(CODEC_OPUS_VOICE, 6).unwrap();
        assert_eq!(enc.channels(), 1);
        let mut dec = Decoder::new(2).unwrap();
        let mut packet = [0u8; MAX_PACKET_BYTES];
        let mut decoded = Vec::new();
        let mut sizes = Vec::new();
        for f in 0..50 {
            let pcm = tone(440.0, FRAME_SAMPLES, 1, f * FRAME_SAMPLES);
            let n = enc.encode(&pcm, &mut packet).unwrap();
            assert!(n > 2 && n <= MAX_PACKET_BYTES);
            sizes.push(n);
            assert_eq!(Decoder::packet_samples(&packet[..n]), Some(FRAME_SAMPLES));
            let mut out = vec![0f32; FRAME_SAMPLES * 2];
            let got = dec.decode(&packet[..n], false, &mut out).unwrap();
            assert_eq!(got, FRAME_SAMPLES);
            decoded.extend_from_slice(&out);
        }
        let avg = sizes.iter().sum::<usize>() as f32 / sizes.len() as f32;
        assert!(avg < 200.0, "average packet {avg} bytes");
        let left: Vec<f32> = decoded.chunks(2).skip(FRAME_SAMPLES * 10).map(|c| c[0]).collect();
        let right: Vec<f32> = decoded.chunks(2).skip(FRAME_SAMPLES * 10).map(|c| c[1]).collect();
        let level = rms(&left);
        assert!((level - 0.2828).abs() < 0.06, "rms {level}");
        assert!((rms(&right) - level).abs() < 0.01);
        let freq = dominant_frequency(&left);
        assert!((freq - 440.0).abs() < 8.0, "frequency {freq}");
    }

    #[test]
    fn music_round_trip_is_stereo() {
        let mut enc = Encoder::new(CODEC_OPUS_MUSIC, 7).unwrap();
        assert_eq!(enc.channels(), 2);
        let mut dec = Decoder::new(2).unwrap();
        let mut packet = [0u8; MAX_PACKET_BYTES];
        let mut decoded = Vec::new();
        for f in 0..40 {
            let mut pcm = tone(1000.0, FRAME_SAMPLES, 2, f * FRAME_SAMPLES);
            for frame in pcm.chunks_mut(2) {
                frame[1] = 0.0;
            }
            let n = enc.encode(&pcm, &mut packet).unwrap();
            let mut out = vec![0f32; FRAME_SAMPLES * 2];
            assert_eq!(dec.decode(&packet[..n], false, &mut out).unwrap(), FRAME_SAMPLES);
            decoded.extend_from_slice(&out);
        }
        let left: Vec<f32> = decoded.chunks(2).skip(FRAME_SAMPLES * 10).map(|c| c[0]).collect();
        let right: Vec<f32> = decoded.chunks(2).skip(FRAME_SAMPLES * 10).map(|c| c[1]).collect();
        assert!(rms(&left) > 0.2, "left {}", rms(&left));
        assert!(rms(&right) < 0.05, "right {}", rms(&right));
        assert!((dominant_frequency(&left) - 1000.0).abs() < 15.0);
    }

    #[test]
    fn concealment_and_errors() {
        let mut enc = Encoder::new(CODEC_OPUS_VOICE, 6).unwrap();
        let mut dec = Decoder::new(2).unwrap();
        let mut packet = [0u8; MAX_PACKET_BYTES];
        let mut out = vec![0f32; FRAME_SAMPLES * 2];
        for f in 0..10 {
            let pcm = tone(300.0, FRAME_SAMPLES, 1, f * FRAME_SAMPLES);
            let n = enc.encode(&pcm, &mut packet).unwrap();
            dec.decode(&packet[..n], false, &mut out).unwrap();
        }
        assert_eq!(dec.conceal(&mut out).unwrap(), FRAME_SAMPLES);
        assert!(rms(&out) > 0.01, "concealed audio should continue the tone");
        assert!(dec.decode(&[0xff, 0xff, 0xff], false, &mut out).is_err());
        assert_eq!(Decoder::packet_samples(&[]), None);
        assert!(enc.encode(&[0.0; 100], &mut packet).is_err());
        enc.reset();
        dec.reset();
        let pcm = tone(300.0, FRAME_SAMPLES, 1, 0);
        let n = enc.encode(&pcm, &mut packet).unwrap();
        assert_eq!(dec.decode(&packet[..n], false, &mut out).unwrap(), FRAME_SAMPLES);
    }
}
