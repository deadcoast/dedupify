//! CORE-12 (Phase 15): PDQ at one working size for every file, against
//! the shipped input (the box-prefiltered plane, whose size depends on the
//! file).
//!
//! ```text
//! DEDUPE_JPEG_DIRS=<folder>[:<folder>…] \
//!   cargo test --release --test pdq_fixed -- --ignored --nocapture
//! ```
//!
//! For every JPEG under the folders (decoded upright, as the program
//! decodes it): copies a library holds — a re-save, 70, 50, 35 and 25 %
//! (JPEG 85), a half-size PNG, a half-size copy turned a quarter (JPEG
//! 85), a mirror — PDQ max / 95th percentile / median to the original,
//! by each preparation (the turned and mirrored copies by the best of the
//! eight orientations, as matching takes them), and the reference tool's
//! hash of the original itself; then every pair of
//! different originals: the closest, and how many come within 31 and 40.

use dedupe::pdq::{distance, PDQ_MAX};
use dedupe::perceptual::{signature_from_image_with, PdqPrep, Signature};
use image::DynamicImage;
use rayon::prelude::*;
use std::path::PathBuf;

const PREPS: [(&str, PdqPrep); 4] = [
    ("shipped", PdqPrep::Prefiltered),
    (
        "512 tent",
        PdqPrep::Fixed {
            side: 512,
            lanczos: false,
        },
    ),
    (
        "576 tent",
        PdqPrep::Fixed {
            side: 576,
            lanczos: false,
        },
    ),
    (
        "576 lanczos",
        PdqPrep::Fixed {
            side: 576,
            lanczos: true,
        },
    ),
];

const KINDS: [&str; 9] = [
    "re-save q85",
    "70 % q85",
    "half q85",
    "35 % q85",
    "quarter q85",
    "half png",
    "half, turned",
    "mirror",
    "the tool's hash",
];

fn jpegs() -> Option<Vec<PathBuf>> {
    let dirs = std::env::var_os("DEDUPE_JPEG_DIRS")?;
    let mut files: Vec<PathBuf> = std::env::split_paths(&dirs)
        .flat_map(|d| {
            walkdir::WalkDir::new(d)
                .into_iter()
                .flatten()
                .filter(|e| e.file_type().is_file())
                .map(|e| e.into_path())
        })
        .filter(|p| {
            p.extension().is_some_and(|e| {
                matches!(e.to_string_lossy().to_lowercase().as_str(), "jpg" | "jpeg")
            })
        })
        .collect();
    files.sort();
    Some(files)
}

fn reencode(img: &DynamicImage) -> DynamicImage {
    let mut v = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut v, 85)
        .encode_image(&img.to_rgb8())
        .unwrap();
    image::load_from_memory(&v).unwrap()
}

fn resized(img: &DynamicImage, k: f32) -> DynamicImage {
    DynamicImage::ImageRgb8(image::imageops::resize(
        &img.to_rgb8(),
        ((img.width() as f32 * k).round() as u32).max(8),
        ((img.height() as f32 * k).round() as u32).max(8),
        image::imageops::FilterType::Triangle,
    ))
}

fn best(a: &Signature, b: &Signature) -> u32 {
    (0..8)
        .map(|k| distance(&a.pdq[0], &b.pdq[k]))
        .min()
        .unwrap()
}

fn pct(v: &mut [u32], p: f64) -> u32 {
    v.sort_unstable();
    v[((v.len() as f64 * p) as usize).min(v.len() - 1)]
}

#[test]
#[ignore]
fn one_working_size() {
    let Some(files) = jpegs() else {
        eprintln!("DEDUPE_JPEG_DIRS not set; skipped");
        return;
    };
    // per file: [prep] -> (original's signature, [kind] -> distance)
    type Row = Vec<(Signature, Vec<u32>)>;
    let rows: Vec<(String, (u32, u32), Row)> = files
        .par_iter()
        .filter_map(|f| {
            let img = dedupe::decode::load(f).ok()?.img;
            let copies = [
                reencode(&img),
                reencode(&resized(&img, 0.7)),
                reencode(&resized(&img, 0.5)),
                reencode(&resized(&img, 0.35)),
                reencode(&resized(&img, 0.25)),
                resized(&img, 0.5),
                reencode(&resized(&img, 0.5).rotate90()),
                img.fliph(),
            ];
            let per: Row = PREPS
                .iter()
                .map(|(_, prep)| {
                    let o = signature_from_image_with(&img, *prep);
                    let mut d: Vec<u32> = copies
                        .iter()
                        .enumerate()
                        .map(|(i, c)| {
                            let s = signature_from_image_with(c, *prep);
                            if i >= 6 {
                                best(&o, &s)
                            } else {
                                distance(&o.pdq[0], &s.pdq[0])
                            }
                        })
                        .collect();
                    // the reference tool's hash of the same picture
                    let t = signature_from_image_with(&img, PdqPrep::Tool);
                    d.push(distance(&o.pdq[0], &t.pdq[0]));
                    (o, d)
                })
                .collect();
            Some((f.display().to_string(), (img.width(), img.height()), per))
        })
        .collect();
    let mp: Vec<f64> = rows
        .iter()
        .map(|(_, (w, h), _)| (*w as f64 * *h as f64) / 1e6)
        .collect();
    eprintln!();
    eprintln!(
        "{} JPEGs ({:.1}–{:.1} MP); PDQ to the original: max / p95 / median",
        rows.len(),
        mp.iter().cloned().fold(f64::INFINITY, f64::min),
        mp.iter().cloned().fold(0.0, f64::max)
    );
    eprint!("{:<14}", "copy");
    for (name, _) in PREPS {
        eprint!(" {name:>16}");
    }
    eprintln!();
    for (k, kind) in KINDS.iter().enumerate() {
        eprint!("{kind:<14}");
        for p in 0..PREPS.len() {
            let mut v: Vec<u32> = rows.iter().map(|(_, _, r)| r[p].1[k]).collect();
            eprint!(
                " {:>6} {:>4} {:>4}",
                pct(&mut v, 1.0),
                pct(&mut v, 0.95),
                pct(&mut v, 0.5)
            );
        }
        eprintln!();
    }
    // different originals
    eprintln!();
    eprintln!("different originals, every pair: closest (by orientation 0 / best of 8), within 31, within 40");
    for (p, (name, _)) in PREPS.iter().enumerate() {
        let n = rows.len();
        let pairs: Vec<(u32, u32, usize, usize)> = (0..n)
            .into_par_iter()
            .flat_map(|i| {
                let rows = &rows;
                (i + 1..n).into_par_iter().map(move |j| {
                    let (a, b) = (&rows[i].2[p].0, &rows[j].2[p].0);
                    (distance(&a.pdq[0], &b.pdq[0]), best(a, b), i, j)
                })
            })
            .collect();
        let min0 = pairs.iter().map(|x| x.0).min().unwrap_or(256);
        let min8 = pairs.iter().map(|x| x.1).min().unwrap_or(256);
        let w31 = pairs.iter().filter(|x| x.0 <= PDQ_MAX).count();
        let w31b = pairs.iter().filter(|x| x.1 <= PDQ_MAX).count();
        let w40 = pairs.iter().filter(|x| x.0 <= 40).count();
        let mut close: Vec<&(u32, u32, usize, usize)> =
            pairs.iter().filter(|x| x.0 <= 40).collect();
        close.sort();
        eprintln!(
            "{name:<12} {} pairs   closest {min0} / {min8}   within 31: {w31} / {w31b}   within 40: {w40}",
            pairs.len()
        );
        for c in close.iter().take(6) {
            let short = |s: &str| {
                s.rsplit('/')
                    .take(2)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect::<Vec<_>>()
                    .join("/")
            };
            eprintln!(
                "    {:>3}  {}  {}",
                c.0,
                short(&rows[c.2].0),
                short(&rows[c.3].0)
            );
        }
    }
}
