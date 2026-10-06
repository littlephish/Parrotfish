pub mod curve;
pub mod der;
pub mod hashcash;
pub mod ini;

use std::fs;
use std::path::{Path, PathBuf};

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use p256::ecdsa::signature::{Signer as _, Verifier as _};
use sha1::Digest as _;
use sha1::Sha1;

pub const OBSCURED_IDENTITY_KEY: &[u8] = b"b9dfaa7bee6ac57ac7b65f1094a1c155e747327bc2fe5d51c512023fe54a280201004e90ad1daaae1075d53b7d571c30e063b5a62a4a017bb394833aa0983e6e";

pub const DEFAULT_SECURITY_LEVEL: u8 = 8;

#[derive(Debug)]
pub enum IdentityError {
    Io(std::io::Error),
    Ini(String),
    Decode(String),
    Der(String),
    Key(String),
}

impl std::fmt::Display for IdentityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IdentityError::Io(e) => write!(f, "io: {e}"),
            IdentityError::Ini(s) => write!(f, "ini: {s}"),
            IdentityError::Decode(s) => write!(f, "decode: {s}"),
            IdentityError::Der(s) => write!(f, "der: {s}"),
            IdentityError::Key(s) => write!(f, "key: {s}"),
        }
    }
}

impl std::error::Error for IdentityError {}

impl From<std::io::Error> for IdentityError {
    fn from(e: std::io::Error) -> Self {
        IdentityError::Io(e)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicKey {
    pub x: [u8; 32],
    pub y: [u8; 32],
}

impl PublicKey {
    pub fn from_der(asn: &[u8]) -> Result<Self, IdentityError> {
        let parsed = der::parse_identity_key(asn)?;
        Ok(Self { x: parsed.x, y: parsed.y })
    }

    pub fn from_der_base64(omega: &str) -> Result<Self, IdentityError> {
        let asn = B64
            .decode(omega.trim().as_bytes())
            .map_err(|e| IdentityError::Decode(format!("omega base64: {e}")))?;
        Self::from_der(&asn)
    }

    pub fn der(&self) -> Vec<u8> {
        der::encode_identity_key(&self.x, &self.y, None)
    }

    pub fn der_base64(&self) -> String {
        B64.encode(self.der())
    }

    pub fn uid(&self) -> String {
        let b64 = self.der_base64();
        B64.encode(Sha1::digest(b64.as_bytes()))
    }

    pub fn verify(&self, data: &[u8], der_signature: &[u8]) -> bool {
        let point = p256::EncodedPoint::from_affine_coordinates(
            &p256::FieldBytes::from(self.x),
            &p256::FieldBytes::from(self.y),
            false,
        );
        let Ok(key) = p256::ecdsa::VerifyingKey::from_encoded_point(&point) else {
            return false;
        };
        let Ok(sig) = p256::ecdsa::Signature::from_der(der_signature) else {
            return false;
        };
        key.verify(data, &sig).is_ok()
    }
}

#[derive(Debug, Clone)]
pub struct Identity {
    pub name: String,
    pub nickname: String,
    pub phonetic_nickname: String,
    pub key_offset: u64,
    pub public_key: PublicKey,
    pub private_key: Option<[u8; 32]>,
}

impl Identity {
    pub fn load(path: &Path) -> Result<Self, IdentityError> {
        let text = fs::read_to_string(path)?;
        Self::from_ini(&text)
    }

    pub fn from_ini(text: &str) -> Result<Self, IdentityError> {
        let sections = ini::parse(text).map_err(IdentityError::Ini)?;
        let id_value = ini::get(&sections, "Identity", "identity")
            .ok_or_else(|| IdentityError::Ini("missing identity key".into()))?;
        let (key_offset, public_key, private_key) = decode_identity_value(id_value)?;
        Ok(Self {
            name: ini::get(&sections, "Identity", "id").unwrap_or("").to_string(),
            nickname: ini::get(&sections, "Identity", "nickname").unwrap_or("").to_string(),
            phonetic_nickname: ini::get(&sections, "Identity", "phonetic_nickname")
                .unwrap_or("")
                .to_string(),
            key_offset,
            public_key,
            private_key,
        })
    }

    pub fn generate(name: &str, nickname: &str) -> Self {
        let secret = p256::SecretKey::random(&mut rand::rngs::OsRng);
        let mut private = [0u8; 32];
        private.copy_from_slice(&secret.to_bytes());
        let (x, y) = curve::scalar_mul_g(&private).expect("fresh secret key is valid");
        let mut id = Self {
            name: name.to_string(),
            nickname: nickname.to_string(),
            phonetic_nickname: String::new(),
            key_offset: 0,
            public_key: PublicKey { x, y },
            private_key: Some(private),
        };
        id.improve_security_level(DEFAULT_SECURITY_LEVEL, &|| false);
        id
    }

    pub fn uid(&self) -> String {
        self.public_key.uid()
    }

    pub fn public_key_der(&self) -> Vec<u8> {
        self.public_key.der()
    }

    pub fn public_key_der_base64(&self) -> String {
        self.public_key.der_base64()
    }

    pub fn on_curve(&self) -> bool {
        curve::is_on_curve(&self.public_key.x, &self.public_key.y)
    }

    pub fn key_matches(&self) -> Option<bool> {
        self.private_key
            .map(|k| curve::scalar_mul_g(&k)
                .map(|(rx, ry)| rx == self.public_key.x && ry == self.public_key.y)
                .unwrap_or(false))
    }

    pub fn security_level(&self) -> u8 {
        hashcash::security_level(&self.public_key_der_base64(), self.key_offset)
    }

    pub fn improve_security_level(&mut self, target: u8, should_stop: &dyn Fn() -> bool) -> bool {
        if self.security_level() >= target {
            return true;
        }
        let omega = self.public_key_der_base64();
        match hashcash::improve_security_level(&omega, self.key_offset, target, should_stop) {
            Some(offset) => {
                self.key_offset = offset;
                true
            }
            None => false,
        }
    }

    pub fn sign(&self, data: &[u8]) -> Result<Vec<u8>, IdentityError> {
        let private = self
            .private_key
            .ok_or_else(|| IdentityError::Key("identity has no private key".into()))?;
        let key = p256::ecdsa::SigningKey::from_bytes(&p256::FieldBytes::from(private))
            .map_err(|e| IdentityError::Key(format!("invalid private key: {e}")))?;
        let sig: p256::ecdsa::Signature = key.sign(data);
        Ok(sig.to_der().as_bytes().to_vec())
    }

    pub fn export_value(&self) -> Result<String, IdentityError> {
        let private = self
            .private_key
            .ok_or_else(|| IdentityError::Key("identity has no private key".into()))?;
        let asn = der::encode_identity_key(&self.public_key.x, &self.public_key.y, Some(&private));
        let mut data = B64.encode(asn).into_bytes();
        for i in 0..data.len().min(100) {
            data[i] ^= OBSCURED_IDENTITY_KEY[i];
        }
        if data.len() < 21 {
            return Err(IdentityError::Key("identity payload too short".into()));
        }
        let null_idx = data[20..].iter().position(|b| *b == 0).unwrap_or(data.len() - 20);
        let hash = Sha1::digest(&data[20..20 + null_idx]);
        for i in 0..20 {
            data[i] ^= hash[i];
        }
        Ok(format!("{}V{}", self.key_offset, B64.encode(data)))
    }

    pub fn to_ini(&self) -> Result<String, IdentityError> {
        Ok(format!(
            "[Identity]\nid={}\nidentity=\"{}\"\nnickname={}\nphonetic_nickname={}\n",
            self.name,
            self.export_value()?,
            self.nickname,
            self.phonetic_nickname
        ))
    }

    pub fn save(&self, path: &Path) -> Result<(), IdentityError> {
        fs::write(path, self.to_ini()?)?;
        Ok(())
    }

    pub fn summary(&self) -> String {
        let mut s = String::new();
        s.push_str(&format!("Name: {}\n", self.name));
        s.push_str(&format!("Nickname: {}\n", self.nickname));
        s.push_str(&format!(
            "Security level: {} (key offset {})\n",
            self.security_level(),
            self.key_offset
        ));
        s.push_str(&format!("UID: {}\n", self.uid()));
        s.push_str(&format!("Point on P-256: {}\n", self.on_curve()));
        match self.key_matches() {
            Some(true) => s.push_str("Private key: matches public key (k*G == point)\n"),
            Some(false) => s.push_str("Private key: does NOT match public key\n"),
            None => s.push_str("Private key: not present (public-key-only identity)\n"),
        }
        s.push_str(&format!("Public key (DER base64): {}\n", self.public_key_der_base64()));
        s
    }
}

pub fn decode_identity_value(value: &str) -> Result<(u64, PublicKey, Option<[u8; 32]>), IdentityError> {
    let v = value.trim().trim_matches('"');
    let vidx = v
        .find('V')
        .ok_or_else(|| IdentityError::Decode("missing 'V' separator".into()))?;
    let key_offset: u64 = v[..vidx]
        .parse()
        .map_err(|_| IdentityError::Decode("invalid key offset prefix".into()))?;
    let mut data = B64
        .decode(v[vidx + 1..].as_bytes())
        .map_err(|e| IdentityError::Decode(format!("outer base64: {e}")))?;
    if data.len() < 21 {
        return Err(IdentityError::Decode(format!("payload too short: {}", data.len())));
    }
    let null_idx = data[20..].iter().position(|b| *b == 0).unwrap_or(data.len() - 20);
    let hash = Sha1::digest(&data[20..20 + null_idx]);
    for i in 0..20 {
        data[i] ^= hash[i];
    }
    for i in 0..data.len().min(100) {
        data[i] ^= OBSCURED_IDENTITY_KEY[i];
    }
    let nul = data.iter().position(|b| *b == 0).unwrap_or(data.len());
    let text = std::str::from_utf8(&data[..nul])
        .map_err(|e| IdentityError::Decode(format!("inner text: {e}")))?;
    let asn = B64
        .decode(text.as_bytes())
        .map_err(|e| IdentityError::Decode(format!("inner base64: {e}")))?;
    let parsed = der::parse_identity_key(&asn)?;
    let private = if parsed.bit_info == 0x80 { parsed.private } else { None };
    Ok((key_offset, PublicKey { x: parsed.x, y: parsed.y }, private))
}

pub fn list_identities(path: &Path) -> Vec<(PathBuf, Result<Identity, IdentityError>)> {
    if path.is_file() {
        return vec![(path.to_path_buf(), Identity::load(path))];
    }
    let Ok(rd) = fs::read_dir(path) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in rd.flatten() {
        let p = entry.path();
        let is_ini = p
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("ini"))
            .unwrap_or(false);
        if is_ini && p.is_file() {
            let loaded = Identity::load(&p);
            let relevant = match &loaded {
                Ok(_) => true,
                Err(IdentityError::Ini(_)) => false,
                Err(_) => true,
            };
            if relevant {
                out.push((p.clone(), loaded));
            }
        }
    }
    out.sort_by(|a, b| a.0.file_name().cmp(&b.0.file_name()));
    out
}
