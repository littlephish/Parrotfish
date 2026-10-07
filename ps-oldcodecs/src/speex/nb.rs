use crate::speex::bits::Bits;
use crate::speex::cb::innovation_unquant;
use crate::speex::filters::{
    bw_lpc, bw_lpc_in_place, compute_rms16_at, highpass_in_place, iir_mem16, iir_mem16_in_place, multicomb,
    sanitize_values32, signal_mul_in_place,
};
use crate::speex::lsp::{lsp_interpolate, lsp_to_lpc, lsp_unquant};
use crate::speex::ltp::{gain_3tap_to_1tap, ltp_unquant, PITCH_END, PITCH_START};
use crate::speex::math::{speex_rand, spx_sqrt};
use crate::speex::modes::{NB_SUBMODES, WB_SKIP_TABLE};
use crate::speex::sb::Low;
use crate::Corrupt;

pub const ORDER: usize = 10;
pub const FRAME_SIZE: usize = 160;
pub const SUBFRAME_SIZE: usize = 40;
pub const SUBFRAMES: usize = 4;
pub const DEC_BUFFER: usize = FRAME_SIZE + 2 * PITCH_END as usize + SUBFRAME_SIZE + 12;
const EXC: usize = 2 * PITCH_END as usize + SUBFRAME_SIZE + 6;
const LSP_MARGIN: f32 = 0.002;
const MAX_COUNT_LOST: i32 = 1 << 20;
const SANE_LIMIT: f32 = 1e9;

const EXC_GAIN_QUANT_SCAL3: [f32; 8] =
    [0.061130, 0.163546, 0.310413, 0.428220, 0.555887, 0.719055, 0.938694, 1.326874];
const EXC_GAIN_QUANT_SCAL1: [f32; 2] = [0.70469, 1.05127];
const ATTENUATION: [f32; 10] = [1., 0.961, 0.852, 0.698, 0.527, 0.368, 0.237, 0.141, 0.077, 0.039];

fn median3(a: f32, b: f32, c: f32) -> f32 {
    if a < b {
        if b < c {
            b
        } else if a < c {
            c
        } else {
            a
        }
    } else if c < b {
        b
    } else if c < a {
        c
    } else {
        a
    }
}

#[derive(Clone)]
pub struct Nb {
    exc_buf: [f32; DEC_BUFFER],
    old_qlsp: [f32; ORDER],
    interp_qlpc: [f32; ORDER],
    mem_sp: [f32; ORDER],
    mem_hp: [f32; 2],
    pi_gain: [f32; SUBFRAMES],
    first: bool,
    count_lost: i32,
    last_pitch: i32,
    last_pitch_gain: f32,
    pitch_gain_buf: [f32; 3],
    pitch_gain_buf_idx: usize,
    seed: u32,
    submode_id: usize,
    lpc_enh_enabled: bool,
    dtx_enabled: bool,
    is_wideband: bool,
    voc_m1: f32,
    voc_m2: f32,
    voc_mean: f32,
    voc_offset: i32,
}

impl Nb {
    pub fn new(is_wideband: bool) -> Self {
        Self {
            exc_buf: [0.0; DEC_BUFFER],
            old_qlsp: [0.0; ORDER],
            interp_qlpc: [0.0; ORDER],
            mem_sp: [0.0; ORDER],
            mem_hp: [0.0; 2],
            pi_gain: [0.0; SUBFRAMES],
            first: true,
            count_lost: 0,
            last_pitch: 40,
            last_pitch_gain: 0.0,
            pitch_gain_buf: [0.0; 3],
            pitch_gain_buf_idx: 0,
            seed: 1000,
            submode_id: 5,
            lpc_enh_enabled: true,
            dtx_enabled: false,
            is_wideband,
            voc_m1: 0.0,
            voc_m2: 0.0,
            voc_mean: 0.0,
            voc_offset: 0,
        }
    }

    fn highpass_id(&self) -> usize {
        if self.is_wideband {
            3
        } else {
            1
        }
    }

    fn bump_gain_buf(&mut self, gain: f32) {
        self.pitch_gain_buf[self.pitch_gain_buf_idx] = gain;
        self.pitch_gain_buf_idx += 1;
        if self.pitch_gain_buf_idx > 2 {
            self.pitch_gain_buf_idx = 0;
        }
    }

    fn read_submode(&mut self, bits: &mut Bits) -> Result<(), Corrupt> {
        if bits.remaining() < 5 {
            return Err(Corrupt);
        }
        if bits.unpack(1) != 0 {
            let submode = bits.unpack(3) as usize;
            bits.advance(WB_SKIP_TABLE[submode & 7] - 4);
            if bits.remaining() < 5 {
                return Err(Corrupt);
            }
            if bits.unpack(1) != 0 {
                let submode = bits.unpack(3) as usize;
                bits.advance(WB_SKIP_TABLE[submode & 7] - 4);
                if bits.unpack(1) != 0 {
                    return Err(Corrupt);
                }
            }
        }
        if bits.remaining() < 4 {
            return Err(Corrupt);
        }
        let m = bits.unpack(4) as usize;
        if m > 8 {
            return Err(Corrupt);
        }
        self.submode_id = m;
        Ok(())
    }

    fn null_submode(&mut self, out: &mut [f32]) {
        let mut lpc = [0f32; ORDER];
        bw_lpc(0.93, &self.interp_qlpc, &mut lpc);
        let innov_gain = compute_rms16_at(&self.exc_buf, EXC as isize, FRAME_SIZE);
        for i in 0..FRAME_SIZE {
            self.exc_buf[EXC + i] = speex_rand(innov_gain, &mut self.seed);
        }
        self.first = true;
        iir_mem16(&self.exc_buf[EXC..EXC + FRAME_SIZE], &lpc, &mut out[..FRAME_SIZE], &mut self.mem_sp);
        self.count_lost = 0;
    }

    fn decode_frame(&mut self, bits: &mut Bits, out: &mut [f32], save_innov: bool) -> Result<(), Corrupt> {
        self.exc_buf.copy_within(FRAME_SIZE.., 0);

        let Some(submode) = NB_SUBMODES[self.submode_id & 15] else {
            self.null_submode(out);
            return Ok(());
        };

        let mut qlsp = [0f32; ORDER];
        lsp_unquant(submode.lsp_unquant, &mut qlsp, ORDER, bits);

        if self.count_lost != 0 {
            let mut lsp_dist = 0f32;
            for i in 0..ORDER {
                lsp_dist += (self.old_qlsp[i] - qlsp[i]).abs();
            }
            let fact = (0.6 * (-0.2 * f64::from(lsp_dist)).exp()) as f32;
            for mem in self.mem_sp.iter_mut() {
                *mem = fact * *mem;
            }
        }

        if self.first || self.count_lost != 0 {
            self.old_qlsp = qlsp;
        }

        let mut ol_pitch = 0i32;
        let mut ol_pitch_coef = 0f32;
        if submode.lbr_pitch != -1 {
            ol_pitch = PITCH_START + bits.unpack(7) as i32;
        }
        if submode.forced_pitch_gain {
            let quant = bits.unpack(4);
            ol_pitch_coef = (0.066667 * f64::from(quant)) as f32;
        }
        let qe = bits.unpack(5);
        let ol_gain = (f64::from(qe) / 3.5).exp() as f32;

        if self.submode_id == 1 {
            self.dtx_enabled = bits.unpack(4) == 15;
        }
        if self.submode_id > 1 {
            self.dtx_enabled = false;
        }

        let mut innov = [0f32; SUBFRAME_SIZE];
        let mut exc32 = [0f32; SUBFRAME_SIZE];
        let mut pitch_gain = [0f32; 3];
        let mut pitch_average = 0f32;
        let mut best_pitch = 40i32;
        let mut best_pitch_gain = 0f32;

        for sub in 0..SUBFRAMES {
            let offset = SUBFRAME_SIZE * sub;
            let base = EXC + offset;
            for v in self.exc_buf[base..base + SUBFRAME_SIZE].iter_mut() {
                *v = 0.0;
            }

            let pit_min = if submode.lbr_pitch != -1 {
                let margin = submode.lbr_pitch;
                if margin != 0 {
                    (ol_pitch - margin + 1).max(PITCH_START)
                } else {
                    ol_pitch
                }
            } else {
                PITCH_START
            };

            let Some(ltp) = submode.ltp else { return Err(Corrupt) };
            let mut pitch = 0i32;
            ltp_unquant(
                ltp,
                &mut self.exc_buf,
                base as isize,
                &mut exc32,
                pit_min,
                ol_pitch_coef,
                SUBFRAME_SIZE,
                &mut pitch,
                &mut pitch_gain,
                bits,
                self.count_lost,
                offset as i32,
                self.last_pitch_gain,
            );
            sanitize_values32(&mut exc32, -32000.0, 32000.0);

            let tmp = gain_3tap_to_1tap(&pitch_gain);
            pitch_average += tmp;
            let far = (2 * best_pitch - pitch).abs() >= 3
                && (3 * best_pitch - pitch).abs() >= 4
                && (4 * best_pitch - pitch).abs() >= 5;
            let multiple_of_pitch = (best_pitch - 2 * pitch).abs() < 3
                || (best_pitch - 3 * pitch).abs() < 4
                || (best_pitch - 4 * pitch).abs() < 5;
            let pitch_multiple = (2 * best_pitch - pitch).abs() < 3
                || (3 * best_pitch - pitch).abs() < 4
                || (4 * best_pitch - pitch).abs() < 5;
            if (tmp > best_pitch_gain && far)
                || (f64::from(tmp) > 0.6 * f64::from(best_pitch_gain) && multiple_of_pitch)
                || (0.67 * f64::from(tmp) > f64::from(best_pitch_gain) && pitch_multiple)
            {
                best_pitch = pitch;
                if tmp > best_pitch_gain {
                    best_pitch_gain = tmp;
                }
            }

            for v in innov.iter_mut() {
                *v = 0.0;
            }
            let ener = match submode.have_subframe_gain {
                3 => EXC_GAIN_QUANT_SCAL3[(bits.unpack(3) & 7) as usize] * ol_gain,
                1 => EXC_GAIN_QUANT_SCAL1[(bits.unpack(1) & 1) as usize] * ol_gain,
                _ => ol_gain,
            };

            let Some(innovation) = submode.innovation else { return Err(Corrupt) };
            innovation_unquant(innovation, &mut innov, bits, &mut self.seed);
            signal_mul_in_place(&mut innov, ener);
            if submode.double_codebook {
                let mut innov2 = [0f32; SUBFRAME_SIZE];
                innovation_unquant(innovation, &mut innov2, bits, &mut self.seed);
                signal_mul_in_place(&mut innov2, 0.454545f32 * ener);
                for i in 0..SUBFRAME_SIZE {
                    innov[i] += innov2[i];
                }
            }
            for i in 0..SUBFRAME_SIZE {
                self.exc_buf[base + i] = exc32[i] + innov[i];
            }
            if save_innov {
                out[FRAME_SIZE + offset..FRAME_SIZE + offset + SUBFRAME_SIZE].copy_from_slice(&innov);
            }

            if self.submode_id == 1 {
                let mut g = 1.5f32 * (ol_pitch_coef - 0.2f32);
                if g < 0.0 {
                    g = 0.0;
                }
                if g > 1.0 {
                    g = 1.0;
                }
                for v in self.exc_buf[base..base + SUBFRAME_SIZE].iter_mut() {
                    *v = 0.0;
                }
                if ol_pitch > 0 {
                    while self.voc_offset < SUBFRAME_SIZE as i32 {
                        if self.voc_offset >= 0 {
                            let mag = spx_sqrt((2 * ol_pitch) as f32);
                            self.exc_buf[base + self.voc_offset as usize] = mag * (g * ol_gain);
                        }
                        self.voc_offset += ol_pitch;
                    }
                }
                self.voc_offset -= SUBFRAME_SIZE as i32;

                for i in 0..SUBFRAME_SIZE {
                    let exci = self.exc_buf[base + i];
                    self.exc_buf[base + i] = (0.7f32 * exci + 0.3f32 * self.voc_m1)
                        + ((1.0f32 - 0.85f32 * g) * innov[i] - (0.15f32 * g) * self.voc_m2);
                    self.voc_m1 = exci;
                    self.voc_m2 = innov[i];
                    self.voc_mean = 0.8f32 * self.voc_mean + 0.2f32 * self.exc_buf[base + i];
                    self.exc_buf[base + i] -= self.voc_mean;
                }
            }
        }

        if self.lpc_enh_enabled && submode.comb_gain > 0.0 && self.count_lost == 0 {
            let comb_gain = submode.comb_gain;
            let two_sub = 2 * SUBFRAME_SIZE;
            multicomb(
                &self.exc_buf,
                (EXC - SUBFRAME_SIZE) as isize,
                &mut out[..two_sub],
                two_sub,
                best_pitch,
                SUBFRAME_SIZE as i32,
                comb_gain,
            );
            let (_, second) = out.split_at_mut(two_sub);
            multicomb(
                &self.exc_buf,
                (EXC + SUBFRAME_SIZE) as isize,
                second,
                two_sub,
                best_pitch,
                SUBFRAME_SIZE as i32,
                comb_gain,
            );
        } else {
            out[..FRAME_SIZE].copy_from_slice(&self.exc_buf[EXC - SUBFRAME_SIZE..EXC - SUBFRAME_SIZE + FRAME_SIZE]);
        }

        if self.count_lost != 0 {
            let exc_ener = compute_rms16_at(&self.exc_buf, EXC as isize, FRAME_SIZE);
            let mut gain32 = ol_gain / (exc_ener + 1f32);
            if gain32 > 2f32 {
                gain32 = 2f32;
            }
            for i in 0..FRAME_SIZE {
                self.exc_buf[EXC + i] = gain32 * self.exc_buf[EXC + i];
                out[i] = self.exc_buf[EXC - SUBFRAME_SIZE + i];
            }
        }

        let mut interp_qlsp = [0f32; ORDER];
        let mut ak = [0f32; ORDER];
        for sub in 0..SUBFRAMES {
            let offset = SUBFRAME_SIZE * sub;
            lsp_interpolate(&self.old_qlsp, &qlsp, &mut interp_qlsp, ORDER, sub, SUBFRAMES, LSP_MARGIN);
            lsp_to_lpc(&interp_qlsp, &mut ak, ORDER);
            let mut pi_g = 1f32;
            let mut i = 0;
            while i < ORDER {
                pi_g += ak[i + 1] - ak[i];
                i += 2;
            }
            self.pi_gain[sub] = pi_g;
            iir_mem16_in_place(&mut out[offset..offset + SUBFRAME_SIZE], &self.interp_qlpc, &mut self.mem_sp);
            self.interp_qlpc = ak;
        }

        highpass_in_place(&mut out[..FRAME_SIZE], self.highpass_id(), &mut self.mem_hp);

        self.old_qlsp = qlsp;
        self.first = false;
        self.count_lost = 0;
        self.last_pitch = best_pitch;
        self.last_pitch_gain = (0.25 * f64::from(pitch_average)) as f32;
        self.bump_gain_buf(self.last_pitch_gain);
        Ok(())
    }

    fn decode_lost(&mut self, out: &mut [f32]) {
        let fact = if self.count_lost < 10 { ATTENUATION[self.count_lost.clamp(0, 9) as usize] } else { 0.0 };
        let gain_med = median3(self.pitch_gain_buf[0], self.pitch_gain_buf[1], self.pitch_gain_buf[2]);
        if gain_med < self.last_pitch_gain {
            self.last_pitch_gain = gain_med;
        }
        let mut pitch_gain = self.last_pitch_gain;
        if f64::from(pitch_gain) > 0.85 {
            pitch_gain = 0.85;
        }
        pitch_gain = fact * pitch_gain + 1e-15f32;
        let innov_gain = compute_rms16_at(&self.exc_buf, EXC as isize, FRAME_SIZE);
        let noise_gain = innov_gain * (fact * (1.0f32 - pitch_gain * pitch_gain));

        self.exc_buf.copy_within(FRAME_SIZE.., 0);

        let jitter = speex_rand((1 + self.count_lost) as f32, &mut self.seed);
        let pitch_val = (i64::from(self.last_pitch) + jitter as i64).clamp(PITCH_START as i64, PITCH_END as i64);
        let pitch_val = pitch_val as usize;
        for i in 0..FRAME_SIZE {
            let past = self.exc_buf[EXC + i - pitch_val] + 1e-15f32;
            self.exc_buf[EXC + i] = pitch_gain * past + speex_rand(noise_gain, &mut self.seed);
        }

        bw_lpc_in_place(0.98, &mut self.interp_qlpc);
        let from = EXC - SUBFRAME_SIZE;
        iir_mem16(&self.exc_buf[from..from + FRAME_SIZE], &self.interp_qlpc, &mut out[..FRAME_SIZE], &mut self.mem_sp);
        highpass_in_place(&mut out[..FRAME_SIZE], 1, &mut self.mem_hp);

        self.first = false;
        self.count_lost = (self.count_lost + 1).min(MAX_COUNT_LOST);
        self.bump_gain_buf(pitch_gain);
    }
}

impl Low for Nb {
    fn decode(&mut self, bits: &mut Bits, out: &mut [f32], save_innov: bool) -> Result<(), Corrupt> {
        if out.len() < FRAME_SIZE || (save_innov && out.len() < 2 * FRAME_SIZE) {
            return Err(Corrupt);
        }
        self.read_submode(bits)?;
        self.decode_frame(bits, out, save_innov)?;
        if bits.overflowed() {
            return Err(Corrupt);
        }
        Ok(())
    }

    fn conceal(&mut self, out: &mut [f32], _save_innov: bool) {
        if out.len() < FRAME_SIZE {
            return;
        }
        if self.dtx_enabled {
            self.submode_id = 0;
            self.exc_buf.copy_within(FRAME_SIZE.., 0);
            self.null_submode(out);
            return;
        }
        self.decode_lost(out);
    }

    fn dtx(&self) -> bool {
        self.dtx_enabled
    }

    fn pi_gain(&self) -> &[f32; SUBFRAMES] {
        &self.pi_gain
    }

    fn exc_rms(&self, rms: &mut [f32; SUBFRAMES]) {
        for (sub, slot) in rms.iter_mut().enumerate() {
            let base = EXC + sub * SUBFRAME_SIZE;
            *slot = compute_rms16_at(&self.exc_buf, base as isize, SUBFRAME_SIZE);
        }
    }

    fn set_enhancement(&mut self, on: bool) {
        self.lpc_enh_enabled = on;
    }

    fn sane(&self) -> bool {
        let ok = |v: &f32| v.is_finite() && v.abs() < SANE_LIMIT;
        self.exc_buf.iter().all(ok)
            && self.old_qlsp.iter().all(ok)
            && self.interp_qlpc.iter().all(ok)
            && self.mem_sp.iter().all(ok)
            && self.mem_hp.iter().all(ok)
            && ok(&self.last_pitch_gain)
            && self.pitch_gain_buf.iter().all(ok)
            && ok(&self.voc_m1)
            && ok(&self.voc_m2)
            && ok(&self.voc_mean)
    }

    fn restart(&mut self) {
        let enhance = self.lpc_enh_enabled;
        let wide = self.is_wideband;
        *self = Nb::new(wide);
        self.lpc_enh_enabled = enhance;
    }
}
