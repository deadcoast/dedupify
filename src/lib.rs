//! `dedupe` — the Dedupelifier.
//!
//! Library surface so every stage is unit-testable and the binary stays a
//! thin dispatcher. Module order follows the data flow:
//!
//! cli → walk → exact → keeper → actions (journal, interrupt) → render
//!
//! `doctor`, `help` and `menu` are the remaining entry points.

pub mod actions;
pub mod capture;
pub mod cli;
#[cfg(feature = "deep")]
pub mod deep;
#[cfg(not(feature = "deep"))]
pub mod deep {
    //! Stub for builds without the deep tier: nothing is ever installed.
    use std::path::{Path, PathBuf};
    pub const MODEL_FILE: &str = "sscd_disc_mixup.onnx";
    pub const DIM: usize = 512;
    pub const DEEP_MIN: f32 = 0.70;
    pub type Embedding = [f32; DIM];
    pub struct Deep;
    #[derive(Debug)]
    pub struct DeepError;
    impl std::fmt::Display for DeepError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "built without the deep tier")
        }
    }
    pub fn model_path() -> PathBuf {
        if let Ok(p) = std::env::var("DEDUPE_MODEL") {
            if !p.is_empty() {
                return PathBuf::from(p);
            }
        }
        crate::journal::app_dir().join("models").join(MODEL_FILE)
    }
    pub fn installed() -> Option<PathBuf> {
        None
    }
    impl Deep {
        pub fn load(_: &Path) -> Result<Deep, DeepError> {
            Err(DeepError)
        }
        pub fn embed(&self, _: &image::DynamicImage) -> Result<Embedding, DeepError> {
            Err(DeepError)
        }
    }
    pub fn cosine(a: &Embedding, b: &Embedding) -> f32 {
        a.iter().zip(b).map(|(x, y)| x * y).sum()
    }
}
pub mod doctor;
pub mod exact;
pub mod geometry;
pub mod hamming;
pub mod help;
pub mod interrupt;
pub mod journal;
pub mod keeper;
pub mod kind;
pub mod menu;
pub mod near;
pub mod pdq;
pub mod perceptual;
pub mod preview;
pub mod recent;
pub mod render;
pub mod theme;
pub mod util;
pub mod video;
pub mod walk;
pub mod workspace;

/// Name of the directory `milk` moves duplicates into, created under the root.
pub const MILKED_DIR: &str = "_milked";
