use sha1::Digest as _;
use sha1::Sha1;

pub fn security_level(omega: &str, key_offset: u64) -> u8 {
    let mut hasher = Sha1::new();
    hasher.update(omega.as_bytes());
    hasher.update(key_offset.to_string().as_bytes());
    leading_zero_bits(&hasher.finalize())
}

fn leading_zero_bits(hash: &[u8]) -> u8 {
    let mut bits = 0u8;
    for b in hash {
        if *b == 0 {
            bits = bits.saturating_add(8);
        } else {
            bits = bits.saturating_add(b.trailing_zeros() as u8);
            break;
        }
    }
    bits
}

pub fn improve_security_level(
    omega: &str,
    start_offset: u64,
    target: u8,
    should_stop: &dyn Fn() -> bool,
) -> Option<u64> {
    let mut base = Sha1::new();
    base.update(omega.as_bytes());
    let mut offset = start_offset;
    loop {
        let mut h = base.clone();
        h.update(offset.to_string().as_bytes());
        if leading_zero_bits(&h.finalize()) >= target {
            return Some(offset);
        }
        offset = offset.checked_add(1)?;
        if offset & 0xffff == 0 && should_stop() {
            return None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_bits_lsb_first() {
        assert_eq!(leading_zero_bits(&[0x01, 0xff]), 0);
        assert_eq!(leading_zero_bits(&[0x02, 0xff]), 1);
        assert_eq!(leading_zero_bits(&[0x80, 0xff]), 7);
        assert_eq!(leading_zero_bits(&[0x00, 0x04]), 10);
        assert_eq!(leading_zero_bits(&[0x00, 0x00, 0x01]), 16);
    }

    #[test]
    fn improve_reaches_target() {
        let omega = "MEsDAgcAAgEgAiBxu2eCLQf8zLnuJJ6FtbVjfaOa1210xFgedoXuGzDbTgIgcGk35eqFavKxS4dROi5uKNSNsmzIL4+fyh5Z/+FWGxU=";
        let offset = improve_security_level(omega, 0, 8, &|| false).unwrap();
        assert!(security_level(omega, offset) >= 8);
        for o in 0..offset {
            assert!(security_level(omega, o) < 8);
        }
    }
}
