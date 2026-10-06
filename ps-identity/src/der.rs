use crate::IdentityError;

#[derive(Debug, Clone)]
pub struct DerIdentity {
    pub bit_info: u8,
    pub x: [u8; 32],
    pub y: [u8; 32],
    pub private: Option<[u8; 32]>,
}

pub fn parse_identity_key(asn: &[u8]) -> Result<DerIdentity, IdentityError> {
    let content = seq_content(asn)?;
    let mut pos = 0usize;

    let (tag, v) = tlv(content, &mut pos)?;
    if tag != 0x03 || v.len() < 2 {
        return Err(IdentityError::Der("expected BIT STRING".into()));
    }
    let bit_info = v[1];

    let (tag, v) = tlv(content, &mut pos)?;
    if tag != 0x02 || v != [0x20] {
        return Err(IdentityError::Der(format!(
            "expected magic integer 02 01 20, got tag {tag:02x} value {v:02x?}"
        )));
    }

    let (tag, v) = tlv(content, &mut pos)?;
    if tag != 0x02 {
        return Err(IdentityError::Der("expected INTEGER x".into()));
    }
    let x = int32(v)?;

    let (tag, v) = tlv(content, &mut pos)?;
    if tag != 0x02 {
        return Err(IdentityError::Der("expected INTEGER y".into()));
    }
    let y = int32(v)?;

    let mut private = None;
    if bit_info == 0x80 && pos < content.len() {
        let (tag, v) = tlv(content, &mut pos)?;
        if tag != 0x02 {
            return Err(IdentityError::Der("expected INTEGER private key".into()));
        }
        private = Some(int32(v)?);
    }

    Ok(DerIdentity { bit_info, x, y, private })
}

pub fn encode_identity_key(x: &[u8; 32], y: &[u8; 32], private: Option<&[u8; 32]>) -> Vec<u8> {
    let bit_info = if private.is_some() { 0x80 } else { 0x00 };
    let mut body = vec![0x03, 0x02, 0x07, bit_info, 0x02, 0x01, 0x20];
    push_integer(&mut body, x);
    push_integer(&mut body, y);
    if let Some(k) = private {
        push_integer(&mut body, k);
    }
    let mut out = vec![0x30];
    if body.len() < 128 {
        out.push(body.len() as u8);
    } else {
        out.extend_from_slice(&[0x81, body.len() as u8]);
    }
    out.extend_from_slice(&body);
    out
}

pub fn push_integer(out: &mut Vec<u8>, v: &[u8; 32]) {
    let start = v.iter().position(|b| *b != 0).unwrap_or(31);
    let digits = &v[start..];
    let pad = digits[0] & 0x80 != 0;
    out.push(0x02);
    out.push((digits.len() + usize::from(pad)) as u8);
    if pad {
        out.push(0x00);
    }
    out.extend_from_slice(digits);
}

fn seq_content(asn: &[u8]) -> Result<&[u8], IdentityError> {
    if asn.len() < 2 || asn[0] != 0x30 {
        return Err(IdentityError::Der("expected SEQUENCE".into()));
    }
    let (start, len) = if asn[1] < 0x80 {
        (2usize, asn[1] as usize)
    } else if asn[1] == 0x81 {
        (
            3usize,
            *asn.get(2).ok_or_else(|| IdentityError::Der("truncated sequence".into()))? as usize,
        )
    } else {
        return Err(IdentityError::Der("unsupported sequence length form".into()));
    };
    asn.get(start..start + len)
        .ok_or_else(|| IdentityError::Der("sequence length out of bounds".into()))
}

fn tlv<'a>(buf: &'a [u8], pos: &mut usize) -> Result<(u8, &'a [u8]), IdentityError> {
    let tag = *buf
        .get(*pos)
        .ok_or_else(|| IdentityError::Der("truncated TLV".into()))?;
    let len = *buf
        .get(*pos + 1)
        .ok_or_else(|| IdentityError::Der("truncated TLV length".into()))? as usize;
    if len >= 0x80 {
        return Err(IdentityError::Der("unsupported TLV length form".into()));
    }
    *pos += 2;
    let v = buf
        .get(*pos..*pos + len)
        .ok_or_else(|| IdentityError::Der("TLV out of bounds".into()))?;
    *pos += len;
    Ok((tag, v))
}

fn int32(v: &[u8]) -> Result<[u8; 32], IdentityError> {
    let start = v.iter().position(|b| *b != 0).unwrap_or(v.len());
    let digits = &v[start..];
    if digits.len() > 32 {
        return Err(IdentityError::Der(format!(
            "integer too large: {} significant bytes",
            digits.len()
        )));
    }
    let mut out = [0u8; 32];
    out[32 - digits.len()..].copy_from_slice(digits);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integers_use_minimal_der_encoding() {
        let mut v = [0u8; 32];
        v[0] = 0x00;
        v[1] = 0x7f;
        v[31] = 0x01;
        let mut out = Vec::new();
        push_integer(&mut out, &v);
        assert_eq!(out[0], 0x02);
        assert_eq!(out[1], 31);
        assert_eq!(out[2], 0x7f);

        let mut v = [0u8; 32];
        v[0] = 0x00;
        v[1] = 0x80;
        let mut out = Vec::new();
        push_integer(&mut out, &v);
        assert_eq!(&out[..4], &[0x02, 32, 0x00, 0x80]);

        let mut v = [0u8; 32];
        v[0] = 0xff;
        let mut out = Vec::new();
        push_integer(&mut out, &v);
        assert_eq!(&out[..4], &[0x02, 33, 0x00, 0xff]);
    }

    #[test]
    fn short_integers_round_trip() {
        let mut x = [0u8; 32];
        x[2] = 0x11;
        x[31] = 0x22;
        let mut y = [0u8; 32];
        y[0] = 0x80;
        y[31] = 0x33;
        let mut k = [0u8; 32];
        k[1] = 0x01;
        let der = encode_identity_key(&x, &y, Some(&k));
        let parsed = parse_identity_key(&der).unwrap();
        assert_eq!(parsed.bit_info, 0x80);
        assert_eq!(parsed.x, x);
        assert_eq!(parsed.y, y);
        assert_eq!(parsed.private, Some(k));
    }
}
