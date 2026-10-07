pub const CODEC_SPEEX_NARROWBAND: u8 = 0;
pub const CODEC_SPEEX_WIDEBAND: u8 = 1;
pub const CODEC_SPEEX_ULTRAWIDEBAND: u8 = 2;
pub const CODEC_CELT_MONO: u8 = 3;
pub const CODEC_OPUS_VOICE: u8 = 4;
pub const CODEC_OPUS_MUSIC: u8 = 5;

pub const C2S_VOICE_HEADER_LEN: usize = 3;
pub const S2C_VOICE_HEADER_LEN: usize = 5;

pub fn is_end_of_stream(data: &[u8]) -> bool {
    data.len() <= 1
}

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

pub const GROUP_WHISPER_HEADER_LEN: usize = 13;

pub fn whisper_header_len(channels: usize, clients: usize) -> usize {
    5 + channels * 8 + clients * 2
}

pub fn encode_c2s_whisper(codec: u8, channels: &[u64], clients: &[u16], data: &[u8]) -> Option<Vec<u8>> {
    if channels.len() > 255 || clients.len() > 255 {
        return None;
    }
    let mut out = Vec::with_capacity(whisper_header_len(channels.len(), clients.len()) + data.len());
    out.extend_from_slice(&[0, 0, codec, channels.len() as u8, clients.len() as u8]);
    for channel in channels {
        out.extend_from_slice(&channel.to_be_bytes());
    }
    for client in clients {
        out.extend_from_slice(&client.to_be_bytes());
    }
    out.extend_from_slice(data);
    Some(out)
}

pub fn encode_c2s_group_whisper(codec: u8, who: u8, scope: u8, id: u64, data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(GROUP_WHISPER_HEADER_LEN + data.len());
    out.extend_from_slice(&[0, 0, codec, who, scope]);
    out.extend_from_slice(&id.to_be_bytes());
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
        assert!(is_end_of_stream(v.data) && is_end_of_stream(&[0x78]) && !is_end_of_stream(&[0x78, 1]));
    }

    #[test]
    fn whisper_to_a_list_matches_the_wire_layout() {
        let raw = encode_c2s_whisper(CODEC_OPUS_VOICE, &[1, 0x0102030405060708], &[9, 0x0A0B], &[0xEE, 0xFF]).unwrap();
        assert_eq!(
            raw,
            vec![0, 0, 4, 2, 2, 0, 0, 0, 0, 0, 0, 0, 1, 1, 2, 3, 4, 5, 6, 7, 8, 0, 9, 0x0A, 0x0B, 0xEE, 0xFF]
        );
        assert_eq!(whisper_header_len(2, 2), 25);
        assert_eq!(whisper_header_len(30, 60), 365);
        assert_eq!(encode_c2s_whisper(CODEC_OPUS_VOICE, &[7], &[], &[]).unwrap().len(), 13);
        assert_eq!(encode_c2s_whisper(CODEC_OPUS_VOICE, &[], &[], &[5]).unwrap(), vec![0, 0, 4, 0, 0, 5]);
        assert!(encode_c2s_whisper(4, &vec![1; 256], &[], &[]).is_none());
        assert!(encode_c2s_whisper(4, &[], &vec![1; 256], &[]).is_none());
    }

    #[test]
    fn whisper_to_a_group_matches_the_wire_layout() {
        assert_eq!(
            encode_c2s_group_whisper(CODEC_OPUS_VOICE, 2, 5, 0, &[0xEE]),
            vec![0, 0, 4, 2, 5, 0, 0, 0, 0, 0, 0, 0, 0, 0xEE]
        );
        assert_eq!(
            encode_c2s_group_whisper(CODEC_OPUS_MUSIC, 0, 0, 0x0102030405060708, &[]),
            vec![0, 0, 5, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8]
        );
        assert_eq!(GROUP_WHISPER_HEADER_LEN, 13);
    }

    #[test]
    fn short_payloads_are_rejected() {
        assert!(parse_c2s_voice(&[0, 1]).is_none());
        assert!(parse_s2c_voice(&[0, 1, 2, 3]).is_none());
        assert!(is_opus(CODEC_OPUS_VOICE) && is_opus(CODEC_OPUS_MUSIC));
        assert!(!is_opus(CODEC_CELT_MONO));
    }
}
