use crate::speex::bits::Bits;
use crate::speex::filters::at;
use crate::speex::modes::{Ltp, LtpParams};

pub const PITCH_START: i32 = 17;
pub const PITCH_END: i32 = 144;

pub fn gain_3tap_to_1tap(g: &[f32; 3]) -> f32 {
    let first = if g[0] > 0.0 { f64::from(g[0]) } else { -0.5 * f64::from(g[0]) };
    let third = if g[2] > 0.0 { f64::from(g[2]) } else { -0.5 * f64::from(g[2]) };
    (f64::from(g[1].abs()) + first + third) as f32
}

pub fn pitch_unquant_3tap(
    exc: &mut [f32],
    base: isize,
    exc_out: &mut [f32],
    start: i32,
    params: &LtpParams,
    nsf: usize,
    pitch_val: &mut i32,
    gain_val: &mut [f32; 3],
    bits: &mut Bits,
    count_lost: i32,
    subframe_offset: i32,
    last_pitch_gain: f32,
) {
    let gain_cdbk_size = 1usize << params.gain_bits;
    let pitch = bits.unpack(params.pitch_bits) as i32 + start;
    let pitch = pitch.clamp(PITCH_START, PITCH_END);
    let gain_index = bits.unpack(params.gain_bits) as usize;
    let mut gain = [0f32; 3];
    let entry = gain_index.min(gain_cdbk_size - 1) * 4;
    for (i, g) in gain.iter_mut().enumerate() {
        let c = params.gain_cdbk.get(entry + i).copied().unwrap_or(0);
        *g = (0.015625 * f64::from(c) + 0.5) as f32;
    }

    if count_lost != 0 && pitch > subframe_offset {
        let mut tmp = if count_lost < 4 { last_pitch_gain } else { (0.5 * f64::from(last_pitch_gain)) as f32 };
        if f64::from(tmp) > 0.95 {
            tmp = 0.95;
        }
        let gain_sum = gain_3tap_to_1tap(&gain);
        if gain_sum > tmp {
            let fact = tmp / gain_sum;
            for g in gain.iter_mut() {
                *g = fact * *g;
            }
        }
    }

    *pitch_val = pitch;
    gain_val.copy_from_slice(&gain);

    for v in exc_out[..nsf].iter_mut() {
        *v = 0.0;
    }
    for i in 0..3 {
        let pp = (pitch + 1 - i as i32) as isize;
        let mut tmp1 = nsf as isize;
        if tmp1 > pp {
            tmp1 = pp;
        }
        for j in 0..tmp1 {
            exc_out[j as usize] += gain[2 - i] * at(exc, base + j - pp);
        }
        let mut tmp3 = nsf as isize;
        if tmp3 > pp + pitch as isize {
            tmp3 = pp + pitch as isize;
        }
        for j in tmp1..tmp3 {
            exc_out[j as usize] += gain[2 - i] * at(exc, base + j - pp - pitch as isize);
        }
    }
}

pub fn forced_pitch_unquant(
    exc: &mut [f32],
    base: isize,
    exc_out: &mut [f32],
    start: i32,
    pitch_coef: f32,
    nsf: usize,
    pitch_val: &mut i32,
    gain_val: &mut [f32; 3],
) {
    let mut pitch_coef = pitch_coef;
    if f64::from(pitch_coef) > 0.99 {
        pitch_coef = 0.99;
    }
    let start = start.clamp(PITCH_START, PITCH_END);
    for i in 0..nsf {
        let v = at(exc, base + i as isize - start as isize) * pitch_coef;
        exc_out[i] = v;
        let Some(slot) = usize::try_from(base + i as isize).ok().and_then(|k| exc.get_mut(k)) else {
            continue;
        };
        *slot = v;
    }
    *pitch_val = start;
    gain_val[0] = 0.0;
    gain_val[2] = 0.0;
    gain_val[1] = pitch_coef;
}

pub fn ltp_unquant(
    which: Ltp,
    exc: &mut [f32],
    base: isize,
    exc_out: &mut [f32],
    pit_min: i32,
    pitch_coef: f32,
    nsf: usize,
    pitch_val: &mut i32,
    gain_val: &mut [f32; 3],
    bits: &mut Bits,
    count_lost: i32,
    subframe_offset: i32,
    last_pitch_gain: f32,
) {
    match which {
        Ltp::ThreeTap(params) => pitch_unquant_3tap(
            exc,
            base,
            exc_out,
            pit_min,
            params,
            nsf,
            pitch_val,
            gain_val,
            bits,
            count_lost,
            subframe_offset,
            last_pitch_gain,
        ),
        Ltp::Forced => {
            forced_pitch_unquant(exc, base, exc_out, pit_min, pitch_coef, nsf, pitch_val, gain_val)
        }
    }
}
