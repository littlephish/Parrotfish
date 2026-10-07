const C1: f32 = 0.9999932946;
const C2: f32 = -0.4999124376;
const C3: f32 = 0.0414877472;
const C4: f32 = -0.0012712095;

const SPX_PI_2: f64 = 1.5707963268;

pub fn spx_cos(x: f32) -> f32 {
    if f64::from(x) < SPX_PI_2 {
        let x = x * x;
        C1 + x * (C2 + x * (C3 + C4 * x))
    } else {
        let x = (std::f64::consts::PI - f64::from(x)) as f32;
        let x = x * x;
        -(C1 + x * (C2 + x * (C3 + C4 * x)))
    }
}

pub fn spx_sqrt(x: f32) -> f32 {
    f64::from(x).sqrt() as f32
}

pub fn spx_exp(x: f32) -> f32 {
    f64::from(x).exp() as f32
}

pub fn speex_rand(std: f32, seed: &mut u32) -> f32 {
    const JFLONE: u32 = 0x3f80_0000;
    const JFLMSK: u32 = 0x007f_ffff;
    *seed = 1_664_525u32.wrapping_mul(*seed).wrapping_add(1_013_904_223);
    let ran = f32::from_bits(JFLONE | (JFLMSK & *seed));
    let ran = (f64::from(ran) - 1.5) as f32;
    (3.4642 * f64::from(std) * f64::from(ran)) as f32
}
