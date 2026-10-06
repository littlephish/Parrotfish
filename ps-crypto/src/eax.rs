use aes::Aes128;
use eax::aead::consts::U8;
use eax::aead::{AeadInPlace, KeyInit};
use eax::Eax;

type TsEax = Eax<Aes128, U8>;

pub const MAC_LEN: usize = 8;

pub fn encrypt(key: &[u8; 16], nonce: &[u8; 16], header: &[u8], data: &mut [u8]) -> [u8; MAC_LEN] {
    let cipher = TsEax::new(&(*key).into());
    let tag = cipher
        .encrypt_in_place_detached(&(*nonce).into(), header, data)
        .expect("packet length is far below the EAX limit");
    let mut mac = [0u8; MAC_LEN];
    mac.copy_from_slice(&tag);
    mac
}

pub fn decrypt(
    key: &[u8; 16],
    nonce: &[u8; 16],
    header: &[u8],
    data: &mut [u8],
    mac: &[u8; MAC_LEN],
) -> bool {
    let cipher = TsEax::new(&(*key).into());
    cipher
        .decrypt_in_place_detached(&(*nonce).into(), header, data, &(*mac).into())
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::{DUMMY_KEY, DUMMY_NONCE};

    #[test]
    fn tsproto_fake_encrypt_vector() {
        let header = [0x00, 0x00, 0x00, 0x00, 0x06];
        let mut data = [0x00, 0x00];
        let mac = encrypt(&DUMMY_KEY, &DUMMY_NONCE, &header, &mut data);
        assert_eq!(mac, [0xa4, 0x7b, 0x47, 0x94, 0xdb, 0xa9, 0x6a, 0xc5]);
        assert_eq!(data, [0xfe, 0x18]);
    }

    #[test]
    fn round_trip_and_tamper_detection() {
        let header = [0x12, 0x34, 0x00, 0x07, 0x22];
        let plain: Vec<u8> = (0..100u8).collect();
        let mut data = plain.clone();
        let mac = encrypt(&DUMMY_KEY, &DUMMY_NONCE, &header, &mut data);
        assert_ne!(data, plain);

        let mut ok = data.clone();
        assert!(decrypt(&DUMMY_KEY, &DUMMY_NONCE, &header, &mut ok, &mac));
        assert_eq!(ok, plain);

        let mut bad_data = data.clone();
        bad_data[5] ^= 1;
        let before = bad_data.clone();
        assert!(!decrypt(&DUMMY_KEY, &DUMMY_NONCE, &header, &mut bad_data, &mac));
        assert_eq!(bad_data, before);

        let mut bad_header = header;
        bad_header[4] ^= 0x10;
        let mut d = data.clone();
        assert!(!decrypt(&DUMMY_KEY, &DUMMY_NONCE, &bad_header, &mut d, &mac));

        let mut bad_mac = mac;
        bad_mac[0] ^= 1;
        let mut d = data.clone();
        assert!(!decrypt(&DUMMY_KEY, &DUMMY_NONCE, &header, &mut d, &bad_mac));
    }
}
