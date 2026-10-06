pub const MAC_LEN: usize = 8;
pub const C2S_HEADER_LEN: usize = 13;
pub const S2C_HEADER_LEN: usize = 11;
pub const MAX_PACKET_LEN: usize = 500;
pub const MAX_C2S_PAYLOAD: usize = MAX_PACKET_LEN - C2S_HEADER_LEN;
pub const INIT_PACKET_ID: u16 = 101;
pub const PACKET_TYPE_COUNT: usize = 9;

pub const FLAG_UNENCRYPTED: u8 = 0x80;
pub const FLAG_COMPRESSED: u8 = 0x40;
pub const FLAG_NEWPROTOCOL: u8 = 0x20;
pub const FLAG_FRAGMENTED: u8 = 0x10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum PacketType {
    Voice = 0,
    VoiceWhisper = 1,
    Command = 2,
    CommandLow = 3,
    Ping = 4,
    Pong = 5,
    Ack = 6,
    AckLow = 7,
    Init1 = 8,
}

impl PacketType {
    pub fn from_u8(v: u8) -> Option<Self> {
        Some(match v & 0x0f {
            0 => PacketType::Voice,
            1 => PacketType::VoiceWhisper,
            2 => PacketType::Command,
            3 => PacketType::CommandLow,
            4 => PacketType::Ping,
            5 => PacketType::Pong,
            6 => PacketType::Ack,
            7 => PacketType::AckLow,
            8 => PacketType::Init1,
            _ => return None,
        })
    }

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn is_voice(self) -> bool {
        matches!(self, PacketType::Voice | PacketType::VoiceWhisper)
    }

    pub fn is_command(self) -> bool {
        matches!(self, PacketType::Command | PacketType::CommandLow)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    C2S,
    S2C,
}

impl Direction {
    pub fn header_len(self) -> usize {
        match self {
            Direction::C2S => C2S_HEADER_LEN,
            Direction::S2C => S2C_HEADER_LEN,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub mac: [u8; MAC_LEN],
    pub packet_id: u16,
    pub client_id: Option<u16>,
    pub type_flags: u8,
}

impl Header {
    pub fn c2s(packet_type: PacketType, flags: u8, packet_id: u16, client_id: u16) -> Self {
        Self {
            mac: [0; MAC_LEN],
            packet_id,
            client_id: Some(client_id),
            type_flags: (flags & 0xf0) | packet_type as u8,
        }
    }

    pub fn s2c(packet_type: PacketType, flags: u8, packet_id: u16) -> Self {
        Self {
            mac: [0; MAC_LEN],
            packet_id,
            client_id: None,
            type_flags: (flags & 0xf0) | packet_type as u8,
        }
    }

    pub fn direction(&self) -> Direction {
        if self.client_id.is_some() {
            Direction::C2S
        } else {
            Direction::S2C
        }
    }

    pub fn packet_type(&self) -> Option<PacketType> {
        PacketType::from_u8(self.type_flags)
    }

    pub fn flags(&self) -> u8 {
        self.type_flags & 0xf0
    }

    pub fn has(&self, flag: u8) -> bool {
        self.type_flags & flag != 0
    }

    pub fn parse(direction: Direction, raw: &[u8]) -> Option<(Header, &[u8])> {
        let len = direction.header_len();
        if raw.len() < len {
            return None;
        }
        let mut mac = [0u8; MAC_LEN];
        mac.copy_from_slice(&raw[..MAC_LEN]);
        let packet_id = u16::from_be_bytes([raw[8], raw[9]]);
        let (client_id, type_flags) = match direction {
            Direction::C2S => (Some(u16::from_be_bytes([raw[10], raw[11]])), raw[12]),
            Direction::S2C => (None, raw[10]),
        };
        Some((Header { mac, packet_id, client_id, type_flags }, &raw[len..]))
    }

    pub fn meta(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(5);
        out.extend_from_slice(&self.packet_id.to_be_bytes());
        if let Some(cid) = self.client_id {
            out.extend_from_slice(&cid.to_be_bytes());
        }
        out.push(self.type_flags);
        out
    }

    pub fn build(&self, payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.direction().header_len() + payload.len());
        out.extend_from_slice(&self.mac);
        out.extend_from_slice(&self.meta());
        out.extend_from_slice(payload);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn c2s_round_trip() {
        let mut h = Header::c2s(PacketType::Command, FLAG_NEWPROTOCOL | FLAG_FRAGMENTED, 0x1234, 7);
        h.mac = [1, 2, 3, 4, 5, 6, 7, 8];
        let raw = h.build(b"hello");
        assert_eq!(raw.len(), C2S_HEADER_LEN + 5);
        assert_eq!(&raw[..8], &[1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(&raw[8..13], &[0x12, 0x34, 0x00, 0x07, 0x32]);
        let (parsed, payload) = Header::parse(Direction::C2S, &raw).unwrap();
        assert_eq!(parsed, h);
        assert_eq!(payload, b"hello");
        assert_eq!(parsed.packet_type(), Some(PacketType::Command));
        assert!(parsed.has(FLAG_NEWPROTOCOL));
        assert!(parsed.has(FLAG_FRAGMENTED));
        assert!(!parsed.has(FLAG_COMPRESSED));
        assert_eq!(parsed.meta(), vec![0x12, 0x34, 0x00, 0x07, 0x32]);
    }

    #[test]
    fn s2c_round_trip() {
        let mut h = Header::s2c(PacketType::Voice, FLAG_UNENCRYPTED, 65535);
        h.mac = *b"ABCDEFGH";
        let raw = h.build(&[9, 9]);
        assert_eq!(raw.len(), S2C_HEADER_LEN + 2);
        assert_eq!(&raw[8..11], &[0xff, 0xff, 0x80]);
        let (parsed, payload) = Header::parse(Direction::S2C, &raw).unwrap();
        assert_eq!(parsed, h);
        assert_eq!(payload, &[9, 9]);
        assert_eq!(parsed.meta(), vec![0xff, 0xff, 0x80]);
    }

    #[test]
    fn matches_tsproto_ack_layout() {
        let mut h = Header::c2s(PacketType::Ack, 0, 0, 0);
        h.mac = [0xa4, 0x7b, 0x47, 0x94, 0xdb, 0xa9, 0x6a, 0xc5];
        let raw = h.build(&[0xfe, 0x18]);
        assert_eq!(
            raw,
            vec![0xa4, 0x7b, 0x47, 0x94, 0xdb, 0xa9, 0x6a, 0xc5, 0, 0, 0, 0, 0x6, 0xfe, 0x18]
        );
    }

    #[test]
    fn rejects_short_and_unknown() {
        assert!(Header::parse(Direction::S2C, &[0u8; 10]).is_none());
        assert!(Header::parse(Direction::C2S, &[0u8; 12]).is_none());
        let mut raw = [0u8; 11];
        raw[10] = 0x0f;
        let (h, _) = Header::parse(Direction::S2C, &raw).unwrap();
        assert_eq!(h.packet_type(), None);
        for v in 0..=8u8 {
            assert_eq!(PacketType::from_u8(v).unwrap() as u8, v);
        }
    }
}
