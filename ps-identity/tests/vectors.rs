use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use ps_identity::{der, hashcash, Identity, PublicKey};

const TSPROTO_KEY_A: &str = "MG8DAgeAAgEgAiEA6rtKxDn/o/Bo50rNtAE5Ph3h2RKLHQ0gbFkvm2yA79kCIQCrfzAZts/vHP+3MOetKLjNnpZXt4c6U3UB4gWLKR4H9AIgYTyJofmztcTBjq3KZcDdxu+G4RPVwE5vg8VaN2jbQao=";
const TSPROTO_UID_A: &str = "test/9PZ9vww/Bpf5vJxtJhpz80=";

const TSPROTO_KEY_B: &str = "MG0DAgeAAgEgAiAIXJBlj1hQbaH0Eq0DuLlCmH8bl+veTAO2+k9EQjEYSgIgNnImcmKo7ls5mExb6skfK2Tw+u54aeDr0OP1ITsC/50CIA8M5nmDBnmDM/gZ//4AAAAAAAAAAAAAAAAAAAAZRzOI";
const TSPROTO_UID_B: &str = "lks7QL5OVMKo4pZ79cEOI5r5oEA=";

fn identity_from_plain_der(b64: &str, key_offset: u64) -> Identity {
    let parsed = der::parse_identity_key(&B64.decode(b64).unwrap()).unwrap();
    Identity {
        name: "test".into(),
        nickname: "test".into(),
        phonetic_nickname: String::new(),
        key_offset,
        public_key: PublicKey { x: parsed.x, y: parsed.y },
        private_key: parsed.private,
    }
}

#[test]
fn tsproto_uid_vectors() {
    let a = identity_from_plain_der(TSPROTO_KEY_A, 0);
    assert_eq!(a.uid(), TSPROTO_UID_A);
    assert_eq!(a.key_matches(), Some(true));
    let b = identity_from_plain_der(TSPROTO_KEY_B, 0);
    assert_eq!(b.uid(), TSPROTO_UID_B);
    assert_eq!(b.key_matches(), Some(true));
}

#[test]
fn tsproto_security_level_vector() {
    let a = identity_from_plain_der(TSPROTO_KEY_A, 2792354);
    assert_eq!(a.security_level(), 21);
    assert_eq!(hashcash::security_level(&a.public_key_der_base64(), 2792354), 21);
}

#[test]
fn plain_der_reencodes_identically() {
    for key in [TSPROTO_KEY_A, TSPROTO_KEY_B] {
        let raw = B64.decode(key).unwrap();
        let parsed = der::parse_identity_key(&raw).unwrap();
        let again = der::encode_identity_key(&parsed.x, &parsed.y, parsed.private.as_ref());
        assert_eq!(again, raw);
    }
}

#[test]
fn export_value_round_trips() {
    for (key, uid, offset) in [(TSPROTO_KEY_A, TSPROTO_UID_A, 2792354u64), (TSPROTO_KEY_B, TSPROTO_UID_B, 4242)] {
        let id = identity_from_plain_der(key, offset);
        let value = id.export_value().unwrap();
        assert!(value.starts_with(&format!("{offset}V")));
        let (key_offset, public_key, private_key) = ps_identity::decode_identity_value(&value).unwrap();
        assert_eq!(key_offset, offset);
        assert_eq!(public_key, id.public_key);
        assert_eq!(private_key, id.private_key);
        let again = Identity {
            name: "Default".into(),
            nickname: "Tester".into(),
            phonetic_nickname: String::new(),
            key_offset,
            public_key,
            private_key,
        };
        assert_eq!(again.export_value().unwrap(), value);
        assert_eq!(again.uid(), uid);
    }
}

#[test]
fn public_key_der_round_trip() {
    let id = identity_from_plain_der(TSPROTO_KEY_B, 0);
    let public_der = id.public_key.der();
    assert_eq!(public_der[0], 0x30);
    let parsed = der::parse_identity_key(&public_der).unwrap();
    assert_eq!(parsed.x, id.public_key.x);
    assert_eq!(parsed.y, id.public_key.y);
    assert!(parsed.private.is_none());
    assert_eq!(PublicKey::from_der(&public_der).unwrap(), id.public_key);
}

#[test]
fn ini_round_trip() {
    let mut id = identity_from_plain_der(TSPROTO_KEY_A, 2792354);
    id.name = "Default".into();
    id.nickname = "Tester".into();
    id.phonetic_nickname = "tester".into();
    let back = Identity::from_ini(&id.to_ini().unwrap()).unwrap();
    assert_eq!(back.name, "Default");
    assert_eq!(back.nickname, "Tester");
    assert_eq!(back.phonetic_nickname, "tester");
    assert_eq!(back.key_offset, id.key_offset);
    assert_eq!(back.public_key, id.public_key);
    assert_eq!(back.private_key, id.private_key);
}

#[test]
fn generated_identity_is_usable() {
    let id = Identity::generate("Generated", "Tester");
    assert!(id.on_curve());
    assert_eq!(id.key_matches(), Some(true));
    assert!(id.security_level() >= 8);
    let back = Identity::from_ini(&id.to_ini().unwrap()).unwrap();
    assert_eq!(back.uid(), id.uid());
    assert_eq!(back.private_key, id.private_key);
    assert_eq!(back.key_offset, id.key_offset);
}

#[test]
fn sign_and_verify() {
    let id = identity_from_plain_der(TSPROTO_KEY_A, 0);
    let sig = id.sign(b"phishspeak proof").unwrap();
    assert_eq!(sig[0], 0x30);
    assert!(id.public_key.verify(b"phishspeak proof", &sig));
    assert!(!id.public_key.verify(b"phishspeak prooF", &sig));
}

#[test]
fn verifies_real_server_license_signature() {
    let license = B64
        .decode("AQBM0LZCVmZ7CX/miewqdjOyuKa6kI78Fk43LoypifqOkAIOkvUAEn46gAcAAAAgQW5vbnltb3VzAABoruUa34pO9zy1Z5zIOmrkIO06lKg/+mBrg6Mw1Rg4OyAPa7A3D2xY9w==")
        .unwrap();
    let signature = B64
        .decode("MEUCIQC+ececxC0NCcuCtrXHAO5h7qbh1s/TGP/AaHa6+wV38wIgV9wwSppEdGjwuH3ETAME9tDj3aNkNvL25i0ikF9vs8M=")
        .unwrap();
    let server = PublicKey::from_der_base64(
        "MEwDAgcAAgEgAiEA96WgYeYU8zoPqXJqicita+rR92FvnTlxYcUUyIDkQ6cCIE/KPo+ms3BEzN/HBR71BJ/Z1Fv8918mdDKLetbOGKWt",
    )
    .unwrap();
    assert!(server.verify(&license, &signature));
    let mut tampered = license.clone();
    tampered[10] ^= 1;
    assert!(!server.verify(&tampered, &signature));
}
