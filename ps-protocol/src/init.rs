use crate::ProtocolError;

pub const INIT_VERSION: u32 = 1_566_914_096;

pub const INIT0_LEN: usize = 21;
pub const INIT1_LEN: usize = 21;
pub const INIT2_LEN: usize = 25;
pub const INIT3_LEN: usize = 233;
pub const INIT4_FIXED_LEN: usize = 301;

pub fn init0(version: u32, timestamp: u32, random: [u8; 4]) -> Vec<u8> {
    let mut out = Vec::with_capacity(INIT0_LEN);
    out.extend_from_slice(&version.to_be_bytes());
    out.push(0x00);
    out.extend_from_slice(&timestamp.to_be_bytes());
    out.extend_from_slice(&random);
    out.extend_from_slice(&[0u8; 8]);
    out
}

pub fn init2(version: u32, server_cookie: &[u8; 16], echo: &[u8; 4]) -> Vec<u8> {
    let mut out = Vec::with_capacity(INIT2_LEN);
    out.extend_from_slice(&version.to_be_bytes());
    out.push(0x02);
    out.extend_from_slice(server_cookie);
    out.extend_from_slice(echo);
    out
}

pub fn init4(version: u32, puzzle: &Puzzle, solution: &[u8; 64], clientinitiv: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(INIT4_FIXED_LEN + clientinitiv.len());
    out.extend_from_slice(&version.to_be_bytes());
    out.push(0x04);
    out.extend_from_slice(&puzzle.x);
    out.extend_from_slice(&puzzle.n);
    out.extend_from_slice(&puzzle.level.to_be_bytes());
    out.extend_from_slice(&puzzle.server_data);
    out.extend_from_slice(solution);
    out.extend_from_slice(clientinitiv.as_bytes());
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Puzzle {
    pub x: [u8; 64],
    pub n: [u8; 64],
    pub level: u32,
    pub server_data: [u8; 100],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerInit {
    Step1 { server_cookie: [u8; 16], echo: [u8; 4] },
    Step3(Box<Puzzle>),
    Restart,
    Error(u32),
}

pub fn parse_server_init(data: &[u8]) -> Result<ServerInit, ProtocolError> {
    let step = *data.first().ok_or(ProtocolError::TooShort("init packet"))?;
    match step {
        1 => match data.len() {
            INIT1_LEN => {
                let mut server_cookie = [0u8; 16];
                server_cookie.copy_from_slice(&data[1..17]);
                let mut echo = [0u8; 4];
                echo.copy_from_slice(&data[17..21]);
                Ok(ServerInit::Step1 { server_cookie, echo })
            }
            5 => Ok(ServerInit::Error(u32::from_be_bytes([data[1], data[2], data[3], data[4]]))),
            other => Err(ProtocolError::InvalidInit(format!("step 1 has {other} bytes"))),
        },
        3 => {
            if data.len() != INIT3_LEN {
                return Err(ProtocolError::InvalidInit(format!("step 3 has {} bytes", data.len())));
            }
            let mut x = [0u8; 64];
            x.copy_from_slice(&data[1..65]);
            let mut n = [0u8; 64];
            n.copy_from_slice(&data[65..129]);
            let level = u32::from_be_bytes([data[129], data[130], data[131], data[132]]);
            let mut server_data = [0u8; 100];
            server_data.copy_from_slice(&data[133..233]);
            Ok(ServerInit::Step3(Box::new(Puzzle { x, n, level, server_data })))
        }
        0x7f => Ok(ServerInit::Restart),
        other => Err(ProtocolError::InvalidInit(format!("unexpected step {other}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init0_layout() {
        let p = init0(INIT_VERSION, 0x6543_2100, [1, 2, 3, 4]);
        assert_eq!(p.len(), INIT0_LEN);
        assert_eq!(&p[..4], &[0x5d, 0x65, 0x36, 0x30]);
        assert_eq!(p[4], 0);
        assert_eq!(&p[5..9], &[0x65, 0x43, 0x21, 0x00]);
        assert_eq!(&p[9..13], &[1, 2, 3, 4]);
        assert_eq!(&p[13..], &[0u8; 8]);
    }

    #[test]
    fn step1_round_trip_into_init2() {
        let mut raw = vec![1u8];
        raw.extend_from_slice(&[0xaa; 16]);
        raw.extend_from_slice(&[4, 3, 2, 1]);
        let ServerInit::Step1 { server_cookie, echo } = parse_server_init(&raw).unwrap() else {
            panic!("expected step 1");
        };
        assert_eq!(server_cookie, [0xaa; 16]);
        assert_eq!(echo, [4, 3, 2, 1]);
        let p = init2(INIT_VERSION, &server_cookie, &echo);
        assert_eq!(p.len(), INIT2_LEN);
        assert_eq!(p[4], 2);
        assert_eq!(&p[5..21], &[0xaa; 16]);
        assert_eq!(&p[21..], &[4, 3, 2, 1]);
    }

    #[test]
    fn step3_round_trip_into_init4() {
        let mut raw = vec![3u8];
        raw.extend_from_slice(&[0x11; 64]);
        raw.extend_from_slice(&[0x22; 64]);
        raw.extend_from_slice(&10_000u32.to_be_bytes());
        raw.extend_from_slice(&[0x33; 100]);
        assert_eq!(raw.len(), INIT3_LEN);
        let ServerInit::Step3(puzzle) = parse_server_init(&raw).unwrap() else {
            panic!("expected step 3");
        };
        assert_eq!(puzzle.level, 10_000);
        assert_eq!(puzzle.x, [0x11; 64]);
        assert_eq!(puzzle.n, [0x22; 64]);
        assert_eq!(puzzle.server_data, [0x33; 100]);

        let text = "clientinitiv alpha=AAAA omega=BBBB ot=1 ip";
        let p = init4(INIT_VERSION, &puzzle, &[0x44; 64], text);
        assert_eq!(p.len(), INIT4_FIXED_LEN + text.len());
        assert_eq!(p[4], 4);
        assert_eq!(&p[5..237], &raw[1..233]);
        assert_eq!(&p[237..301], &[0x44; 64]);
        assert_eq!(&p[301..], text.as_bytes());
    }

    #[test]
    fn special_steps_and_errors() {
        assert_eq!(parse_server_init(&[0x7f]).unwrap(), ServerInit::Restart);
        assert_eq!(parse_server_init(&[1, 0, 0, 0x02, 0x09]).unwrap(), ServerInit::Error(0x209));
        assert!(parse_server_init(&[]).is_err());
        assert!(parse_server_init(&[1, 2, 3]).is_err());
        assert!(parse_server_init(&[3; 100]).is_err());
        assert!(parse_server_init(&[2; 25]).is_err());
    }
}
