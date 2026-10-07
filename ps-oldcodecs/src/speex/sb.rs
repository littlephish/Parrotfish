use crate::speex::bits::Bits;
use crate::speex::cb::innovation_unquant;
use crate::speex::filters::{
    bw_lpc_in_place, compute_rms16, iir_mem16, iir_mem16_in_place, qmf_synth, signal_mul_in_place, H0, QMF_ORDER,
};
use crate::speex::lsp::{lsp_interpolate, lsp_to_lpc, lsp_unquant};
use crate::speex::math::{speex_rand, spx_exp, spx_sqrt};
use crate::speex::modes::SbMode;
use crate::Corrupt;

pub const SUBFRAMES: usize = 4;
const MAX_LPC: usize = 8;
const MAX_SUBFRAME: usize = 80;
const LSP_MARGIN: f32 = 0.05;
const SB_SUBMODE_BITS: u32 = 3;
const SANE_LIMIT: f32 = 1e9;

const GC_QUANT_BOUND: [f32; 16] = [
    0.97979, 1.28384, 1.68223, 2.20426, 2.88829, 3.78458, 4.95900, 6.49787, 8.51428, 11.15642, 14.61846, 19.15484,
    25.09895, 32.88761, 43.09325, 56.46588,
];

pub trait Low: Clone {
    fn decode(&mut self, bits: &mut Bits, out: &mut [f32], save_innov: bool) -> Result<(), Corrupt>;
    fn conceal(&mut self, out: &mut [f32], save_innov: bool);
    fn dtx(&self) -> bool;
    fn pi_gain(&self) -> &[f32; SUBFRAMES];
    fn exc_rms(&self, rms: &mut [f32; SUBFRAMES]);
    fn set_enhancement(&mut self, on: bool);
    fn sane(&self) -> bool;
    fn restart(&mut self);
}

#[derive(Clone)]
pub struct Sb<L: Low> {
    low: L,
    mode: &'static SbMode,
    g0_mem: [f32; QMF_ORDER],
    g1_mem: [f32; QMF_ORDER],
    exc_buf: [f32; MAX_SUBFRAME],
    old_qlsp: [f32; MAX_LPC],
    interp_qlpc: [f32; MAX_LPC],
    mem_sp: [f32; 2 * MAX_LPC],
    pi_gain: [f32; SUBFRAMES],
    exc_rms: [f32; SUBFRAMES],
    last_ener: f32,
    seed: u32,
    submode_id: usize,
    first: bool,
}

impl<L: Low> Sb<L> {
    pub fn new(low: L, mode: &'static SbMode) -> Self {
        Self {
            low,
            mode,
            g0_mem: [0.0; QMF_ORDER],
            g1_mem: [0.0; QMF_ORDER],
            exc_buf: [0.0; MAX_SUBFRAME],
            old_qlsp: [0.0; MAX_LPC],
            interp_qlpc: [0.0; MAX_LPC],
            mem_sp: [0.0; 2 * MAX_LPC],
            pi_gain: [0.0; SUBFRAMES],
            exc_rms: [0.0; SUBFRAMES],
            last_ener: 0.0,
            seed: 1000,
            submode_id: 1,
            first: true,
        }
    }

    fn full_frame_size(&self) -> usize {
        2 * self.mode.frame_size
    }

    fn decode_lost(&mut self, out: &mut [f32], dtx: bool) {
        let frame = self.mode.frame_size;
        let lpc = self.mode.lpc_size;
        if !dtx {
            bw_lpc_in_place(0.99, &mut self.interp_qlpc[..lpc]);
        }
        self.first = true;
        if !dtx {
            self.last_ener = 0.9f32 * self.last_ener;
        }
        for v in out[frame..frame + frame].iter_mut() {
            *v = speex_rand(self.last_ener, &mut self.seed);
        }
        iir_mem16_in_place(&mut out[frame..frame + frame], &self.interp_qlpc[..lpc], &mut self.mem_sp);
        qmf_synth(out, &H0, self.full_frame_size(), &mut self.g0_mem, &mut self.g1_mem);
    }

    fn null_submode(&mut self, out: &mut [f32]) {
        let frame = self.mode.frame_size;
        let lpc = self.mode.lpc_size;
        for v in out[frame..frame + frame].iter_mut() {
            *v = 1e-15;
        }
        self.first = true;
        iir_mem16_in_place(&mut out[frame..frame + frame], &self.interp_qlpc[..lpc], &mut self.mem_sp);
        qmf_synth(out, &H0, self.full_frame_size(), &mut self.g0_mem, &mut self.g1_mem);
    }

    fn decode_high(&mut self, bits: &mut Bits, out: &mut [f32], save_innov: bool) -> Result<(), Corrupt> {
        let frame = self.mode.frame_size;
        let full = 2 * frame;
        let lpc = self.mode.lpc_size;
        let nsf = self.mode.subframe_size;
        if self.mode.nb_subframes != SUBFRAMES || nsf > MAX_SUBFRAME || lpc > MAX_LPC || nsf % 2 != 0 {
            return Err(Corrupt);
        }

        let mut low_pi_gain = [0f32; SUBFRAMES];
        let mut low_exc_rms = [0f32; SUBFRAMES];
        low_pi_gain.copy_from_slice(self.low.pi_gain());
        self.low.exc_rms(&mut low_exc_rms);

        let Some(submode) = self.mode.submodes[self.submode_id & 7] else { return Err(Corrupt) };

        let mut qlsp = [0f32; MAX_LPC];
        let mut interp_qlsp = [0f32; MAX_LPC];
        lsp_unquant(submode.lsp_unquant, &mut qlsp[..lpc], lpc, bits);

        if self.first {
            self.old_qlsp = qlsp;
        }

        let mut ak = [0f32; MAX_LPC];
        let mut exc = [0f32; MAX_SUBFRAME];
        let mut exc_ener_sum = 0f32;

        for sub in 0..SUBFRAMES {
            let offset = nsf * sub;
            if save_innov {
                for v in out[full + 2 * offset..full + 2 * offset + 2 * nsf].iter_mut() {
                    *v = 0.0;
                }
            }

            let old = &self.old_qlsp[..lpc];
            lsp_interpolate(old, &qlsp[..lpc], &mut interp_qlsp[..lpc], lpc, sub, SUBFRAMES, LSP_MARGIN);
            lsp_to_lpc(&interp_qlsp[..lpc], &mut ak[..lpc], lpc);

            let mut pi_gain = 1f32;
            let mut rh = 1f32;
            let mut i = 0;
            while i + 1 < lpc {
                rh += ak[i + 1] - ak[i];
                pi_gain += ak[i] + ak[i + 1];
                i += 2;
            }
            self.pi_gain[sub] = pi_gain;
            let rl = low_pi_gain[sub];
            let filter_ratio = ((f64::from(rl) + 0.01) / (f64::from(rh) + 0.01)) as f32;

            for v in exc[..nsf].iter_mut() {
                *v = 0.0;
            }
            match submode.innovation {
                None => {
                    let quant = bits.unpack(5) as i32;
                    let g = spx_exp(0.125f32 * (quant - 10) as f32);
                    let g = g / filter_ratio;
                    let mut i = 0;
                    while i + 1 < nsf {
                        let low0 = out[frame + offset + i];
                        let low1 = out[frame + offset + i + 1];
                        exc[i] = (self.mode.folding_gain * low0) * g;
                        exc[i + 1] = -((self.mode.folding_gain * low1) * g);
                        i += 2;
                    }
                }
                Some(innovation) => {
                    let qgc = (bits.unpack(4) & 15) as usize;
                    let el = low_exc_rms[sub];
                    let mut gc = (0.87360 * f64::from(GC_QUANT_BOUND[qgc])) as f32;
                    if nsf == 80 {
                        gc = 1.4142f32 * gc;
                    }
                    let scale = (gc * el) / filter_ratio;
                    innovation_unquant(innovation, &mut exc[..nsf], bits, &mut self.seed);
                    signal_mul_in_place(&mut exc[..nsf], scale);
                    if submode.double_codebook {
                        let mut innov2 = [0f32; MAX_SUBFRAME];
                        innovation_unquant(innovation, &mut innov2[..nsf], bits, &mut self.seed);
                        signal_mul_in_place(&mut innov2[..nsf], 0.4f32 * scale);
                        for i in 0..nsf {
                            exc[i] += innov2[i];
                        }
                    }
                }
            }

            if save_innov {
                for i in 0..nsf {
                    out[full + 2 * offset + 2 * i] = exc[i];
                }
            }

            iir_mem16(
                &self.exc_buf[..nsf],
                &self.interp_qlpc[..lpc],
                &mut out[frame + offset..frame + offset + nsf],
                &mut self.mem_sp,
            );
            self.exc_buf[..nsf].copy_from_slice(&exc[..nsf]);
            self.interp_qlpc[..lpc].copy_from_slice(&ak[..lpc]);
            self.exc_rms[sub] = compute_rms16(&self.exc_buf[..nsf]);
            let rms = self.exc_rms[sub];
            exc_ener_sum += (rms * rms) / SUBFRAMES as f32;
        }
        self.last_ener = spx_sqrt(exc_ener_sum);

        qmf_synth(out, &H0, full, &mut self.g0_mem, &mut self.g1_mem);
        self.old_qlsp = qlsp;
        self.first = false;
        Ok(())
    }
}

impl<L: Low> Low for Sb<L> {
    fn decode(&mut self, bits: &mut Bits, out: &mut [f32], save_innov: bool) -> Result<(), Corrupt> {
        let full = self.full_frame_size();
        if out.len() < full || (save_innov && out.len() < 2 * full) {
            return Err(Corrupt);
        }
        self.low.decode(bits, &mut out[..full], true)?;
        let dtx = self.low.dtx();

        let wideband = if bits.remaining() > 0 { bits.peek() } else { 0 };
        if wideband != 0 {
            bits.unpack(1);
            self.submode_id = bits.unpack(SB_SUBMODE_BITS) as usize;
        } else {
            self.submode_id = 0;
        }
        if self.submode_id != 0 && self.mode.submodes[self.submode_id & 7].is_none() {
            return Err(Corrupt);
        }

        if self.mode.submodes[self.submode_id & 7].is_none() {
            if dtx {
                self.decode_lost(out, true);
            } else {
                self.null_submode(out);
            }
        } else {
            self.decode_high(bits, out, save_innov)?;
        }
        if bits.overflowed() {
            return Err(Corrupt);
        }
        Ok(())
    }

    fn conceal(&mut self, out: &mut [f32], _save_innov: bool) {
        let full = self.full_frame_size();
        if out.len() < full {
            return;
        }
        self.low.conceal(&mut out[..full], true);
        let dtx = self.low.dtx();
        self.decode_lost(out, dtx);
    }

    fn dtx(&self) -> bool {
        self.low.dtx()
    }

    fn pi_gain(&self) -> &[f32; SUBFRAMES] {
        &self.pi_gain
    }

    fn exc_rms(&self, rms: &mut [f32; SUBFRAMES]) {
        *rms = self.exc_rms;
    }

    fn set_enhancement(&mut self, on: bool) {
        self.low.set_enhancement(on);
    }

    fn sane(&self) -> bool {
        let ok = |v: &f32| v.is_finite() && v.abs() < SANE_LIMIT;
        self.low.sane()
            && self.g0_mem.iter().all(ok)
            && self.g1_mem.iter().all(ok)
            && self.exc_buf.iter().all(ok)
            && self.old_qlsp.iter().all(ok)
            && self.interp_qlpc.iter().all(ok)
            && self.mem_sp.iter().all(ok)
            && ok(&self.last_ener)
    }

    fn restart(&mut self) {
        self.low.restart();
        self.g0_mem = [0.0; QMF_ORDER];
        self.g1_mem = [0.0; QMF_ORDER];
        self.exc_buf = [0.0; MAX_SUBFRAME];
        self.old_qlsp = [0.0; MAX_LPC];
        self.interp_qlpc = [0.0; MAX_LPC];
        self.mem_sp = [0.0; 2 * MAX_LPC];
        self.pi_gain = [0.0; SUBFRAMES];
        self.exc_rms = [0.0; SUBFRAMES];
        self.last_ener = 0.0;
        self.seed = 1000;
        self.submode_id = 1;
        self.first = true;
    }
}
