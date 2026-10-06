use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use curve25519_dalek::edwards::{CompressedEdwardsY, EdwardsPoint};
use curve25519_dalek::scalar::clamp_integer;
use num_bigint::BigUint;
use ps_identity::{Identity, PublicKey};
use rand::RngCore as _;
use sha1::Digest as _;
use sha1::Sha1;

use crate::keys::SessionKeys;
use crate::license::License;
use crate::{decode_b64, CryptoError};

pub const MAX_PUZZLE_LEVEL: u32 = 1_000_000;

pub fn solve_puzzle(x: &[u8; 64], n: &[u8; 64], level: u32) -> Result<[u8; 64], CryptoError> {
    if level > MAX_PUZZLE_LEVEL {
        return Err(CryptoError::Puzzle(format!("level {level} is not acceptable")));
    }
    let modulus = BigUint::from_bytes_be(n);
    if modulus.bits() == 0 {
        return Err(CryptoError::Puzzle("modulus is zero".into()));
    }
    let base = BigUint::from_bytes_be(x);
    let exponent = BigUint::from(1u8) << (level as usize);
    let y = base.modpow(&exponent, &modulus).to_bytes_be();
    if y.len() > 64 {
        return Err(CryptoError::Puzzle("result does not fit 64 bytes".into()));
    }
    let mut out = [0u8; 64];
    out[64 - y.len()..].copy_from_slice(&y);
    Ok(out)
}

pub fn random_alpha() -> [u8; 10] {
    let mut alpha = [0u8; 10];
    rand::rngs::OsRng.fill_bytes(&mut alpha);
    alpha
}

#[derive(Clone)]
pub struct EphemeralKey {
    private: [u8; 32],
    pub public: [u8; 32],
}

impl EphemeralKey {
    pub fn generate() -> Self {
        let mut private = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut private);
        Self::from_private(private)
    }

    pub fn from_private(private: [u8; 32]) -> Self {
        let private = clamp_integer(private);
        let public = EdwardsPoint::mul_base_clamped(private).compress().to_bytes();
        Self { private, public }
    }

    pub fn shared_point(&self, peer_public: &[u8; 32]) -> Result<[u8; 32], CryptoError> {
        let point = CompressedEdwardsY(*peer_public)
            .decompress()
            .ok_or_else(|| CryptoError::Handshake("cannot decompress server ephemeral key".into()))?;
        Ok(point.mul_clamped(self.private).compress().to_bytes())
    }
}

#[derive(Debug, Clone)]
pub struct ServerHello {
    pub license: Vec<u8>,
    pub beta: [u8; 54],
    pub omega: String,
    pub proof: Vec<u8>,
}

impl ServerHello {
    pub fn from_base64(license: &str, beta: &str, omega: &str, proof: &str) -> Result<Self, CryptoError> {
        let license = decode_b64("l", license)?;
        let beta_vec = decode_b64("beta", beta)?;
        if beta_vec.len() != 54 {
            return Err(CryptoError::Handshake(format!(
                "beta has {} bytes, expected 54",
                beta_vec.len()
            )));
        }
        let mut beta_arr = [0u8; 54];
        beta_arr.copy_from_slice(&beta_vec);
        let proof = decode_b64("proof", proof)?;
        Ok(Self { license, beta: beta_arr, omega: omega.trim().to_string(), proof })
    }
}

#[derive(Debug, Clone)]
pub struct HandshakeOutcome {
    pub keys: SessionKeys,
    pub ek: String,
    pub proof: String,
    pub server_uid: String,
    pub license: License,
}

pub fn complete_handshake(
    identity: &Identity,
    alpha: &[u8; 10],
    hello: &ServerHello,
) -> Result<HandshakeOutcome, CryptoError> {
    complete_handshake_with(identity, alpha, hello, EphemeralKey::generate())
}

pub fn complete_handshake_with(
    identity: &Identity,
    alpha: &[u8; 10],
    hello: &ServerHello,
    ephemeral: EphemeralKey,
) -> Result<HandshakeOutcome, CryptoError> {
    let server_key = PublicKey::from_der_base64(&hello.omega)
        .map_err(|e| CryptoError::Handshake(format!("server public key is invalid: {e}")))?;
    if !server_key.verify(&hello.license, &hello.proof) {
        return Err(CryptoError::Handshake(
            "the server's license proof is not valid; the connection may be tampered with".into(),
        ));
    }
    let license = License::parse(&hello.license)?;
    let server_ephemeral = license.derive_public_key()?;
    let shared = ephemeral.shared_point(&server_ephemeral)?;
    let keys = SessionKeys::from_shared_point(&shared, alpha, &hello.beta);

    let mut to_sign = Vec::with_capacity(86);
    to_sign.extend_from_slice(&ephemeral.public);
    to_sign.extend_from_slice(&hello.beta);
    let signature = identity
        .sign(&to_sign)
        .map_err(|e| CryptoError::Identity(e.to_string()))?;

    Ok(HandshakeOutcome {
        keys,
        ek: B64.encode(ephemeral.public),
        proof: B64.encode(signature),
        server_uid: B64.encode(Sha1::digest(hello.omega.as_bytes())),
        license,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Sha256, Sha512};

    const CLIENT_EK: [u8; 32] = [
        0xb0, 0x4e, 0xa1, 0xd9, 0x5c, 0x72, 0x64, 0xdf, 0x0d, 0xe8, 0xb3, 0x6b, 0xaa, 0x7c, 0xa1,
        0x5f, 0x75, 0x71, 0xf5, 0x1f, 0xa0, 0x54, 0xb5, 0x51, 0x27, 0x08, 0x8e, 0xdd, 0x96, 0x3d,
        0x6e, 0x79,
    ];

    const LICENSE: &str = "AQA1hUFJiiSs0wFXkYuPUJVcDa6XCrZTcsvkB0Ffzz4CmwIITRXgCqeTYAcAAAAgQW5vbnltb3VzAAC4R+5mos+UQ/KCbkpQLMI5WRp4wkQu8e5PZY4zU+/FlyAJwaE8CcJJ/A==";

    #[test]
    fn puzzle_small_numbers() {
        let mut x = [0u8; 64];
        x[63] = 3;
        let mut n = [0u8; 64];
        n[63] = 7;
        let y = solve_puzzle(&x, &n, 2).unwrap();
        assert_eq!(y[63], 4);
        assert!(y[..63].iter().all(|b| *b == 0));

        let y0 = solve_puzzle(&x, &n, 0).unwrap();
        assert_eq!(y0[63], 3);
    }

    #[test]
    fn puzzle_matches_repeated_squaring() {
        let mut x = [0u8; 64];
        let mut n = [0u8; 64];
        for i in 0..64 {
            x[i] = (i as u8).wrapping_mul(37).wrapping_add(11);
            n[i] = (i as u8).wrapping_mul(101).wrapping_add(3);
        }
        n[0] |= 0x80;
        n[63] |= 1;
        x[0] &= 0x7f;
        let level = 500;
        let modulus = BigUint::from_bytes_be(&n);
        let mut acc = BigUint::from_bytes_be(&x) % &modulus;
        for _ in 0..level {
            acc = (&acc * &acc) % &modulus;
        }
        let expect = acc.to_bytes_be();
        let got = solve_puzzle(&x, &n, level).unwrap();
        assert_eq!(&got[64 - expect.len()..], &expect[..]);
        assert!(got[..64 - expect.len()].iter().all(|b| *b == 0));
    }

    #[test]
    fn puzzle_rejects_bad_input() {
        let x = [1u8; 64];
        assert!(solve_puzzle(&x, &[0u8; 64], 10).is_err());
        assert!(solve_puzzle(&x, &[5u8; 64], MAX_PUZZLE_LEVEL + 1).is_err());
    }

    #[test]
    fn tsproto_shared_iv_vector() {
        let license = License::parse(&B64.decode(LICENSE).unwrap()).unwrap();
        let server_key = license.derive_public_key().unwrap();
        let ek = EphemeralKey::from_private(CLIENT_EK);

        let mut alpha = [0u8; 10];
        alpha.copy_from_slice(&B64.decode("Jkxq1wIvvhzaCA==").unwrap());
        let mut beta = [0u8; 54];
        beta.copy_from_slice(
            &B64.decode("wU5T/MM6toW6Wge9th7VlTlzVZ9JDWypw2P9migfc25pjGP2Tj7Hm6rJpmKeHRr08Ch7BEAR")
                .unwrap(),
        );

        let shared = ek.shared_point(&server_key).unwrap();
        let plain_iv: [u8; 64] = Sha512::digest(shared).into();
        let expected_plain: [u8; 64] = [
            0x58, 0x78, 0xae, 0x08, 0x08, 0x72, 0x05, 0xb0, 0x13, 0x27, 0x10, 0xe9, 0x81, 0xb4,
            0xaf, 0x14, 0x14, 0x71, 0xad, 0xcd, 0x82, 0x98, 0xf3, 0xd1, 0x1d, 0x07, 0x20, 0x72,
            0x7e, 0xb2, 0x1b, 0x89, 0x47, 0x82, 0x1e, 0xfb, 0x02, 0x53, 0x5a, 0x8a, 0x52, 0x4d,
            0x9a, 0x7a, 0x09, 0x2c, 0x1b, 0xe7, 0x1f, 0xd1, 0x9d, 0x2a, 0x9d, 0x4f, 0xbd, 0xe3,
            0x22, 0x09, 0xe4, 0x86, 0x7d, 0x63, 0x49, 0x07,
        ];
        assert_eq!(plain_iv, expected_plain);

        let keys = SessionKeys::from_shared_point(&shared, &alpha, &beta);
        let expected_xored: [u8; 64] = [
            0x7e, 0x34, 0xc4, 0xdf, 0x0a, 0x5d, 0xbb, 0xac, 0xc9, 0x2f, 0xd1, 0xa7, 0xd2, 0x48,
            0x6c, 0x2e, 0xa2, 0xf4, 0x17, 0x97, 0x85, 0x25, 0x45, 0xcf, 0xc8, 0x92, 0x19, 0x01,
            0x2b, 0x2d, 0x52, 0x84, 0x2b, 0x2b, 0xdd, 0x98, 0xff, 0xc9, 0x72, 0x95, 0x21, 0x23,
            0xf3, 0xf6, 0x6a, 0xda, 0x55, 0xd9, 0xd8, 0x4a, 0x37, 0xe3, 0x3b, 0x2d, 0x23, 0xfe,
            0x38, 0xfd, 0x14, 0xae, 0x06, 0x67, 0x09, 0x16,
        ];
        assert_eq!(keys.shared_iv, expected_xored);

        let (key, nonce) = keys.key_nonce(false, 2, 0, 0);
        let expected_keynonce: [u8; 32] = [
            0xf3, 0x70, 0xd3, 0x43, 0xe7, 0x78, 0x15, 0x70, 0x7a, 0xff, 0x60, 0x48, 0xfb, 0xd9,
            0xac, 0x6b, 0xb6, 0x33, 0x35, 0x79, 0x31, 0x9b, 0x88, 0x0e, 0x2d, 0x25, 0xef, 0x9c,
            0xe9, 0x9e, 0x77, 0x5c,
        ];
        assert_eq!(key, expected_keynonce[..16]);
        assert_eq!(nonce, expected_keynonce[16..]);

        let mut temp = [0u8; 70];
        temp[0] = 0x31;
        temp[1] = 2;
        temp[6..].copy_from_slice(&keys.shared_iv);
        assert_eq!(Sha256::digest(temp)[..], expected_keynonce[..]);

        let (key_pid, _) = keys.key_nonce(false, 2, 0x0102, 0);
        assert_eq!(key_pid[0], expected_keynonce[0] ^ 0x01);
        assert_eq!(key_pid[1], expected_keynonce[1] ^ 0x02);
        assert_eq!(key_pid[2..], expected_keynonce[2..16]);

        let (key_s2c, _) = keys.key_nonce(true, 2, 0, 0);
        assert_ne!(key_s2c, key);
        let (key_gen, _) = keys.key_nonce(false, 2, 0, 1);
        assert_ne!(key_gen, key);
    }

    #[test]
    fn ephemeral_keys_agree_like_diffie_hellman() {
        let a = EphemeralKey::generate();
        let b = EphemeralKey::generate();
        assert_ne!(a.public, b.public);
        assert_eq!(a.shared_point(&b.public).unwrap(), b.shared_point(&a.public).unwrap());
    }

    #[test]
    fn full_handshake_against_simulated_server() {
        let server_identity = Identity::generate("server", "server");
        let client_identity = Identity::generate("client", "client");
        let license_bytes = B64.decode(LICENSE).unwrap();
        let proof = server_identity.sign(&license_bytes).unwrap();
        let beta = [0x5au8; 54];
        let hello = ServerHello::from_base64(
            LICENSE,
            &B64.encode(beta),
            &server_identity.public_key_der_base64(),
            &B64.encode(&proof),
        )
        .unwrap();
        let alpha = random_alpha();

        let out = complete_handshake(&client_identity, &alpha, &hello).unwrap();
        assert_eq!(out.server_uid, server_identity.uid());
        assert_eq!(out.license.blocks.len(), 2);

        let ek = B64.decode(&out.ek).unwrap();
        assert_eq!(ek.len(), 32);
        let mut signed = ek.clone();
        signed.extend_from_slice(&beta);
        assert!(client_identity.public_key.verify(&signed, &B64.decode(&out.proof).unwrap()));

        let mut bad = hello.clone();
        bad.license[50] ^= 1;
        assert!(complete_handshake(&client_identity, &alpha, &bad).is_err());

        assert!(ServerHello::from_base64(LICENSE, "AAAA", &hello.omega, "AAAA").is_err());
    }
}
