use curve25519_dalek::edwards::CompressedEdwardsY;
use sha2::{Digest as _, Sha512};

use crate::CryptoError;

pub const ROOT_KEY: [u8; 32] = [
    0xcd, 0x0d, 0xe2, 0xae, 0xd4, 0x63, 0x45, 0x50, 0x9a, 0x7e, 0x3c, 0xfd, 0x8f, 0x68, 0xb3, 0xdc,
    0x75, 0x55, 0xb2, 0x9d, 0xcc, 0xec, 0x73, 0xcd, 0x18, 0x75, 0x0f, 0x99, 0x38, 0x12, 0x40, 0x8a,
];

pub const TIMESTAMP_OFFSET: u64 = 0x50e2_2700;

const BLOCK_MIN_LEN: usize = 42;
const MAX_BLOCKS: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockType {
    Intermediate,
    Website,
    Server,
    Code,
    Ts5Server,
    Ephemeral,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LicenseBlock {
    pub block_type: BlockType,
    pub public_key: [u8; 32],
    pub not_valid_before: u64,
    pub not_valid_after: u64,
    pub issuer: Option<String>,
    pub hash: [u8; 32],
    pub len: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct License {
    pub blocks: Vec<LicenseBlock>,
}

fn err(msg: impl Into<String>) -> CryptoError {
    CryptoError::License(msg.into())
}

fn nul_string(data: &[u8]) -> Result<(String, usize), CryptoError> {
    let end = data
        .iter()
        .position(|b| *b == 0)
        .ok_or_else(|| err("non-null-terminated issuer string"))?;
    Ok((String::from_utf8_lossy(&data[..end]).into_owned(), end))
}

impl LicenseBlock {
    pub fn parse(data: &[u8]) -> Result<Self, CryptoError> {
        if data.len() < BLOCK_MIN_LEN {
            return Err(err(format!("block too short ({} bytes)", data.len())));
        }
        if data[0] != 0 {
            return Err(err(format!("wrong key kind {}", data[0])));
        }
        let content = &data[BLOCK_MIN_LEN..];
        let (block_type, issuer, extra) = match data[33] {
            0 => {
                if content.len() < 5 {
                    return Err(err("intermediate block too short"));
                }
                let (issuer, n) = nul_string(&content[4..])?;
                (BlockType::Intermediate, Some(issuer), 5 + n)
            }
            t @ (1 | 3) => {
                let (issuer, n) = nul_string(content)?;
                let kind = if t == 1 { BlockType::Website } else { BlockType::Code };
                (kind, Some(issuer), 1 + n)
            }
            2 => {
                if content.len() < 6 {
                    return Err(err("server block too short"));
                }
                let (issuer, n) = nul_string(&content[5..])?;
                (BlockType::Server, Some(issuer), 6 + n)
            }
            8 => {
                if content.len() < 2 {
                    return Err(err("ts5 server block too short"));
                }
                let count = content[1] as usize;
                let mut pos = 2usize;
                let mut issuer = None;
                for _ in 0..count {
                    let plen = *content
                        .get(pos)
                        .ok_or_else(|| err("missing ts5 license property"))? as usize;
                    pos += 1;
                    let prop = content
                        .get(pos..pos + plen)
                        .ok_or_else(|| err("cut off ts5 license property"))?;
                    if prop.len() >= 2 && prop[0] == 2 && prop[1] == 0 {
                        let text = &prop[2..];
                        let end = text.iter().position(|b| *b == 0).unwrap_or(text.len());
                        issuer = Some(String::from_utf8_lossy(&text[..end]).into_owned());
                    }
                    pos += plen;
                }
                (BlockType::Ts5Server, issuer, pos)
            }
            32 => (BlockType::Ephemeral, None, 0),
            other => return Err(err(format!("unknown block type {other}"))),
        };
        let len = BLOCK_MIN_LEN + extra;
        if data.len() < len {
            return Err(err("block exceeds license data"));
        }
        let not_valid_before =
            u32::from_be_bytes([data[34], data[35], data[36], data[37]]) as u64 + TIMESTAMP_OFFSET;
        let not_valid_after =
            u32::from_be_bytes([data[38], data[39], data[40], data[41]]) as u64 + TIMESTAMP_OFFSET;
        if not_valid_after < not_valid_before {
            return Err(err("license times are invalid"));
        }
        let mut public_key = [0u8; 32];
        public_key.copy_from_slice(&data[1..33]);
        let mut hash = [0u8; 32];
        hash.copy_from_slice(&Sha512::digest(&data[1..len])[..32]);
        Ok(Self { block_type, public_key, not_valid_before, not_valid_after, issuer, hash, len })
    }
}

impl License {
    pub fn parse(data: &[u8]) -> Result<Self, CryptoError> {
        let version = *data.first().ok_or_else(|| err("empty license"))?;
        if version != 0 && version != 1 {
            return Err(err(format!("unsupported version {version}")));
        }
        let mut blocks = Vec::new();
        let mut rest = &data[1..];
        while !rest.is_empty() {
            if blocks.len() >= MAX_BLOCKS {
                return Err(err("too many license blocks"));
            }
            let block = LicenseBlock::parse(rest)?;
            rest = &rest[block.len..];
            blocks.push(block);
        }
        if blocks.is_empty() {
            return Err(err("license contains no blocks"));
        }
        Ok(Self { blocks })
    }

    pub fn derive_public_key(&self) -> Result<[u8; 32], CryptoError> {
        let mut key = CompressedEdwardsY(ROOT_KEY)
            .decompress()
            .ok_or_else(|| err("invalid root key"))?;
        for block in &self.blocks {
            let point = CompressedEdwardsY(block.public_key)
                .decompress()
                .ok_or_else(|| err("cannot decompress block public key"))?;
            key = point.mul_clamped(block.hash) + key;
        }
        Ok(key.compress().to_bytes())
    }

    pub fn validity_problem(&self, now_unix: u64) -> Option<String> {
        for (i, b) in self.blocks.iter().enumerate() {
            if now_unix < b.not_valid_before {
                return Some(format!("license block {i} is not valid yet"));
            }
            if now_unix > b.not_valid_after {
                return Some(format!("license block {i} has expired"));
            }
        }
        None
    }

    pub fn issuer(&self) -> Option<&str> {
        self.blocks
            .iter()
            .rev()
            .find(|b| matches!(b.block_type, BlockType::Server | BlockType::Ts5Server))
            .and_then(|b| b.issuer.as_deref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::STANDARD as B64;
    use base64::Engine as _;

    fn parse(b64: &str) -> License {
        let clean: String = b64.chars().filter(|c| !c.is_whitespace()).collect();
        License::parse(&B64.decode(clean).unwrap()).unwrap()
    }

    #[test]
    fn standard_license() {
        let l = parse(
            "AQA1hUFJiiSs0wFXkYuPUJVcDa6XCrZTcsvkB0Ffzz4CmwIITRXgCqeTYAcAAAAgQW5vbnltb3VzAACiIBip9hQaK6P3QhwOJs/BkPn0ioyIDPaNgzJ6M8x0kiAJf4hxCYAxMQ==",
        );
        assert_eq!(l.blocks.len(), 2);
        assert_eq!(l.blocks[0].block_type, BlockType::Server);
        assert_eq!(l.blocks[0].issuer.as_deref(), Some("Anonymous"));
        assert_eq!(l.blocks[1].block_type, BlockType::Ephemeral);
        assert_eq!(l.issuer(), Some("Anonymous"));
        assert!(l.validity_problem(4_000_000_000).is_some());
    }

    #[test]
    fn aal_license_with_intermediates() {
        let l = parse(
            "AQCvbHFTQDY/terPeilrp/ECU9xCH5U3xC92lYTNaY/0KQAJFueAazbsgAAAACVUZWFtU3BlYWsgU3lzdGVtcyBHbWJIAABhl9gwla/UJp2Eszst9TRVXO/PeE6a6d+CTI6Pg7OEVgAJc5CrL4Nh8gAAACRUZWFtU3BlYWsgc3lzdGVtcyBHbWJIAACvTQIgpv6zmLZq3znh7ygmOSokGFkFjz4bTigrOnetrgIJdIIACdS/gAYAAAAAU29zc2VuU3lzdGVtcy5iaWQAADY7+uV1CQ1niOvYSdGzsu83kPTNWijovr3B78eHGeePIAm98vQJvpu0",
        );
        let kinds: Vec<_> = l.blocks.iter().map(|b| b.block_type).collect();
        assert_eq!(
            kinds,
            vec![
                BlockType::Intermediate,
                BlockType::Intermediate,
                BlockType::Server,
                BlockType::Ephemeral
            ]
        );
        assert_eq!(l.blocks[0].issuer.as_deref(), Some("TeamSpeak Systems GmbH"));
        assert_eq!(l.issuer(), Some("SossenSystems.bid"));
        l.derive_public_key().unwrap();
    }

    #[test]
    fn ts5_server_licenses() {
        let long = parse(
            "AQDVsMGbcrMmGif1vSXPWWXNW2CB5Fe9oZ/2uxP29j1EXQAQSfiAazbsgAAAASVUZWFtU3BlYWsgU3lzdGVtcyBHbWJIAAALB6QfbeJyN+9foJhe+/KPFwyU+i++4MAA0q1/WCnizwARRuEPN1aeBQAAASBUZWFtU3BlYWsgc3lzdGVtcyBHbWJIAADrhbI5gUR3thsS7FqKV5P5h7djnwMSJfF2vi58lm1VcwgRUFMAE0P7gAUCGQIAVGVhbVNwZWFrIFN5c3RlbXMgR21iSAAGAwEAAAAFAAf2KhQ7WLjOvwwY0Bi7LxAcWmQeT+LQtuaOzjhYoA+YIBGNq1kRjlQZ",
        );
        assert_eq!(long.blocks.len(), 4);
        assert_eq!(long.blocks[2].block_type, BlockType::Ts5Server);
        assert_eq!(long.blocks[2].issuer.as_deref(), Some("TeamSpeak Systems GmbH"));
        long.derive_public_key().unwrap();

        let three_props = parse(
            "AQDVsMGbcrMmGif1vSXPWWXNW2CB5Fe9oZ/2uxP29j1EXQAQSfiAazbsgAAAASVUZWFtU3BlYWsgU3lzdGVtcyBHbWJIAAAtXG5p2niXlDfpVAGuD88w8hetKYL4vqHRkB5xB8ASRwAR2t/MN+ttjAAAASBUZWFtU3BlYWsgc3lzdGVtcyBHbWJIAAAdZYGtwkeZFhzqnoV1uk+Tcphe8GgcqiPVtELF9y4wOAgR4qmAF4jnAAkDGQIAVGVhbVNwZWFrIFN5c3RlbXMgR21iSAAGAwEAAAAFBgEBAAGGoADzyFvD+9G6uhIxmh0jK+Uo8z8fYGJVH81vWFULDS0l8yATKe4cEyqW3A==",
        );
        assert_eq!(three_props.blocks.len(), 4);
        assert_eq!(three_props.blocks[3].block_type, BlockType::Ephemeral);

        let single = parse(
            "AQAuio9ZxThXKE+hmzQyzBRedysp979JBTv2xP3s2oCkiAgQI70AE+YkAAcBBgMBAAAABQBoazM313063zaipPTH06zrXc91ch3huBYrUET9sEbz1CATKgK8EyqrfA==",
        );
        assert_eq!(single.blocks.len(), 2);
        assert_eq!(single.blocks[0].block_type, BlockType::Ts5Server);

        let with_issuer = parse(
            "AQBgjAAqtcBUrw5futTtkl3+EM3OW4Lal6OTPlwuv4xV/gIRFlEAG0NlAAcAAAAgQW5vbnltb3VzAACKNY+/9qCbonCSxG18vBb7y7zPIgDdjTmcZoAHHclnJSATPa69Ez5XfQ==",
        );
        assert_eq!(with_issuer.blocks.len(), 2);
    }

    #[test]
    fn tsproto_derive_public_key_vector() {
        let l = parse(
            "AQA1hUFJiiSs0wFXkYuPUJVcDa6XCrZTcsvkB0Ffzz4CmwIITRXgCqeTYAcAAAAgQW5vbnltb3VzAAC4R+5mos+UQ/KCbkpQLMI5WRp4wkQu8e5PZY4zU+/FlyAJwaE8CcJJ/A==",
        );
        let expected = [
            0x40, 0xe9, 0x50, 0xc4, 0x61, 0xba, 0x18, 0x3a, 0x1e, 0xb7, 0xcb, 0xb1, 0x9a, 0xc3,
            0xd8, 0xd9, 0xc4, 0xd5, 0x24, 0xdb, 0x38, 0xf7, 0x2d, 0x3d, 0x66, 0x75, 0x77, 0x2a,
            0xc5, 0x9c, 0xc5, 0xc6,
        ];
        assert_eq!(l.derive_public_key().unwrap(), expected);
    }

    #[test]
    fn rejects_garbage() {
        assert!(License::parse(&[]).is_err());
        assert!(License::parse(&[2, 0, 0]).is_err());
        assert!(License::parse(&[1]).is_err());
        assert!(License::parse(&[1, 0, 1, 2, 3]).is_err());
        let mut block = vec![1u8];
        block.extend_from_slice(&[0u8; 42]);
        block[1 + 33] = 99;
        assert!(License::parse(&block).is_err());
    }
}
