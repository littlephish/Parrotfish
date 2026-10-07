use crate::speex::bits::Bits;
use crate::speex::math::spx_cos;
use crate::speex::modes::LspUnquant;
use crate::speex::tables;

pub const MAX_ORDER: usize = 10;

pub fn lsp_to_lpc(freq: &[f32], ak: &mut [f32], lpcrdr: usize) {
    let m = lpcrdr >> 1;
    if m == 0 || lpcrdr > MAX_ORDER || ak.len() < lpcrdr || freq.len() < lpcrdr {
        return;
    }
    let mut wp = [0f32; 4 * (MAX_ORDER >> 1) + 2];
    let mut x_freq = [0f32; MAX_ORDER];
    for i in 0..lpcrdr {
        x_freq[i] = spx_cos(freq[i]);
    }

    let mut xin1 = 1.0f32;
    let mut xin2 = 1.0f32;
    for j in 0..=lpcrdr {
        let mut i2 = 0;
        for i in 0..m {
            let n1 = i * 4;
            let xout1 = xin1 - 2f32 * x_freq[i2] * wp[n1] + wp[n1 + 1];
            let xout2 = xin2 - 2f32 * x_freq[i2 + 1] * wp[n1 + 2] + wp[n1 + 3];
            wp[n1 + 1] = wp[n1];
            wp[n1 + 3] = wp[n1 + 2];
            wp[n1] = xin1;
            wp[n1 + 2] = xin2;
            xin1 = xout1;
            xin2 = xout2;
            i2 += 2;
        }
        let last = (m - 1) * 4 + 3;
        let xout1 = xin1 + wp[last + 1];
        let xout2 = xin2 - wp[last + 2];
        if j > 0 {
            ak[j - 1] = (xout1 + xout2) * 0.5f32;
        }
        wp[last + 1] = xin1;
        wp[last + 2] = xin2;
        xin1 = 0.0;
        xin2 = 0.0;
    }
}

pub fn lsp_interpolate(
    old_lsp: &[f32],
    new_lsp: &[f32],
    lsp: &mut [f32],
    len: usize,
    subframe: usize,
    nb_subframes: usize,
    margin: f32,
) {
    if len < 2 || lsp.len() < len || old_lsp.len() < len || new_lsp.len() < len {
        return;
    }
    let tmp = (1.0f32 + subframe as f32) / nb_subframes as f32;
    for i in 0..len {
        lsp[i] = (1f32 - tmp) * old_lsp[i] + tmp * new_lsp[i];
    }
    if lsp[0] < margin {
        lsp[0] = margin;
    }
    let top = (std::f64::consts::PI - f64::from(margin)) as f32;
    if f64::from(lsp[len - 1]) > std::f64::consts::PI - f64::from(margin) {
        lsp[len - 1] = top;
    }
    for i in 1..len - 1 {
        if lsp[i] < lsp[i - 1] + margin {
            lsp[i] = lsp[i - 1] + margin;
        }
        if lsp[i] > lsp[i + 1] - margin {
            lsp[i] = 0.5f32 * (lsp[i] + lsp[i + 1] - margin);
        }
    }
}

fn lsp_div_256(x: i8) -> f64 {
    0.0039062 * f64::from(x)
}

fn lsp_div_512(x: i8) -> f64 {
    0.0019531 * f64::from(x)
}

fn lsp_div_1024(x: i8) -> f64 {
    0.00097656 * f64::from(x)
}

fn add(lsp: &mut f32, delta: f64) {
    *lsp = (f64::from(*lsp) + delta) as f32;
}

fn row(cdbk: &[i8], id: u32, dim: usize) -> &[i8] {
    let start = id as usize * dim;
    cdbk.get(start..start + dim).unwrap_or(&[])
}

pub fn lsp_unquant_nb(lsp: &mut [f32], order: usize, bits: &mut Bits) {
    if order != 10 || lsp.len() < 10 {
        return;
    }
    for i in 0..order {
        lsp[i] = (0.25 * i as f64 + 0.25) as f32;
    }

    let id = bits.unpack(6);
    for (i, c) in row(&tables::CDBK_NB, id, 10).iter().enumerate() {
        add(&mut lsp[i], lsp_div_256(*c));
    }

    let id = bits.unpack(6);
    for (i, c) in row(&tables::CDBK_NB_LOW1, id, 5).iter().enumerate() {
        add(&mut lsp[i], lsp_div_512(*c));
    }

    let id = bits.unpack(6);
    for (i, c) in row(&tables::CDBK_NB_LOW2, id, 5).iter().enumerate() {
        add(&mut lsp[i], lsp_div_1024(*c));
    }

    let id = bits.unpack(6);
    for (i, c) in row(&tables::CDBK_NB_HIGH1, id, 5).iter().enumerate() {
        add(&mut lsp[i + 5], lsp_div_512(*c));
    }

    let id = bits.unpack(6);
    for (i, c) in row(&tables::CDBK_NB_HIGH2, id, 5).iter().enumerate() {
        add(&mut lsp[i + 5], lsp_div_1024(*c));
    }
}

pub fn lsp_unquant_lbr(lsp: &mut [f32], order: usize, bits: &mut Bits) {
    if order != 10 || lsp.len() < 10 {
        return;
    }
    for i in 0..order {
        lsp[i] = (0.25 * i as f64 + 0.25) as f32;
    }

    let id = bits.unpack(6);
    for (i, c) in row(&tables::CDBK_NB, id, 10).iter().enumerate() {
        add(&mut lsp[i], lsp_div_256(*c));
    }

    let id = bits.unpack(6);
    for (i, c) in row(&tables::CDBK_NB_LOW1, id, 5).iter().enumerate() {
        add(&mut lsp[i], lsp_div_512(*c));
    }

    let id = bits.unpack(6);
    for (i, c) in row(&tables::CDBK_NB_HIGH1, id, 5).iter().enumerate() {
        add(&mut lsp[i + 5], lsp_div_512(*c));
    }
}

pub fn lsp_unquant_high(lsp: &mut [f32], order: usize, bits: &mut Bits) {
    if order != 8 || lsp.len() < 8 {
        return;
    }
    for i in 0..order {
        lsp[i] = (0.3125 * i as f64 + 0.75) as f32;
    }

    let id = bits.unpack(6);
    for (i, c) in row(&tables::HIGH_LSP_CDBK, id, order).iter().enumerate() {
        add(&mut lsp[i], lsp_div_256(*c));
    }

    let id = bits.unpack(6);
    for (i, c) in row(&tables::HIGH_LSP_CDBK2, id, order).iter().enumerate() {
        add(&mut lsp[i], lsp_div_512(*c));
    }
}

pub fn lsp_unquant(which: LspUnquant, lsp: &mut [f32], order: usize, bits: &mut Bits) {
    match which {
        LspUnquant::Nb => lsp_unquant_nb(lsp, order, bits),
        LspUnquant::Lbr => lsp_unquant_lbr(lsp, order, bits),
        LspUnquant::High => lsp_unquant_high(lsp, order, bits),
    }
}
