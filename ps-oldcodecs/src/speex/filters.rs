use crate::speex::math::spx_sqrt;

pub const QMF_ORDER: usize = 64;

const PCOEF: [[f32; 3]; 5] = [
    [1.00000, -1.91120, 0.91498],
    [1.00000, -1.92683, 0.93071],
    [1.00000, -1.93338, 0.93553],
    [1.00000, -1.97226, 0.97332],
    [1.00000, -1.37000, 0.39900],
];

const ZCOEF: [[f32; 3]; 5] = [
    [0.95654, -1.91309, 0.95654],
    [0.96446, -1.92879, 0.96446],
    [0.96723, -1.93445, 0.96723],
    [0.98645, -1.97277, 0.98645],
    [0.88000, -1.76000, 0.88000],
];

const SHIFT_FILT: [[f32; 7]; 3] = [
    [-0.011915, 0.046995, -0.152373, 0.614108, 0.614108, -0.152373, 0.046995],
    [-0.0324855, 0.0859768, -0.2042986, 0.9640297, 0.2086420, -0.0302054, -0.0063646],
    [-0.0063646, -0.0302054, 0.2086420, 0.9640297, -0.2042986, 0.0859768, -0.0324855],
];

pub const H0: [f32; QMF_ORDER] = [
    3.596189e-05,
    -0.0001123515,
    -0.0001104587,
    0.0002790277,
    0.0002298438,
    -0.0005953563,
    -0.0003823631,
    0.00113826,
    0.0005308539,
    -0.001986177,
    -0.0006243724,
    0.003235877,
    0.0005743159,
    -0.004989147,
    -0.0002584767,
    0.007367171,
    -0.0004857935,
    -0.01050689,
    0.001894714,
    0.01459396,
    -0.004313674,
    -0.01994365,
    0.00828756,
    0.02716055,
    -0.01485397,
    -0.03764973,
    0.026447,
    0.05543245,
    -0.05095487,
    -0.09779096,
    0.1382363,
    0.4600981,
    0.4600981,
    0.1382363,
    -0.09779096,
    -0.05095487,
    0.05543245,
    0.026447,
    -0.03764973,
    -0.01485397,
    0.02716055,
    0.00828756,
    -0.01994365,
    -0.004313674,
    0.01459396,
    0.001894714,
    -0.01050689,
    -0.0004857935,
    0.007367171,
    -0.0002584767,
    -0.004989147,
    0.0005743159,
    0.003235877,
    -0.0006243724,
    -0.001986177,
    0.0005308539,
    0.00113826,
    -0.0003823631,
    -0.0005953563,
    0.0002298438,
    0.0002790277,
    -0.0001104587,
    -0.0001123515,
    3.596189e-05,
];

pub fn at(buf: &[f32], idx: isize) -> f32 {
    if idx < 0 {
        return 0.0;
    }
    buf.get(idx as usize).copied().unwrap_or(0.0)
}

pub fn bw_lpc(gamma: f32, lpc_in: &[f32], lpc_out: &mut [f32]) {
    let mut tmp = gamma;
    for (out, inp) in lpc_out.iter_mut().zip(lpc_in) {
        *out = tmp * inp;
        tmp *= gamma;
    }
}

pub fn bw_lpc_in_place(gamma: f32, lpc: &mut [f32]) {
    let mut tmp = gamma;
    for coef in lpc.iter_mut() {
        *coef = tmp * *coef;
        tmp *= gamma;
    }
}

pub fn sanitize_values32(vec: &mut [f32], min_val: f32, max_val: f32) {
    for v in vec.iter_mut() {
        if !(*v >= min_val && *v <= max_val) {
            if *v < min_val {
                *v = min_val;
            } else if *v > max_val {
                *v = max_val;
            } else {
                *v = 0.0;
            }
        }
    }
}

pub fn compute_rms16(x: &[f32]) -> f32 {
    let mut sum = 0f32;
    for v in x {
        sum += v * v;
    }
    rms_of(sum, x.len())
}

fn rms_of(sum: f32, len: usize) -> f32 {
    (0.1 + f64::from(sum / len as f32)).sqrt() as f32
}

pub fn signal_mul_in_place(y: &mut [f32], scale: f32) {
    for v in y.iter_mut() {
        *v = scale * *v;
    }
}

fn iir_step(xi: f32, den: &[f32], mem: &mut [f32]) -> f32 {
    let Some((last, head)) = den.split_last() else { return xi };
    let yi = xi + mem[0];
    let nyi = -yi;
    for (j, d) in head.iter().enumerate() {
        mem[j] = mem[j + 1] + d * nyi;
    }
    mem[head.len()] = last * nyi;
    yi
}

pub fn iir_mem16(x: &[f32], den: &[f32], y: &mut [f32], mem: &mut [f32]) {
    for (out, xi) in y.iter_mut().zip(x) {
        *out = iir_step(*xi, den, mem);
    }
}

pub fn iir_mem16_in_place(y: &mut [f32], den: &[f32], mem: &mut [f32]) {
    for out in y.iter_mut() {
        *out = iir_step(*out, den, mem);
    }
}

pub fn highpass_in_place(y: &mut [f32], filt_id: usize, mem: &mut [f32; 2]) {
    let filt_id = filt_id.min(4);
    let den = &PCOEF[filt_id];
    let num = &ZCOEF[filt_id];
    for v in y.iter_mut() {
        let x = *v;
        let vout = num[0] * x + mem[0];
        let yi = vout;
        mem[0] = (mem[1] + num[1] * x) + -den[1] * vout;
        mem[1] = num[2] * x + -den[2] * vout;
        *v = yi;
    }
}

pub fn qmf_synth(out: &mut [f32], a: &[f32], n: usize, mem1: &mut [f32], mem2: &mut [f32]) {
    const MAX_HALF: usize = 320;
    let m2 = QMF_ORDER >> 1;
    let n2 = n >> 1;
    if n2 > MAX_HALF || n > out.len() || n2 < 2 || n2 % 2 != 0 {
        return;
    }
    let mut xx1 = [0f32; MAX_HALF + (QMF_ORDER >> 1)];
    let mut xx2 = [0f32; MAX_HALF + (QMF_ORDER >> 1)];
    for i in 0..n2 {
        xx1[i] = out[n2 - 1 - i];
    }
    for i in 0..m2 {
        xx1[n2 + i] = mem1[2 * i + 1];
    }
    for i in 0..n2 {
        xx2[i] = out[n2 + n2 - 1 - i];
    }
    for i in 0..m2 {
        xx2[n2 + i] = mem2[2 * i + 1];
    }

    let mut i = 0;
    while i < n2 {
        let mut y0 = 0f32;
        let mut y1 = 0f32;
        let mut y2 = 0f32;
        let mut y3 = 0f32;
        let mut x10 = xx1[n2 - 2 - i];
        let mut x20 = xx2[n2 - 2 - i];
        let mut j = 0;
        while j < m2 {
            let mut a0 = a[2 * j];
            let mut a1 = a[2 * j + 1];
            let x11 = xx1[n2 - 1 + j - i];
            let x21 = xx2[n2 - 1 + j - i];

            y0 += a0 * (x11 - x21);
            y1 += a1 * (x11 + x21);
            y2 += a0 * (x10 - x20);
            y3 += a1 * (x10 + x20);

            a0 = a[2 * j + 2];
            a1 = a[2 * j + 3];
            x10 = xx1[n2 + j - i];
            x20 = xx2[n2 + j - i];

            y0 += a0 * (x10 - x20);
            y1 += a1 * (x10 + x20);
            y2 += a0 * (x11 - x21);
            y3 += a1 * (x11 + x21);
            j += 2;
        }
        out[2 * i] = 2f32 * y0;
        out[2 * i + 1] = 2f32 * y1;
        out[2 * i + 2] = 2f32 * y2;
        out[2 * i + 3] = 2f32 * y3;
        i += 2;
    }

    for i in 0..m2 {
        mem1[2 * i + 1] = xx1[i];
        mem2[2 * i + 1] = xx2[i];
    }
}

pub fn inner_prod(x: &[f32], xa: isize, y: &[f32], ya: isize, len: usize) -> f32 {
    let mut sum = 0f32;
    let mut k = 0isize;
    let len = (len >> 2) as isize;
    for _ in 0..len {
        let mut part = 0f32;
        part += at(x, xa + k) * at(y, ya + k);
        part += at(x, xa + k + 1) * at(y, ya + k + 1);
        part += at(x, xa + k + 2) * at(y, ya + k + 2);
        part += at(x, xa + k + 3) * at(y, ya + k + 3);
        sum += part;
        k += 4;
    }
    sum
}

fn interp_pitch(exc: &[f32], base: isize, interp: &mut [f32], pitch: isize, len: usize) {
    let mut corr = [[0f32; 7]; 4];
    for i in 0..7 {
        corr[0][i] = inner_prod(exc, base, exc, base - pitch - 3 + i as isize, len);
    }
    for i in 0..3 {
        for j in 0..7 {
            let mut tmp = 0f32;
            let i1 = if j > 3 { 0 } else { 3 - j };
            let i2 = if 10 - j > 7 { 7 } else { 10 - j };
            for k in i1..i2 {
                tmp += SHIFT_FILT[i][k] * corr[0][j + k - 3];
            }
            corr[i + 1][j] = tmp;
        }
    }
    let mut maxi = 0usize;
    let mut maxj = 0usize;
    let mut maxcorr = corr[0][0];
    for i in 0..4 {
        for j in 0..7 {
            if corr[i][j] > maxcorr {
                maxcorr = corr[i][j];
                maxi = i;
                maxj = j;
            }
        }
    }
    for (i, out) in interp.iter_mut().enumerate().take(len) {
        let mut tmp = 0f32;
        let start = base + i as isize - (pitch - maxj as isize + 3);
        if maxi > 0 {
            for k in 0..7 {
                tmp += at(exc, start + k as isize - 3) * SHIFT_FILT[maxi - 1][k];
            }
        } else {
            tmp = at(exc, start);
        }
        *out = tmp;
    }
}

pub fn multicomb(
    exc: &[f32],
    base: isize,
    new_exc: &mut [f32],
    nsf: usize,
    pitch: i32,
    max_pitch: i32,
    comb_gain: f32,
) {
    const MAX_NSF: usize = 80;
    if nsf != MAX_NSF || nsf > new_exc.len() {
        return;
    }
    let corr_pitch = pitch as isize;
    let mut iexc = [0f32; 2 * MAX_NSF];
    let (first, second) = iexc.split_at_mut(nsf);
    interp_pitch(exc, base, first, corr_pitch, 80);
    if pitch > max_pitch {
        interp_pitch(exc, base, second, 2 * corr_pitch, 80);
    } else {
        interp_pitch(exc, base, second, -corr_pitch, 80);
    }

    let iexc0_mag = spx_sqrt(1000.0 + inner_prod(&iexc, 0, &iexc, 0, nsf));
    let iexc1_mag = spx_sqrt(1000.0 + inner_prod(&iexc, nsf as isize, &iexc, nsf as isize, nsf));
    let exc_mag = spx_sqrt(1.0 + inner_prod(exc, base, exc, base, nsf));
    let mut corr0 = inner_prod(&iexc, 0, exc, base, nsf);
    if corr0 < 0.0 {
        corr0 = 0.0;
    }
    let mut corr1 = inner_prod(&iexc, nsf as isize, exc, base, nsf);
    if corr1 < 0.0 {
        corr1 = 0.0;
    }
    let pgain1 = if corr0 > iexc0_mag * exc_mag { 1.0 } else { (corr0 / exc_mag) / iexc0_mag };
    let pgain2 = if corr1 > iexc1_mag * exc_mag { 1.0 } else { (corr1 / exc_mag) / iexc1_mag };
    let gg1 = exc_mag / iexc0_mag;
    let gg2 = exc_mag / iexc1_mag;
    let (c1, c2) = if comb_gain > 0.0 {
        let c1 = (0.4 * f64::from(comb_gain) + 0.07) as f32;
        let c2 = (0.5 + 1.72 * (f64::from(c1) - 0.07)) as f32;
        (c1, c2)
    } else {
        (0.0, 0.0)
    };
    let mut g1 = 1.0 - c2 * pgain1 * pgain1;
    let mut g2 = 1.0 - c2 * pgain2 * pgain2;
    if g1 < c1 {
        g1 = c1;
    }
    if g2 < c1 {
        g2 = c1;
    }
    g1 = c1 / g1;
    g2 = c1 / g2;
    let (gain0, gain1) = if pitch > max_pitch {
        ((0.7 * f64::from(g1 * gg1)) as f32, (0.3 * f64::from(g2 * gg2)) as f32)
    } else {
        ((0.6 * f64::from(g1 * gg1)) as f32, (0.6 * f64::from(g2 * gg2)) as f32)
    };
    for i in 0..nsf {
        new_exc[i] = at(exc, base + i as isize) + (gain0 * iexc[i] + gain1 * iexc[i + nsf]);
    }
    let new_ener = compute_rms16(&new_exc[..nsf]);
    let mut old_ener = compute_rms16_at(exc, base, nsf);
    if old_ener < 1.0 {
        old_ener = 1.0;
    }
    let new_ener = if new_ener < 1.0 { 1.0 } else { new_ener };
    if old_ener > new_ener {
        old_ener = new_ener;
    }
    let ngain = old_ener / new_ener;
    for v in new_exc[..nsf].iter_mut() {
        *v = ngain * *v;
    }
}

pub fn compute_rms16_at(x: &[f32], base: isize, len: usize) -> f32 {
    let mut sum = 0f32;
    for i in 0..len as isize {
        let v = at(x, base + i);
        sum += v * v;
    }
    rms_of(sum, len)
}
