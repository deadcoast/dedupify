//! The TMK descriptor (Temporal Matching Kernel on PDQ float features —
//! Poullot et al. 2015; Meta's `tmkfv.cpp` for the constants) and its two
//! scores.

use super::FPS;

/// TMK: periods in frames at 15 fps (Poullot's list) and the 32 Fourier
/// coefficients a_j = I_j(β)/sinh(β), a_0 = (I_0(β) − e^−β)/(2 sinh β), β = 32,
/// which make Σ a_j cos(jδ) a von Mises kernel in the time offset δ.
pub const PERIODS: [u32; 4] = [2731, 4391, 9767, 14653];

/// Copied digit for digit from the reference table.
#[allow(clippy::excessive_precision)]
pub const COEFFS: [f32; 32] = [
    0.0708041893112,
    0.13937789309,
    0.132897260304,
    0.122765735552,
    0.109878684888,
    0.09529606433,
    0.0800986647852,
    0.0652590650356,
    0.0515478238322,
    0.0394851531195,
    0.0293374252025,
    0.0211492623679,
    0.0147973073245,
    0.0100512818746,
    0.0066306408014,
    0.00424947117334,
    0.0026467615764,
    0.00160270959695,
    0.000943882629639,
    0.000540841638603,
    0.000301633183798,
    0.000163800158855,
    8.66454753015e-05,
    4.46626303151e-05,
    2.24429442235e-05,
    1.09982139799e-05,
    5.25823487999e-06,
    2.45358229988e-06,
    1.11781474895e-06,
    4.97406489221e-07,
    2.16265487234e-07,
    9.19087006565e-08,
];

pub const DIM: usize = 256;

#[derive(Debug, Clone, PartialEq)]
pub struct Tmk {
    /// Mean raw PDQ float feature, DIM values.
    pub pure: Vec<f32>,
    /// cos[(period·32 + j)·DIM + k], normalised and scaled by √a_j.
    pub cos: Vec<f32>,
    pub sin: Vec<f32>,
    pub frames: u32,
}

pub struct TmkBuilder {
    pure: Vec<f32>,
    cos: Vec<f32>,
    sin: Vec<f32>,
    frames: u32,
}

impl TmkBuilder {
    pub fn new() -> TmkBuilder {
        let n = PERIODS.len() * COEFFS.len() * DIM;
        TmkBuilder {
            pure: vec![0.0; DIM],
            cos: vec![0.0; n],
            sin: vec![0.0; n],
            frames: 0,
        }
    }

    /// Ingest frame `t` (index at FPS) with its raw PDQ float feature.
    pub fn ingest(&mut self, t: u32, feature: &[f32; DIM]) {
        for (p, f) in self.pure.iter_mut().zip(feature) {
            *p += *f;
        }
        let norm = feature.iter().map(|x| x * x).sum::<f32>().sqrt();
        let inv = if norm > 0.0 { 1.0 / norm } else { 0.0 };
        let mut unit = [0.0f32; DIM];
        for (u, f) in unit.iter_mut().zip(feature) {
            *u = f * inv;
        }
        for (pi, period) in PERIODS.iter().enumerate() {
            let theta = std::f64::consts::TAU * t as f64 / *period as f64;
            let (s1, c1) = theta.sin_cos();
            // cos(jθ), sin(jθ) by rotation
            let mut cj = 1.0f64;
            let mut sj = 0.0f64;
            for j in 0..COEFFS.len() {
                let base = (pi * COEFFS.len() + j) * DIM;
                if j == 0 {
                    for (c, u) in self.cos[base..base + DIM].iter_mut().zip(&unit) {
                        *c += *u;
                    }
                } else {
                    let (cjf, sjf) = (cj as f32, sj as f32);
                    let cs = &mut self.cos[base..base + DIM];
                    let ss = &mut self.sin[base..base + DIM];
                    for k in 0..DIM {
                        cs[k] += unit[k] * cjf;
                        ss[k] += unit[k] * sjf;
                    }
                }
                let ncj = cj * c1 - sj * s1;
                let nsj = sj * c1 + cj * s1;
                cj = ncj;
                sj = nsj;
            }
        }
        self.frames += 1;
    }

    pub fn finish(mut self) -> Tmk {
        if self.frames > 0 {
            let inv = 1.0 / self.frames as f32;
            for p in &mut self.pure {
                *p *= inv;
            }
        }
        for pi in 0..PERIODS.len() {
            for (j, a) in COEFFS.iter().enumerate() {
                let base = (pi * COEFFS.len() + j) * DIM;
                let scale = a.sqrt();
                l2_normalize_scale(&mut self.cos[base..base + DIM], scale);
                l2_normalize_scale(&mut self.sin[base..base + DIM], scale);
            }
        }
        Tmk {
            pure: self.pure,
            cos: self.cos,
            sin: self.sin,
            frames: self.frames,
        }
    }
}

impl Default for TmkBuilder {
    fn default() -> Self {
        Self::new()
    }
}

fn l2_normalize_scale(v: &mut [f32], scale: f32) {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if n > 0.0 {
        let f = scale / n;
        for x in v {
            *x *= f;
        }
    }
}

#[inline]
fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let na = dot(a, a).sqrt();
    let nb = dot(b, b).sqrt();
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot(a, b) / (na * nb)
    }
}

/// Level 1: cosine similarity of the mean features.
pub fn level1(a: &Tmk, b: &Tmk) -> f32 {
    cosine(&a.pure, &b.pure)
}

/// Level 2: the kernel maximised over time offsets, per period, then the
/// best period; normalised to [−1, 1]. Returns (score, offset in seconds
/// from the best period, modulo that period).
pub fn level2(a: &Tmk, b: &Tmk) -> (f32, f32) {
    let m = COEFFS.len();
    let normalizer: f32 = COEFFS[0] + 2.0 * COEFFS[1..].iter().sum::<f32>();
    let mut best = f32::MIN;
    let mut best_offset = 0.0f32;
    for (pi, period) in PERIODS.iter().enumerate() {
        let mut cc = [0.0f32; 32];
        let mut ss = [0.0f32; 32];
        let mut sc = [0.0f32; 32];
        let mut cs = [0.0f32; 32];
        for j in 0..m {
            let base = (pi * m + j) * DIM;
            let (ca, cb) = (&a.cos[base..base + DIM], &b.cos[base..base + DIM]);
            let (sa, sb) = (&a.sin[base..base + DIM], &b.sin[base..base + DIM]);
            cc[j] = dot(ca, cb);
            ss[j] = dot(sa, sb);
            sc[j] = dot(sa, cb);
            cs[j] = dot(ca, sb);
        }
        let mut period_best = f32::MIN;
        let mut period_offset = 0u32;
        for offset in 0..*period {
            let delta = std::f64::consts::TAU * offset as f64 / *period as f64;
            let (sd, cd) = delta.sin_cos();
            let mut k = cc[0];
            let mut cj = 1.0f64;
            let mut sj = 0.0f64;
            for j in 1..m {
                let ncj = cj * cd - sj * sd;
                let nsj = sj * cd + cj * sd;
                cj = ncj;
                sj = nsj;
                k += cj as f32 * (cc[j] + ss[j]) + sj as f32 * (sc[j] - cs[j]);
            }
            if k > period_best {
                period_best = k;
                period_offset = offset;
            }
        }
        if period_best > best {
            best = period_best;
            best_offset = period_offset as f32 / FPS as f32;
        }
    }
    (best / normalizer, best_offset)
}

// ---------------------------------------------------------------------------
// hashing a file
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// bars
