pub const CODEC_SPEEX_NARROWBAND: u8 = 0;
pub const CODEC_SPEEX_WIDEBAND: u8 = 1;
pub const CODEC_SPEEX_ULTRAWIDEBAND: u8 = 2;
pub const CODEC_CELT_MONO: u8 = 3;
pub const CODEC_OPUS_VOICE: u8 = 4;
pub const CODEC_OPUS_MUSIC: u8 = 5;

pub const C2S_VOICE_HEADER_LEN: usize = 3;
pub const S2C_VOICE_HEADER_LEN: usize = 5;

pub fn is_opus(codec: u8) -> bool {
    codec == CODEC_OPUS_VOICE || codec == CODEC_OPUS_MUSIC
}

pub fn encode_c2s_voice(voice_id: u16, codec: u8, data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(C2S_VOICE_HEADER_LEN + data.len());
    out.extend_from_slice(&voice_id.to_be_bytes());
    out.push(codec);
    out.extend_from_slice(data);
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct C2SVoice<'a> {
    pub voice_id: u16,
    pub codec: u8,
    pub data: &'a [u8],
}

pub fn parse_c2s_voice(payload: &[u8]) -> Option<C2SVoice<'_>> {
    if payload.len() < C2S_VOICE_HEADER_LEN {
        return None;
    }
    Some(C2SVoice {
        voice_id: u16::from_be_bytes([payload[0], payload[1]]),
        codec: payload[2],
        data: &payload[C2S_VOICE_HEADER_LEN..],
    })
}

pub fn encode_s2c_voice(voice_id: u16, client_id: u16, codec: u8, data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(S2C_VOICE_HEADER_LEN + data.len());
    out.extend_from_slice(&voice_id.to_be_bytes());
    out.extend_from_slice(&client_id.to_be_bytes());
    out.push(codec);
    out.extend_from_slice(data);
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct S2CVoice<'a> {
    pub voice_id: u16,
    pub client_id: u16,
    pub codec: u8,
    pub data: &'a [u8],
}

pub fn parse_s2c_voice(payload: &[u8]) -> Option<S2CVoice<'_>> {
    if payload.len() < S2C_VOICE_HEADER_LEN {
        return None;
    }
    Some(S2CVoice {
        voice_id: u16::from_be_bytes([payload[0], payload[1]]),
        client_id: u16::from_be_bytes([payload[2], payload[3]]),
        codec: payload[4],
        data: &payload[S2C_VOICE_HEADER_LEN..],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn c2s_round_trip() {
        let raw = encode_c2s_voice(0x1234, CODEC_OPUS_VOICE, &[1, 2, 3]);
        assert_eq!(raw, vec![0x12, 0x34, 4, 1, 2, 3]);
        let v = parse_c2s_voice(&raw).unwrap();
        assert_eq!(v, C2SVoice { voice_id: 0x1234, codec: 4, data: &[1, 2, 3] });
    }

    #[test]
    fn s2c_round_trip() {
        let raw = encode_s2c_voice(0x1234, 0x5678, CODEC_OPUS_MUSIC, &[1, 2, 3]);
        assert_eq!(raw, vec![0x12, 0x34, 0x56, 0x78, 5, 1, 2, 3]);
        let v = parse_s2c_voice(&raw).unwrap();
        assert_eq!(v, S2CVoice { voice_id: 0x1234, client_id: 0x5678, codec: 5, data: &[1, 2, 3] });
    }

    #[test]
    fn end_of_stream_marker_has_no_data() {
        let raw = encode_c2s_voice(7, CODEC_OPUS_VOICE, &[]);
        assert_eq!(raw.len(), C2S_VOICE_HEADER_LEN);
        assert!(parse_c2s_voice(&raw).unwrap().data.is_empty());
        let v = parse_s2c_voice(&[0, 7, 0, 9, 4]).unwrap();
        assert!(v.data.is_empty());
        assert_eq!(v.client_id, 9);
    }

    #[test]
    fn short_payloads_are_rejected() {
        assert!(parse_c2s_voice(&[0, 1]).is_none());
        assert!(parse_s2c_voice(&[0, 1, 2, 3]).is_none());
        assert!(is_opus(CODEC_OPUS_VOICE) && is_opus(CODEC_OPUS_MUSIC));
        assert!(!is_opus(CODEC_CELT_MONO));
    }
}
