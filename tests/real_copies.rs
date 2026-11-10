//! CORE-04 (Phase 11): the deep threshold on real labelled copies.
//!
//! ```text
//! DEDUPE_COPYDAYS=<folder holding original/ and strong/> DEDUPE_MODEL=<onnx> \
//!   cargo test --release --test real_copies -- --ignored --nocapture
//! ```
//!
//! INRIA Copydays (Jégou, Douze, Schmid, ECCV 2008): 157 personal holiday
//! photographs and 229 copies made from them by hand — print and scan,
//! paint, small crops, blur, perspective, collage. Copydays' JPEG and crop
//! sets are regenerated from the originals at its own levels (quality 75
//! down to 3; 10 to 80 % of the surface removed, here centred). Negatives:
//! every pair of pictures made from different originals, and with
//! `DEDUPE_REAL_DIR` every Copydays picture against every picture there.
//! `DEDUPE_PAIRS_OUT=<file>` writes every scored pair (class, a, b,
//! cosine, fit) for sweeps. Nothing here is a unit test; it prints a
//! record for VERIFICATION.md § Phase 11.

mod common;
use common::*;

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::Instant;

use image::{DynamicImage, RgbImage};

use dedupe::deep::{cosine, installed, Deep, Embedding, DEEP_MIN};
use dedupe::geometry::{fit, Thumb, FIT_MIN};
use dedupe::pdq::PDQ_MAX;
use dedupe::perceptual::{DHASH_MAX, PHASH_MAX};

/// Copydays' JPEG quality levels and crop levels (percent of the surface
/// removed).
const JPEG_LEVELS: [u8; 9] = [75, 50, 30, 20, 15, 10, 8, 5, 3];
const CROP_LEVELS: [u32; 9] = [10, 15, 20, 30, 40, 50, 60, 70, 80];
/// Negatives at or above this cosine also get a fit, for the acting rule.
const NEG_FIT_FROM: f32 = 0.40;
const SWEEP: [f32; 10] = [0.40, 0.45, 0.50, 0.55, 0.60, 0.65, 0.70, 0.75, 0.80, 0.85];

/// One scored picture: which original it comes from (`None` for an outside
/// negative), its descriptor and its fit thumbnail.
struct Pic {
    name: String,
    orig: Option<usize>,
    emb: Embedding,
    thumb: Thumb,
}

fn files(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = walkdir::WalkDir::new(dir)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file())
        .map(|e| e.into_path())
        .filter(|p| dedupe::decode::decodable(p))
        .collect();
    v.sort();
    v
}

fn stem(p: &Path) -> String {
    p.file_stem().unwrap().to_string_lossy().into_owned()
}

#[test]
#[ignore]
fn measure_deep_on_copydays() {
    let Some(root) = std::env::var_os("DEDUPE_COPYDAYS").map(PathBuf::from) else {
        eprintln!("DEDUPE_COPYDAYS not set; skipped");
        return;
    };
    let Some(model) = installed() else {
        eprintln!("no model installed; skipped");
        return;
    };
    let model = Deep::load(&model).expect("load model");
    let t0 = Instant::now();
    let embed = |img: &DynamicImage| model.embed(img).expect("embed");
    let thumb = |img: &DynamicImage| Thumb::from_image(img, dedupe::geometry::MAX_SIDE);

    // originals, and from each the regenerated JPEG and crop copies
    let mut pics: Vec<Pic> = Vec::new();
    // (class, positive index, original index)
    let mut positives: Vec<(String, usize, usize)> = Vec::new();
    let mut orig_ix: Vec<usize> = Vec::new();
    let mut orig_sig = Vec::new();
    let mut id_of: std::collections::HashMap<String, usize> = Default::default();
    for (o, p) in files(&root.join("original")).iter().enumerate() {
        let img = dedupe::decode::load_image(p).expect("decode original");
        let name = stem(p);
        id_of.insert(name[..4].to_string(), o);
        orig_ix.push(pics.len());
        orig_sig.push(sig_of(&img.to_rgb8()));
        pics.push(Pic {
            name: name.clone(),
            orig: Some(o),
            emb: embed(&img),
            thumb: thumb(&img),
        });
        let rgb: RgbImage = img.to_rgb8();
        drop(img);
        for q in JPEG_LEVELS {
            let v = DynamicImage::ImageRgb8(jpeg(&rgb, q));
            positives.push((format!("jpeg q{q}"), pics.len(), o));
            pics.push(Pic {
                name: format!("{name}~q{q}"),
                orig: Some(o),
                emb: embed(&v),
                thumb: thumb(&v),
            });
        }
        for r in CROP_LEVELS {
            let keep = (1.0 - r as f32 / 100.0).sqrt();
            let v = DynamicImage::ImageRgb8(crop_center(&rgb, keep));
            positives.push((format!("crop {r}%"), pics.len(), o));
            pics.push(Pic {
                name: format!("{name}~crop{r}"),
                orig: Some(o),
                emb: embed(&v),
                thumb: thumb(&v),
            });
        }
        if o % 20 == 19 {
            eprintln!("  {} originals, {:.0}s", o + 1, t0.elapsed().as_secs_f64());
        }
    }
    let n_orig = orig_ix.len();

    // the strong copies, made by hand
    let mut strong_hash: Vec<(String, u32, u32, u32)> = Vec::new();
    for p in files(&root.join("strong")) {
        let name = stem(&p);
        let o = id_of[&name[..4]];
        let img = dedupe::decode::load_image(&p).expect("decode strong");
        let (d, ph, q) = best_triple(&orig_sig[o], &sig_of(&img.to_rgb8()));
        strong_hash.push((name.clone(), d, ph, q));
        positives.push(("strong".into(), pics.len(), o));
        pics.push(Pic {
            name,
            orig: Some(o),
            emb: embed(&img),
            thumb: thumb(&img),
        });
    }
    let n_copydays = pics.len();

    // outside negatives
    if let Some(extra) = std::env::var_os("DEDUPE_REAL_DIR").map(PathBuf::from) {
        for p in files(&extra) {
            let Ok(img) = dedupe::decode::load_image(&p) else {
                continue;
            };
            pics.push(Pic {
                name: format!("real/{}", stem(&p)),
                orig: None,
                emb: embed(&img),
                thumb: thumb(&img),
            });
        }
    }
    eprintln!();
    eprintln!(
        "copydays: {n_orig} originals, {} positives ({} strong), {} outside pictures; {} embeddings in {:.0}s",
        positives.len(),
        strong_hash.len(),
        pics.len() - n_copydays,
        pics.len(),
        t0.elapsed().as_secs_f64()
    );

    let mut out = std::env::var_os("DEDUPE_PAIRS_OUT")
        .map(|p| std::io::BufWriter::new(std::fs::File::create(p).expect("pairs file")));

    // positives: each copy against its own original
    let mut by_class: Vec<(String, Vec<(f32, f32)>)> = Vec::new();
    let mut strong_rows: Vec<(String, f32, f32)> = Vec::new();
    for (class, pi, o) in &positives {
        let a = &pics[orig_ix[*o]];
        let b = &pics[*pi];
        let c = cosine(&a.emb, &b.emb);
        let f = fit(&a.thumb, &b.thumb).ncc;
        if let Some(w) = out.as_mut() {
            writeln!(w, "{class}\t{}\t{}\t{c:.4}\t{f:.4}", a.name, b.name).unwrap();
        }
        if class == "strong" {
            strong_rows.push((b.name.clone(), c, f));
        }
        match by_class.iter_mut().find(|(k, _)| k == class) {
            Some((_, v)) => v.push((c, f)),
            None => by_class.push((class.clone(), vec![(c, f)])),
        }
    }

    // negatives: different originals (or an outside picture against
    // Copydays), the outside set not against itself
    let mut neg: Vec<f32> = Vec::new();
    let mut neg_fit: Vec<(f32, f32, usize, usize)> = Vec::new();
    for i in 0..pics.len() {
        for j in i + 1..pics.len() {
            let (a, b) = (&pics[i], &pics[j]);
            let negative = match (a.orig, b.orig) {
                (Some(x), Some(y)) => x != y,
                (None, None) => false,
                _ => i < n_copydays || j < n_copydays,
            };
            if !negative {
                continue;
            }
            let c = cosine(&a.emb, &b.emb);
            neg.push(c);
            if c >= NEG_FIT_FROM {
                let f = fit(&a.thumb, &b.thumb).ncc;
                neg_fit.push((c, f, i, j));
                if let Some(w) = out.as_mut() {
                    writeln!(w, "negative\t{}\t{}\t{c:.4}\t{f:.4}", a.name, b.name).unwrap();
                }
            }
        }
    }

    eprintln!();
    eprintln!(
        "{:<11} {:>4} {:>6} {:>6} {:>6} {:>6} {:>6} {:>6} {:>6} {:>7} {:>7}",
        "copy", "n", "min", "p5", "median", "≥.60", "≥.65", "≥.70", "≥.75", "fit≥", "acts"
    );
    for (class, v) in &by_class {
        let mut cs: Vec<f32> = v.iter().map(|x| x.0).collect();
        let n = cs.len() as f32;
        let rate = |t: f32| 100.0 * v.iter().filter(|x| x.0 >= t).count() as f32 / n;
        let fit_rate = 100.0 * v.iter().filter(|x| x.1 >= FIT_MIN).count() as f32 / n;
        let acts = 100.0
            * v.iter()
                .filter(|x| x.0 >= DEEP_MIN && x.1 >= FIT_MIN)
                .count() as f32
            / n;
        let (mn, p5, med) = (pct(&mut cs, 0.0), pct(&mut cs, 0.05), pct(&mut cs, 0.5));
        eprintln!(
            "{class:<11} {:>4} {mn:>6.3} {p5:>6.3} {med:>6.3} {:>5.1}% {:>5.1}% {:>5.1}% {:>5.1}% {fit_rate:>6.1}% {acts:>6.1}%",
            v.len(),
            rate(0.60),
            rate(0.65),
            rate(0.70),
            rate(0.75)
        );
    }

    let n_neg = neg.len();
    let (med, p99, p999, max) = (
        pct(&mut neg, 0.5),
        pct(&mut neg, 0.99),
        pct(&mut neg, 0.999),
        pct(&mut neg, 1.0),
    );
    eprintln!();
    eprintln!(
        "negatives: {n_neg} pairs, median {med:.3}, p99 {p99:.3}, p99.9 {p999:.3}, max {max:.3}"
    );
    eprintln!();
    eprintln!(
        "{:>5} {:>9} {:>11} {:>8} {:>8} {:>8}",
        "cos", "false pos", "fp & fit≥", "strong", "jpeg", "crop"
    );
    let recall = |prefix: &str, t: f32| {
        let (mut k, mut n) = (0, 0);
        for (class, v) in &by_class {
            if class.starts_with(prefix) {
                n += v.len();
                k += v.iter().filter(|x| x.0 >= t).count();
            }
        }
        100.0 * k as f32 / n.max(1) as f32
    };
    for t in SWEEP {
        let fp = neg.iter().filter(|&&c| c >= t).count();
        let fp_fit = neg_fit
            .iter()
            .filter(|x| x.0 >= t && x.1 >= FIT_MIN)
            .count();
        eprintln!(
            "{t:>5.2} {fp:>9} {fp_fit:>11} {:>7.1}% {:>7.1}% {:>7.1}%",
            recall("strong", t),
            recall("jpeg", t),
            recall("crop", t)
        );
    }

    neg_fit.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
    eprintln!();
    eprintln!("closest negatives (cosine, fit):");
    for (c, f, i, j) in neg_fit.iter().take(15) {
        eprintln!("  {c:.3} {f:>5.2}  {}  {}", pics[*i].name, pics[*j].name);
    }

    strong_rows.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
    let hash_of = |n: &str| strong_hash.iter().find(|h| h.0 == n).unwrap();
    let hash_pair = strong_hash
        .iter()
        .filter(|h| h.1 <= DHASH_MAX && h.2 <= PHASH_MAX)
        .count();
    let pdq = strong_hash.iter().filter(|h| h.3 <= PDQ_MAX).count();
    eprintln!();
    eprintln!(
        "strong copies the hashes alone find: dHash∧pHash {hash_pair} of {}, PDQ ≤ {PDQ_MAX} {pdq}",
        strong_hash.len()
    );
    eprintln!("strong copies under {DEEP_MIN:.2} (cosine, fit, d p q):");
    for (n, c, f) in strong_rows.iter().filter(|r| r.1 < DEEP_MIN) {
        let h = hash_of(n);
        eprintln!("  {n}  {c:.3} {f:>5.2}  {} {} {}", h.1, h.2, h.3);
    }
    eprintln!();
    eprintln!("total {:.0}s", t0.elapsed().as_secs_f64());
}

/// The real set against itself: every pair's cosine, fit and hashes, so
/// the pairs a lower `DEEP_MIN` would admit can be named — copies,
/// another shot of the same scene, or strangers.
///
/// `DEDUPE_REAL_DIR=<folder> DEDUPE_MODEL=<onnx> cargo test --release
/// --test real_copies -- --ignored measure_deep_real_pairs --nocapture`
#[test]
#[ignore]
fn measure_deep_real_pairs() {
    let Some(dir) = std::env::var_os("DEDUPE_REAL_DIR").map(PathBuf::from) else {
        eprintln!("DEDUPE_REAL_DIR not set; skipped");
        return;
    };
    let Some(model) = installed() else {
        eprintln!("no model installed; skipped");
        return;
    };
    let model = Deep::load(&model).expect("load model");
    let mut pics: Vec<(String, Embedding, Thumb, dedupe::perceptual::Signature)> = Vec::new();
    for p in files(&dir) {
        let Ok(img) = dedupe::decode::load_image(&p) else {
            continue;
        };
        pics.push((
            stem(&p),
            model.embed(&img).expect("embed"),
            Thumb::from_image(&img, dedupe::geometry::MAX_SIDE),
            sig_of(&img.to_rgb8()),
        ));
    }
    let mut rows: Vec<(f32, f32, u32, u32, u32, usize, usize)> = Vec::new();
    for i in 0..pics.len() {
        for j in i + 1..pics.len() {
            let c = cosine(&pics[i].1, &pics[j].1);
            if c >= 0.45 {
                let f = fit(&pics[i].2, &pics[j].2).score();
                let (d, p, q) = best_triple(&pics[i].3, &pics[j].3);
                rows.push((c, f, d, p, q, i, j));
            }
        }
    }
    rows.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
    eprintln!();
    eprintln!(
        "{} pictures; pairs at cosine ≥ 0.45 (cosine, fit, d p q):",
        pics.len()
    );
    for (c, f, d, p, q, i, j) in rows {
        eprintln!(
            "  {c:.3}  {f:>5.2}  {d:>2} {p:>2} {q:>3}   {} ~ {}",
            pics[i].0, pics[j].0
        );
    }
}
