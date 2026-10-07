use crate::speex::bits::Bits;
use crate::speex::math::speex_rand;
use crate::speex::modes::{Innovation, SplitCbParams};

const MAX_SUBVECT: usize = 20;

pub fn split_cb_shape_sign_unquant(exc: &mut [f32], params: &SplitCbParams, bits: &mut Bits) {
    let mut ind = [0u32; MAX_SUBVECT];
    let mut signs = [false; MAX_SUBVECT];
    if params.nb_subvect > MAX_SUBVECT || params.subvect_size * params.nb_subvect > exc.len() {
        return;
    }
    for i in 0..params.nb_subvect {
        signs[i] = params.have_sign && bits.unpack(1) != 0;
        ind[i] = bits.unpack(params.shape_bits);
    }
    for i in 0..params.nb_subvect {
        let s = if signs[i] { -1f32 } else { 1f32 };
        let start = ind[i] as usize * params.subvect_size;
        let Some(shape) = params.shape_cb.get(start..start + params.subvect_size) else {
            return;
        };
        for (j, c) in shape.iter().enumerate() {
            let v = &mut exc[params.subvect_size * i + j];
            *v = (f64::from(*v) + f64::from(s) * 0.03125 * f64::from(*c)) as f32;
        }
    }
}

pub fn noise_codebook_unquant(exc: &mut [f32], seed: &mut u32) {
    for v in exc.iter_mut() {
        *v = speex_rand(1.0, seed);
    }
}

pub fn innovation_unquant(which: Innovation, exc: &mut [f32], bits: &mut Bits, seed: &mut u32) {
    match which {
        Innovation::Split(params) => split_cb_shape_sign_unquant(exc, params, bits),
        Innovation::Noise => noise_codebook_unquant(exc, seed),
    }
}
