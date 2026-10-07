use crate::speex::tables;

pub struct LtpParams {
    pub gain_cdbk: &'static [i8],
    pub gain_bits: u32,
    pub pitch_bits: u32,
}

pub struct SplitCbParams {
    pub subvect_size: usize,
    pub nb_subvect: usize,
    pub shape_cb: &'static [i8],
    pub shape_bits: u32,
    pub have_sign: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LspUnquant {
    Nb,
    Lbr,
    High,
}

#[derive(Clone, Copy)]
pub enum Ltp {
    ThreeTap(&'static LtpParams),
    Forced,
}

#[derive(Clone, Copy)]
pub enum Innovation {
    Split(&'static SplitCbParams),
    Noise,
}

pub struct Submode {
    pub lbr_pitch: i32,
    pub forced_pitch_gain: bool,
    pub have_subframe_gain: u32,
    pub double_codebook: bool,
    pub lsp_unquant: LspUnquant,
    pub ltp: Option<Ltp>,
    pub innovation: Option<Innovation>,
    pub comb_gain: f32,
}

static LTP_PARAMS_NB: LtpParams = LtpParams { gain_cdbk: &tables::GAIN_CDBK_NB, gain_bits: 7, pitch_bits: 7 };
static LTP_PARAMS_VLBR: LtpParams = LtpParams { gain_cdbk: &tables::GAIN_CDBK_LBR, gain_bits: 5, pitch_bits: 0 };
static LTP_PARAMS_LBR: LtpParams = LtpParams { gain_cdbk: &tables::GAIN_CDBK_LBR, gain_bits: 5, pitch_bits: 7 };
static LTP_PARAMS_MED: LtpParams = LtpParams { gain_cdbk: &tables::GAIN_CDBK_LBR, gain_bits: 5, pitch_bits: 7 };

static SPLIT_CB_NB_VLBR: SplitCbParams = SplitCbParams {
    subvect_size: 10,
    nb_subvect: 4,
    shape_cb: &tables::EXC_10_16_TABLE,
    shape_bits: 4,
    have_sign: false,
};

static SPLIT_CB_NB_ULBR: SplitCbParams = SplitCbParams {
    subvect_size: 20,
    nb_subvect: 2,
    shape_cb: &tables::EXC_20_32_TABLE,
    shape_bits: 5,
    have_sign: false,
};

static SPLIT_CB_NB_LBR: SplitCbParams = SplitCbParams {
    subvect_size: 10,
    nb_subvect: 4,
    shape_cb: &tables::EXC_10_32_TABLE,
    shape_bits: 5,
    have_sign: false,
};

static SPLIT_CB_NB: SplitCbParams = SplitCbParams {
    subvect_size: 5,
    nb_subvect: 8,
    shape_cb: &tables::EXC_5_64_TABLE,
    shape_bits: 6,
    have_sign: false,
};

static SPLIT_CB_NB_MED: SplitCbParams = SplitCbParams {
    subvect_size: 8,
    nb_subvect: 5,
    shape_cb: &tables::EXC_8_128_TABLE,
    shape_bits: 7,
    have_sign: false,
};

static SPLIT_CB_SB: SplitCbParams = SplitCbParams {
    subvect_size: 5,
    nb_subvect: 8,
    shape_cb: &tables::EXC_5_256_TABLE,
    shape_bits: 8,
    have_sign: false,
};

static SPLIT_CB_HIGH: SplitCbParams = SplitCbParams {
    subvect_size: 8,
    nb_subvect: 5,
    shape_cb: &tables::HEXC_TABLE,
    shape_bits: 7,
    have_sign: true,
};

static SPLIT_CB_HIGH_LBR: SplitCbParams = SplitCbParams {
    subvect_size: 10,
    nb_subvect: 4,
    shape_cb: &tables::HEXC_10_32_TABLE,
    shape_bits: 5,
    have_sign: false,
};

static NB_SUBMODE1: Submode = Submode {
    lbr_pitch: 0,
    forced_pitch_gain: true,
    have_subframe_gain: 0,
    double_codebook: false,
    lsp_unquant: LspUnquant::Lbr,
    ltp: Some(Ltp::Forced),
    innovation: Some(Innovation::Noise),
    comb_gain: -1.0,
};

static NB_SUBMODE2: Submode = Submode {
    lbr_pitch: 0,
    forced_pitch_gain: false,
    have_subframe_gain: 0,
    double_codebook: false,
    lsp_unquant: LspUnquant::Lbr,
    ltp: Some(Ltp::ThreeTap(&LTP_PARAMS_VLBR)),
    innovation: Some(Innovation::Split(&SPLIT_CB_NB_VLBR)),
    comb_gain: 0.6,
};

static NB_SUBMODE3: Submode = Submode {
    lbr_pitch: -1,
    forced_pitch_gain: false,
    have_subframe_gain: 1,
    double_codebook: false,
    lsp_unquant: LspUnquant::Lbr,
    ltp: Some(Ltp::ThreeTap(&LTP_PARAMS_LBR)),
    innovation: Some(Innovation::Split(&SPLIT_CB_NB_LBR)),
    comb_gain: 0.55,
};

static NB_SUBMODE4: Submode = Submode {
    lbr_pitch: -1,
    forced_pitch_gain: false,
    have_subframe_gain: 1,
    double_codebook: false,
    lsp_unquant: LspUnquant::Lbr,
    ltp: Some(Ltp::ThreeTap(&LTP_PARAMS_MED)),
    innovation: Some(Innovation::Split(&SPLIT_CB_NB_MED)),
    comb_gain: 0.45,
};

static NB_SUBMODE5: Submode = Submode {
    lbr_pitch: -1,
    forced_pitch_gain: false,
    have_subframe_gain: 3,
    double_codebook: false,
    lsp_unquant: LspUnquant::Nb,
    ltp: Some(Ltp::ThreeTap(&LTP_PARAMS_NB)),
    innovation: Some(Innovation::Split(&SPLIT_CB_NB)),
    comb_gain: 0.25,
};

static NB_SUBMODE6: Submode = Submode {
    lbr_pitch: -1,
    forced_pitch_gain: false,
    have_subframe_gain: 3,
    double_codebook: false,
    lsp_unquant: LspUnquant::Nb,
    ltp: Some(Ltp::ThreeTap(&LTP_PARAMS_NB)),
    innovation: Some(Innovation::Split(&SPLIT_CB_SB)),
    comb_gain: 0.15,
};

static NB_SUBMODE7: Submode = Submode {
    lbr_pitch: -1,
    forced_pitch_gain: false,
    have_subframe_gain: 3,
    double_codebook: true,
    lsp_unquant: LspUnquant::Nb,
    ltp: Some(Ltp::ThreeTap(&LTP_PARAMS_NB)),
    innovation: Some(Innovation::Split(&SPLIT_CB_NB)),
    comb_gain: 0.05,
};

static NB_SUBMODE8: Submode = Submode {
    lbr_pitch: 0,
    forced_pitch_gain: true,
    have_subframe_gain: 0,
    double_codebook: false,
    lsp_unquant: LspUnquant::Lbr,
    ltp: Some(Ltp::Forced),
    innovation: Some(Innovation::Split(&SPLIT_CB_NB_ULBR)),
    comb_gain: 0.5,
};

pub static NB_SUBMODES: [Option<&'static Submode>; 16] = [
    None,
    Some(&NB_SUBMODE1),
    Some(&NB_SUBMODE2),
    Some(&NB_SUBMODE3),
    Some(&NB_SUBMODE4),
    Some(&NB_SUBMODE5),
    Some(&NB_SUBMODE6),
    Some(&NB_SUBMODE7),
    Some(&NB_SUBMODE8),
    None,
    None,
    None,
    None,
    None,
    None,
    None,
];

static WB_SUBMODE1: Submode = Submode {
    lbr_pitch: 0,
    forced_pitch_gain: false,
    have_subframe_gain: 1,
    double_codebook: false,
    lsp_unquant: LspUnquant::High,
    ltp: None,
    innovation: None,
    comb_gain: -1.0,
};

static WB_SUBMODE2: Submode = Submode {
    lbr_pitch: 0,
    forced_pitch_gain: false,
    have_subframe_gain: 1,
    double_codebook: false,
    lsp_unquant: LspUnquant::High,
    ltp: None,
    innovation: Some(Innovation::Split(&SPLIT_CB_HIGH_LBR)),
    comb_gain: -1.0,
};

static WB_SUBMODE3: Submode = Submode {
    lbr_pitch: 0,
    forced_pitch_gain: false,
    have_subframe_gain: 1,
    double_codebook: false,
    lsp_unquant: LspUnquant::High,
    ltp: None,
    innovation: Some(Innovation::Split(&SPLIT_CB_HIGH)),
    comb_gain: -1.0,
};

static WB_SUBMODE4: Submode = Submode {
    lbr_pitch: 0,
    forced_pitch_gain: false,
    have_subframe_gain: 1,
    double_codebook: true,
    lsp_unquant: LspUnquant::High,
    ltp: None,
    innovation: Some(Innovation::Split(&SPLIT_CB_HIGH)),
    comb_gain: -1.0,
};

pub struct SbMode {
    pub frame_size: usize,
    pub subframe_size: usize,
    pub nb_subframes: usize,
    pub lpc_size: usize,
    pub folding_gain: f32,
    pub submodes: [Option<&'static Submode>; 8],
}

pub static WB_MODE: SbMode = SbMode {
    frame_size: 160,
    subframe_size: 40,
    nb_subframes: 4,
    lpc_size: 8,
    folding_gain: 0.9,
    submodes: [
        None,
        Some(&WB_SUBMODE1),
        Some(&WB_SUBMODE2),
        Some(&WB_SUBMODE3),
        Some(&WB_SUBMODE4),
        None,
        None,
        None,
    ],
};

pub static UWB_MODE: SbMode = SbMode {
    frame_size: 320,
    subframe_size: 80,
    nb_subframes: 4,
    lpc_size: 8,
    folding_gain: 0.7,
    submodes: [None, Some(&WB_SUBMODE1), None, None, None, None, None, None],
};

pub const WB_SKIP_TABLE: [i32; 8] = [0, 36, 112, 192, 352, 0, 0, 0];

#[cfg(test)]
mod tests {
    use super::*;

    const SPLIT_PARAMS: [&SplitCbParams; 8] = [
        &SPLIT_CB_NB_VLBR,
        &SPLIT_CB_NB_ULBR,
        &SPLIT_CB_NB_LBR,
        &SPLIT_CB_NB,
        &SPLIT_CB_NB_MED,
        &SPLIT_CB_SB,
        &SPLIT_CB_HIGH,
        &SPLIT_CB_HIGH_LBR,
    ];

    #[test]
    fn every_codebook_holds_a_row_for_every_index_a_frame_can_name() {
        for params in SPLIT_PARAMS {
            assert_eq!(params.shape_cb.len(), params.subvect_size << params.shape_bits);
        }
        for params in [&LTP_PARAMS_NB, &LTP_PARAMS_VLBR, &LTP_PARAMS_LBR, &LTP_PARAMS_MED] {
            assert_eq!(params.gain_cdbk.len(), 4 << params.gain_bits);
        }
    }

    #[test]
    fn every_codebook_fills_exactly_one_subframe() {
        for params in SPLIT_PARAMS {
            assert_eq!(params.subvect_size * params.nb_subvect, 40);
        }
    }

    #[test]
    fn every_submode_has_the_parts_its_decoder_asks_for() {
        for submode in NB_SUBMODES.iter().flatten() {
            assert!(submode.ltp.is_some());
            assert!(submode.innovation.is_some());
            assert!(submode.lbr_pitch == -1 || submode.lbr_pitch == 0);
        }
        for mode in [&WB_MODE, &UWB_MODE] {
            for submode in mode.submodes.iter().flatten() {
                assert!(submode.ltp.is_none());
                assert!(matches!(submode.lsp_unquant, LspUnquant::High));
            }
        }
        assert!(UWB_MODE.submodes[1].and_then(|s| s.innovation).is_none());
    }

    #[test]
    fn a_pitch_a_frame_can_name_stays_inside_the_excitation_buffer() {
        for submode in NB_SUBMODES.iter().flatten() {
            let Some(Ltp::ThreeTap(params)) = submode.ltp else { continue };
            let start = if submode.lbr_pitch == -1 { 17 } else { 144 };
            assert!(start + (1i32 << params.pitch_bits) - 1 <= 144);
        }
    }
}
