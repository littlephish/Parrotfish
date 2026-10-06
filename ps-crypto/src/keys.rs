use sha1::Digest as _;
use sha1::Sha1;
use sha2::{Sha256, Sha512};

pub const DUMMY_KEY: [u8; 16] = *b"c:\\windows\\syste";
pub const DUMMY_NONCE: [u8; 16] = *b"m\\firewall32.cpl";
pub const INIT_MAC: [u8; 8] = *b"TS3INIT1";

#[derive(Clone, PartialEq, Eq)]
pub struct SessionKeys {
    pub shared_iv: [u8; 64],
    pub shared_mac: [u8; 8],
}

impl std::fmt::Debug for SessionKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SessionKeys(..)")
    }
}

impl SessionKeys {
    pub fn from_shared_point(shared_point: &[u8; 32], alpha: &[u8; 10], beta: &[u8; 54]) -> Self {
        let mut shared_iv = [0u8; 64];
        shared_iv.copy_from_slice(&Sha512::digest(shared_point));
        Self::from_hashed_secret(shared_iv, alpha, beta)
    }

    pub fn from_hashed_secret(mut shared_iv: [u8; 64], alpha: &[u8; 10], beta: &[u8; 54]) -> Self {
        for i in 0..10 {
            shared_iv[i] ^= alpha[i];
        }
        for i in 0..54 {
            shared_iv[10 + i] ^= beta[i];
        }
        let mut shared_mac = [0u8; 8];
        shared_mac.copy_from_slice(&Sha1::digest(shared_iv)[..8]);
        Self { shared_iv, shared_mac }
    }

    pub fn key_nonce(
        &self,
        from_server: bool,
        packet_type: u8,
        packet_id: u16,
        generation: u32,
    ) -> ([u8; 16], [u8; 16]) {
        let mut temp = [0u8; 70];
        temp[0] = if from_server { 0x30 } else { 0x31 };
        temp[1] = packet_type & 0x0f;
        temp[2..6].copy_from_slice(&generation.to_be_bytes());
        temp[6..].copy_from_slice(&self.shared_iv);
        let hash = Sha256::digest(temp);
        let mut key = [0u8; 16];
        let mut nonce = [0u8; 16];
        key.copy_from_slice(&hash[..16]);
        nonce.copy_from_slice(&hash[16..]);
        key[0] ^= (packet_id >> 8) as u8;
        key[1] ^= packet_id as u8;
        (key, nonce)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::STANDARD as B64;
    use base64::Engine as _;

    fn hex(s: &str) -> Vec<u8> {
        let clean: String = s.chars().filter(|c| c.is_ascii_hexdigit()).collect();
        (0..clean.len() / 2)
            .map(|i| u8::from_str_radix(&clean[i * 2..i * 2 + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn dummy_key_is_the_firewall_path() {
        let mut joined = DUMMY_KEY.to_vec();
        joined.extend_from_slice(&DUMMY_NONCE);
        assert_eq!(joined, b"c:\\windows\\system\\firewall32.cpl");
    }

    #[test]
    fn tsproto_new_decrypt_vector() {
        let secret = hex("C2456FCBFC2208AE442B7DE73A671BDA9309B200F2CD104908CD3AB07BDD58AD");
        let mut point = [0u8; 32];
        point.copy_from_slice(&secret);
        let beta_vec = B64
            .decode("I4onb0zMyAD6bd24QANDls40eOES7qmjonBFtt5wRWzAfIIQWTSxjEas6TGTZIJ8QSJNX+Pl")
            .unwrap();
        let mut beta = [0u8; 54];
        beta.copy_from_slice(&beta_vec);
        let keys = SessionKeys::from_shared_point(&point, &[0u8; 10], &beta);

        let (key, nonce) = keys.key_nonce(false, 2, 0, 0);
        assert_eq!(key.to_vec(), hex("D24275719CEE8335EF8ACEE0B72840B8"));
        assert_eq!(nonce.to_vec(), hex("9CD930B758FE5023646611C5360EA25F"));

        let packet = hex("2b982443ab38be6b00020000329abf64d4572e1349897b5e1e96fbc4a763a4c4ce1f64f0c1e3febd0a5f04a82ab1f2bc2344bb374fd16181beb8233b5b06944280470e9b6893290a1da0776ffcd89f3beec2ce23b9694930c09efaaea0d88a6895a08ede4d5cbfea61291fc553ac651f1e2bc1d2bd277a8bd9ab5386415579a9e56fac46d8b6b119f454bebd99179cd317dec60af205341d11f274d02bbacdd7e9773f72a426358ca1d39016dd95bde2409cd81bf99b340887e997ea982370c6790cf4d23150460820224766838ea4ec4d71dd102ede701ea0001f392623aa410dd9ab0e45874da82e29e6e370515ec30a37dd73f5a364c233ff014384beab5f1708c9f48dfba33a520f8fcdcef055789c54693c3fe72c5bfaca7cb4ca1fed77b8624660b8abc882f4b95b1284cb6dc55019c6082dd6dd146fa50383662d7298bef04ababaf1af80e15cd4c1f81326f085788e2918e00324147dce39b23db71326abc3de4b94df10f1531e9cce202bba71fa3ebeefd77b21fa3260a62e92eeee2183421d384a8c48777e2f9efbc58d4f442c5f0529c7c0e27e81b2b6b1b05eb8fa19256886248d553582dfd24c7cfab3c3f7317a5cebc6504b53fa0e86fc8c1100fc1d506fcf96caa76a7c0b6a27e577f2efdecd4070e847a559bf37d75bfdbe9e814c702426ce696d8645bc300b5f28f9e7f1ce");
        let mut mac = [0u8; 8];
        mac.copy_from_slice(&packet[..8]);
        let header = &packet[8..13];
        assert_eq!(header, &[0x00, 0x02, 0x00, 0x00, 0x32]);
        let mut data = packet[13..].to_vec();
        let (key, nonce) = keys.key_nonce(false, header[4], 2, 0);
        assert!(crate::eax::decrypt(&key, &nonce, header, &mut data, &mac));
        let text = String::from_utf8_lossy(&data);
        assert!(text.starts_with("clientinit "), "{text}");
    }
}
