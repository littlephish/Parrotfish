use p256::elliptic_curve::sec1::{FromEncodedPoint, ToEncodedPoint};
use p256::{AffinePoint, EncodedPoint, FieldBytes, SecretKey};

pub fn is_on_curve(x: &[u8; 32], y: &[u8; 32]) -> bool {
    let ep = EncodedPoint::from_affine_coordinates(
        &FieldBytes::from(*x),
        &FieldBytes::from(*y),
        false,
    );
    AffinePoint::from_encoded_point(&ep).is_some().into()
}

pub fn scalar_mul_g(k: &[u8; 32]) -> Option<([u8; 32], [u8; 32])> {
    let sk = SecretKey::from_bytes(&FieldBytes::from(*k)).ok()?;
    let ep = sk.public_key().to_encoded_point(false);
    let x = <[u8; 32]>::try_from(&**ep.x()?).ok()?;
    let y = <[u8; 32]>::try_from(&**ep.y()?).ok()?;
    Some((x, y))
}
