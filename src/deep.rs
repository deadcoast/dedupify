//! Deep tier (docs/RESEARCH.md §2.6): a learned copy-detection descriptor,
//! off unless a model file is present.
//!
//! Model: SSCD `sscd_disc_mixup` (Pizzi et al., CVPR 2022) — ResNet-50 with
//! GeM pooling trained for copy detection on DISC2021, 512-d L2-normalised
//! embedding, compared by dot product. Run with `tract` (pure Rust ONNX
//! inference) so the binary stays single-file; the model is the only thing
//! the user installs. Preprocessing is the official `skew_320` transform:
//! resize to 320×320 ignoring aspect, RGB in [0, 1], ImageNet mean/std.
//!
//! Where the model lives: `DEDUPE_MODEL` (a path), else
//! `<local data>/dedupe/models/sscd_disc_mixup.onnx` beside the journal
//! directory. `doctor` reports which.

use std::path::{Path, PathBuf};

use image::imageops::FilterType;
use image::DynamicImage;
use tract_onnx::prelude::*;

pub const MODEL_FILE: &str = "sscd_disc_mixup.onnx";
/// The published file's SHA-256 and size (docs/DEEP.md); `doctor` checks
/// an installed model against them.
pub const MODEL_SHA256: &str = "e65e2a5b9a5e44a0b0f011bddaa6f28621021f6120ca7ce8b526c19f2f6156f8";
pub const MODEL_BYTES: u64 = 98_156_202;
pub const ENV_MODEL: &str = "DEDUPE_MODEL";
pub const SIZE: usize = 320;
pub const DIM: usize = 512;
const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
const STD: [f32; 3] = [0.229, 0.224, 0.225];

/// Cosine (dot product of unit vectors) at or above which two images are
/// grouped as edited copies. Measured on the generated corpus (docs/
/// VERIFICATION.md §Phase 4): 0 false positives in 33,540 negative pairs
/// whose maximum was 0.669; 0.65 let 4 through, 0.60 let 127 through. On
/// real photographs (Phase 11) unrelated pairs top out at 0.526 in 5.44
/// million, while photographs of one subject from another angle reach
/// 0.88: the bar stays where listing them as similar is rare. The deep
/// tier alone only lists; it acts with the fit (see `near::Relation`).
pub const DEEP_MIN: f32 = 0.70;

/// Under `DEEP_MIN`, down to this cosine, a pair enters a group only when
/// the geometric fit confirms it as well — and then as a confirmed copy,
/// never as a listed one (CORE-04, Phase 11). Measured on real labelled
/// copies (INRIA Copydays, docs/VERIFICATION.md § Phase 11): no pair of
/// 5.44 million unrelated real photographs reaches 0.55 (the closest,
/// two different photographs both at JPEG quality 5–15, 0.526);
/// photographs of one subject from another position (a chessboard in 13
/// poses from two cameras) reach 0.88, and in the band fit at most 0.787
/// under `geometry::SEARCH`, so they stay out; copies cropped to 40 % of
/// their area act at 91.7 % instead of 46.5 %. In the band a single
/// joins a keeper or a partner only after every pair at `DEEP_MIN` is
/// made (`near::group`).
pub const DEEP_FIT_MIN: f32 = 0.60;

pub type Embedding = [f32; DIM];

type Plan = std::sync::Arc<TypedRunnableModel>;

pub struct Deep {
    plan: Plan,
    pub path: PathBuf,
}

#[derive(Debug)]
pub enum DeepError {
    Load(String),
    Run(String),
    Shape(String),
}

impl std::fmt::Display for DeepError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeepError::Load(s) => write!(f, "model load: {s}"),
            DeepError::Run(s) => write!(f, "model run: {s}"),
            DeepError::Shape(s) => write!(f, "model output: {s}"),
        }
    }
}

/// Where a model would be looked for, whether or not it exists.
pub fn model_path() -> PathBuf {
    if let Ok(p) = std::env::var(ENV_MODEL) {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    crate::journal::app_dir().join("models").join(MODEL_FILE)
}

/// `Some(path)` when a model file is present.
pub fn installed() -> Option<PathBuf> {
    let p = model_path();
    if p.is_file() {
        Some(p)
    } else {
        None
    }
}

impl Deep {
    pub fn load(path: &Path) -> Result<Deep, DeepError> {
        let model = tract_onnx::onnx()
            .model_for_path(path)
            .map_err(|e| DeepError::Load(e.to_string()))?
            .with_input_fact(0, f32::fact([1, 3, SIZE, SIZE]).into())
            .map_err(|e| DeepError::Load(e.to_string()))?
            .into_optimized()
            .map_err(|e| DeepError::Load(e.to_string()))?
            .into_runnable()
            .map_err(|e| DeepError::Load(e.to_string()))?;
        Ok(Deep {
            plan: model,
            path: path.to_path_buf(),
        })
    }

    /// `skew_320` preprocessing into an NCHW tensor.
    pub fn preprocess(img: &DynamicImage) -> Tensor {
        let rgb = image::imageops::resize(
            &img.to_rgb8(),
            SIZE as u32,
            SIZE as u32,
            FilterType::Triangle,
        );
        let mut t = tract_ndarray::Array4::<f32>::zeros((1, 3, SIZE, SIZE));
        for (x, y, p) in rgb.enumerate_pixels() {
            for c in 0..3 {
                t[[0, c, y as usize, x as usize]] = (p.0[c] as f32 / 255.0 - MEAN[c]) / STD[c];
            }
        }
        t.into()
    }

    /// Embed one decoded image. Thread-safe; each call has its own state.
    pub fn embed(&self, img: &DynamicImage) -> Result<Embedding, DeepError> {
        let input = Self::preprocess(img);
        let out = self
            .plan
            .run(tvec!(input.into()))
            .map_err(|e| DeepError::Run(e.to_string()))?;
        let view = out[0]
            .to_plain_array_view::<f32>()
            .map_err(|e| DeepError::Shape(e.to_string()))?;
        let flat: Vec<f32> = view.iter().copied().collect();
        if flat.len() != DIM {
            return Err(DeepError::Shape(format!(
                "{} values, expected {DIM}",
                flat.len()
            )));
        }
        let mut e = [0.0f32; DIM];
        e.copy_from_slice(&flat);
        let n = e.iter().map(|v| v * v).sum::<f32>().sqrt();
        if n > 0.0 {
            for v in &mut e {
                *v /= n;
            }
        }
        Ok(e)
    }
}

#[inline]
pub fn cosine(a: &Embedding, b: &Embedding) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model() -> Option<Deep> {
        let p = installed()?;
        Deep::load(&p).ok()
    }

    #[test]
    fn preprocess_shape_and_normalisation() {
        let img = DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            50,
            30,
            image::Rgb([255, 0, 128]),
        ));
        let t = Deep::preprocess(&img);
        assert_eq!(t.shape(), &[1, 3, SIZE, SIZE]);
        let a = t.to_plain_array_view::<f32>().unwrap();
        // red channel at 1.0 → (1 − 0.485)/0.229
        assert!((a[[0, 0, 10, 10]] - (1.0 - 0.485) / 0.229).abs() < 1e-4);
        assert!((a[[0, 1, 10, 10]] - (0.0 - 0.456) / 0.224).abs() < 1e-4);
    }

    /// A picture with real structure: coloured blocks, discs and bars laid
    /// out from a seed, so two seeds give two different pictures rather
    /// than two renderings of one layout.
    fn picture(seed: u64, w: u32, h: u32) -> image::RgbImage {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        let mut rnd = move || {
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40) as f32 / (1u64 << 24) as f32
        };
        let bg = image::Rgb([
            (rnd() * 255.0) as u8,
            (rnd() * 255.0) as u8,
            (rnd() * 255.0) as u8,
        ]);
        let mut img = image::RgbImage::from_pixel(w, h, bg);
        for _ in 0..7 {
            let col = image::Rgb([
                (rnd() * 255.0) as u8,
                (rnd() * 255.0) as u8,
                (rnd() * 255.0) as u8,
            ]);
            let cx = rnd() * w as f32;
            let cy = rnd() * h as f32;
            let rw = (0.08 + 0.25 * rnd()) * w as f32;
            let rh = (0.08 + 0.25 * rnd()) * h as f32;
            let disc = rnd() > 0.5;
            for y in 0..h {
                for px in 0..w {
                    let dx = (px as f32 - cx) / rw;
                    let dy = (y as f32 - cy) / rh;
                    let inside = if disc {
                        dx * dx + dy * dy < 1.0
                    } else {
                        dx.abs() < 1.0 && dy.abs() < 1.0
                    };
                    if inside {
                        img.put_pixel(px, y, col);
                    }
                }
            }
        }
        img
    }

    /// Runs only when the model is installed (see docs/DEEP.md).
    #[test]
    fn copies_score_high_and_strangers_low() {
        let Some(m) = model() else {
            eprintln!("no model installed; skipped");
            return;
        };
        let a = picture(3, 640, 480);
        let a_img = DynamicImage::ImageRgb8(a.clone());
        let small =
            DynamicImage::ImageRgb8(image::imageops::resize(&a, 160, 120, FilterType::Triangle));
        let crop =
            DynamicImage::ImageRgb8(image::imageops::crop_imm(&a, 96, 72, 448, 336).to_image());
        let other = DynamicImage::ImageRgb8(picture(4, 640, 480));
        let t = std::time::Instant::now();
        let ea = m.embed(&a_img).unwrap();
        eprintln!(
            "one embedding: {:.0} ms",
            t.elapsed().as_secs_f64() * 1000.0
        );
        let es = m.embed(&small).unwrap();
        let ec = m.embed(&crop).unwrap();
        let eo = m.embed(&other).unwrap();
        eprintln!(
            "small {:.3}  crop {:.3}  other {:.3}",
            cosine(&ea, &es),
            cosine(&ea, &ec),
            cosine(&ea, &eo)
        );
        assert!((cosine(&ea, &ea) - 1.0).abs() < 1e-3);
        assert!(cosine(&ea, &es) > DEEP_MIN, "small {}", cosine(&ea, &es));
        assert!(cosine(&ea, &ec) > DEEP_MIN, "crop {}", cosine(&ea, &ec));
        assert!(cosine(&ea, &eo) < DEEP_MIN, "other {}", cosine(&ea, &eo));
    }
}
