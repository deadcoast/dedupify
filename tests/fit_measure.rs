//! CORE-01 and CORE-02 (Phase 11): the fit's cell score and mirror search,
//! measured against the plain fit on the same pairs.
//!
//! ```text
//! DEDUPE_REAL_DIR=<real set> [DEDUPE_COPYDAYS=<folder with original/ strong/>] \
//! [DEDUPE_PAIRS_IN=<pairs file written by real_copies>] \
//!   cargo test --release --test fit_measure -- --ignored --nocapture
//! ```
//!
//! Four searches on every pair: `plain` (the shipped fit through Phase 10),
//! `cells` (the window's cells, worst quarter set aside), `mirror` (the
//! second picture also searched mirrored), `both`. Positives: the 21
//! perturbations of real sources, a caption bar, a mirrored crop, the ELA
//! pair, Copydays' hand-made copies and its crops. Negatives: every pair of
//! the real set, a sample of Copydays' originals, and the pairs the learned
//! descriptor admits that are not copies (`DEDUPE_PAIRS_IN`). Nothing here
//! is a unit test; it prints a record for VERIFICATION.md § Phase 11.

mod common;
use common::*;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use image::{DynamicImage, Rgb, RgbImage};

use dedupe::geometry::{fit_with, Search, Thumb, FIT_MIN, MAX_SIDE};
use dedupe::perceptual::{DHASH_MAX, PHASH_MAX};

const SEARCHES: [(&str, Search); 4] = [
    ("plain", Search::PLAIN),
    (
        "cells",
        Search {
            robust: true,
            mirror: false,
        },
    ),
    (
        "mirror",
        Search {
            robust: false,
            mirror: true,
        },
    ),
    (
        "both",
        Search {
            robust: true,
            mirror: true,
        },
    ),
];

fn scores(a: &Thumb, b: &Thumb) -> [f32; 4] {
    SEARCHES.map(|(_, s)| fit_with(a, b, s).score())
}

fn thumb(img: &DynamicImage) -> Thumb {
    Thumb::from_image(img, MAX_SIDE)
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

/// A caption bar over the lower 15 %: near-black, with light strokes where
/// text would be.
fn caption(img: &RgbImage) -> RgbImage {
    let mut out = img.clone();
    let (w, h) = (out.width(), out.height());
    let top = h - h * 15 / 100;
    for y in top..h {
        for x in 0..w {
            out.put_pixel(x, y, Rgb([18, 18, 22]));
        }
    }
    // a line of "letters": light boxes with gaps
    let (ly0, ly1) = (top + (h - top) / 3, top + 2 * (h - top) / 3);
    let mut x = w / 20;
    let mut k = 0u32;
    while x + w / 40 < w - w / 20 {
        let cw = w / 60 + (k * 7 % 5) * w / 300;
        for y in ly0..ly1 {
            for xx in x..(x + cw).min(w) {
                out.put_pixel(xx, y, Rgb([235, 235, 235]));
            }
        }
        x += cw + w / 90 + if k % 6 == 5 { w / 40 } else { 0 };
        k += 1;
    }
    out
}

/// Distribution of one population under each search.
fn report(label: &str, rows: &[[f32; 4]], negative: bool) {
    eprintln!();
    eprintln!("{label}: {} pairs", rows.len());
    if rows.is_empty() {
        return;
    }
    for (k, (name, _)) in SEARCHES.iter().enumerate() {
        let mut v: Vec<f32> = rows.iter().map(|r| r[k]).collect();
        let n = v.len() as f32;
        let at = |t: f32| v.iter().filter(|&&s| s >= t).count();
        if negative {
            let (c85, c90, c92, c95) = (at(0.85), at(0.90), at(FIT_MIN), at(0.95));
            eprintln!(
                "  {name:<7} max {:.3}  p99.9 {:.3}  p99 {:.3}  median {:.3}   ≥.85 {c85}  ≥.90 {c90}  ≥{FIT_MIN:.2} {c92}  ≥.95 {c95}",
                pct(&mut v, 1.0),
                pct(&mut v, 0.999),
                pct(&mut v, 0.99),
                pct(&mut v, 0.5)
            );
        } else {
            let rec = 100.0 * at(FIT_MIN) as f32 / n;
            let (rec85, rec90) = (100.0 * at(0.85) as f32 / n, 100.0 * at(0.90) as f32 / n);
            eprintln!(
                "  {name:<7} min {:.3}  p5 {:.3}  median {:.3}   ≥{FIT_MIN:.2} {rec:5.1}%   (≥.90 {rec90:5.1}%  ≥.85 {rec85:5.1}%)",
                pct(&mut v, 0.0),
                pct(&mut v, 0.05),
                pct(&mut v, 0.5)
            );
        }
    }
}

#[test]
#[ignore]
fn measure_fit_searches() {
    let Some(real) = std::env::var_os("DEDUPE_REAL_DIR").map(PathBuf::from) else {
        eprintln!("DEDUPE_REAL_DIR not set; skipped");
        return;
    };
    let t0 = Instant::now();
    let real_pics: Vec<(String, RgbImage)> = files(&real)
        .into_iter()
        .filter_map(|p| {
            dedupe::decode::load_image(&p)
                .ok()
                .map(|i| (stem(&p), i.to_rgb8()))
        })
        .collect();
    let real_thumbs: Vec<Thumb> = real_pics
        .iter()
        .map(|(_, i)| thumb(&DynamicImage::ImageRgb8(i.clone())))
        .collect();
    let rt = |img: &RgbImage| thumb(&DynamicImage::ImageRgb8(img.clone()));

    // positives on real sources
    let m = real_pics.len().min(60);
    let mut per_kind: Vec<(&str, Vec<[f32; 4]>)> = Vec::new();
    for (k, name) in PERTURBATIONS.iter().enumerate() {
        let rows = (0..m)
            .map(|i| scores(&real_thumbs[i], &rt(&perturb(&real_pics[i].1, k))))
            .collect();
        per_kind.push((name, rows));
    }
    eprintln!();
    eprintln!(
        "real set: {} pictures; perturbations of the first {m} ({:.0}s)",
        real_pics.len(),
        t0.elapsed().as_secs_f64()
    );
    eprintln!(
        "{:<14} {:>14} {:>14} {:>14} {:>14}",
        "recall ≥0.92", "plain", "cells", "mirror", "both"
    );
    for (name, rows) in &per_kind {
        let rec = |k: usize| {
            100.0 * rows.iter().filter(|r| r[k] >= FIT_MIN).count() as f32 / rows.len() as f32
        };
        eprintln!(
            "{name:<14} {:>13.1}% {:>13.1}% {:>13.1}% {:>13.1}%",
            rec(0),
            rec(1),
            rec(2),
            rec(3)
        );
    }
    let all: Vec<[f32; 4]> = per_kind.iter().flat_map(|(_, r)| r.clone()).collect();
    report("real, all 21 perturbations", &all, false);
    let cap: Vec<[f32; 4]> = (0..m)
        .map(|i| scores(&real_thumbs[i], &rt(&caption(&real_pics[i].1))))
        .collect();
    report("real, caption bar over the lower 15 %", &cap, false);
    let mcrop: Vec<[f32; 4]> = (0..m)
        .map(|i| {
            let mirrored = image::imageops::flip_horizontal(&real_pics[i].1);
            scores(&real_thumbs[i], &rt(&crop_center(&mirrored, 0.8944)))
        })
        .collect();
    report(
        "real, mirrored and cropped to 80 % of the area",
        &mcrop,
        false,
    );

    // named pairs: the ELA edit, and every pair the two hashes group
    let ix: HashMap<&str, usize> = real_pics
        .iter()
        .enumerate()
        .map(|(i, (n, _))| (n.as_str(), i))
        .collect();
    eprintln!();
    eprintln!("named pairs (plain / cells / mirror / both):");
    let named = |a: &str, b: &str| {
        if let (Some(&i), Some(&j)) = (ix.get(a), ix.get(b)) {
            let s = scores(&real_thumbs[i], &real_thumbs[j]);
            eprintln!(
                "  {:.3} / {:.3} / {:.3} / {:.3}   {a} ~ {b}",
                s[0], s[1], s[2], s[3]
            );
        }
    };
    named("ela_original", "ela_modified");
    let sigs: Vec<_> = real_pics.iter().map(|(_, i)| sig_of(i)).collect();
    let n = real_pics.len();
    let mut neg_real: Vec<[f32; 4]> = Vec::new();
    let mut grouped: Vec<(usize, usize)> = Vec::new();
    for i in 0..n {
        for j in i + 1..n {
            neg_real.push(scores(&real_thumbs[i], &real_thumbs[j]));
            let (d, p, _) = best_triple(&sigs[i], &sigs[j]);
            if d <= DHASH_MAX && p <= PHASH_MAX {
                grouped.push((i, j));
            }
        }
    }
    eprintln!("  the hash-grouped pairs:");
    for (i, j) in grouped {
        let s = scores(&real_thumbs[i], &real_thumbs[j]);
        eprintln!(
            "  {:.3} / {:.3} / {:.3} / {:.3}   {} ~ {}",
            s[0], s[1], s[2], s[3], real_pics[i].0, real_pics[j].0
        );
    }
    // the real set's pairs are every pair of distinct files: a few are the
    // same picture (rubberwhale, basketball, the ELA pair, Suzanne) and
    // the rest siblings and strangers
    report(
        "real set, every pair of distinct files (copies among them)",
        &neg_real,
        true,
    );
    let mut top: Vec<(f32, usize)> = neg_real
        .iter()
        .enumerate()
        .map(|(k, r)| (r[3].max(r[1]), k))
        .collect();
    top.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
    let pair_of = |mut k: usize| {
        for i in 0..n {
            let row = n - i - 1;
            if k < row {
                return (i, i + 1 + k);
            }
            k -= row;
        }
        unreachable!()
    };
    eprintln!("  the 15 highest under cells or both:");
    for (_, k) in top.iter().take(15) {
        let (i, j) = pair_of(*k);
        let s = neg_real[*k];
        eprintln!(
            "  {:.3} / {:.3} / {:.3} / {:.3}   {} ~ {}",
            s[0], s[1], s[2], s[3], real_pics[i].0, real_pics[j].0
        );
    }

    // Copydays
    if let Some(cd) = std::env::var_os("DEDUPE_COPYDAYS").map(PathBuf::from) {
        let mut orig: HashMap<String, Thumb> = HashMap::new();
        let mut crops: Vec<[f32; 4]> = Vec::new();
        for (k, p) in files(&cd.join("original")).iter().enumerate() {
            let img = dedupe::decode::load_image(p).expect("decode");
            let t = thumb(&img);
            if k < 40 {
                let rgb = img.to_rgb8();
                for r in [20u32, 30, 40, 50] {
                    let keep = (1.0 - r as f32 / 100.0).sqrt();
                    crops.push(scores(&t, &rt(&crop_center(&rgb, keep))));
                }
            }
            orig.insert(stem(p), t);
        }
        report(
            "copydays crops (20–50 % of the surface, first 40 originals)",
            &crops,
            false,
        );
        let mut strong: Vec<[f32; 4]> = Vec::new();
        let mut strong_named: Vec<(String, [f32; 4])> = Vec::new();
        let mut strong_thumbs: HashMap<String, Thumb> = HashMap::new();
        for p in files(&cd.join("strong")) {
            let name = stem(&p);
            let t = thumb(&dedupe::decode::load_image(&p).expect("decode"));
            let s = scores(&orig[&format!("{}00", &name[..4])], &t);
            strong.push(s);
            strong_named.push((name.clone(), s));
            strong_thumbs.insert(name, t);
        }
        report("copydays strong (made by hand)", &strong, false);
        strong_named.sort_by(|a, b| (b.1[1] - b.1[0]).partial_cmp(&(a.1[1] - a.1[0])).unwrap());
        eprintln!("  most raised by cells (plain / cells / mirror / both):");
        for (nm, s) in strong_named.iter().take(8) {
            eprintln!(
                "  {:.3} / {:.3} / {:.3} / {:.3}   {nm}",
                s[0], s[1], s[2], s[3]
            );
        }
        // originals against originals, a deterministic sample
        let names: Vec<&String> = {
            let mut v: Vec<&String> = orig.keys().collect();
            v.sort();
            v
        };
        let mut neg_cd: Vec<[f32; 4]> = Vec::new();
        let mut idx = 0usize;
        for i in 0..names.len() {
            for j in i + 1..names.len() {
                if idx.is_multiple_of(4) {
                    neg_cd.push(scores(&orig[names[i]], &orig[names[j]]));
                }
                idx += 1;
            }
        }
        report(
            "copydays originals, every fourth pair (unrelated photographs)",
            &neg_cd,
            true,
        );
        // the pairs the descriptor admits that are not copies
        if let Some(pin) = std::env::var_os("DEDUPE_PAIRS_IN") {
            let text = std::fs::read_to_string(pin).expect("pairs file");
            let lookup = |n: &str| -> Option<&Thumb> {
                if let Some(r) = n.strip_prefix("real/") {
                    return ix.get(r).map(|&i| &real_thumbs[i]);
                }
                orig.get(n).or_else(|| strong_thumbs.get(n))
            };
            let mut admitted: Vec<(f32, [f32; 4], String, String)> = Vec::new();
            for line in text.lines() {
                let f: Vec<&str> = line.split('\t').collect();
                if f.len() < 5 || f[0] != "negative" {
                    continue;
                }
                let cos: f32 = f[3].parse().unwrap_or(0.0);
                if cos < 0.55 {
                    continue;
                }
                if let (Some(a), Some(b)) = (lookup(f[1]), lookup(f[2])) {
                    admitted.push((cos, scores(a, b), f[1].into(), f[2].into()));
                }
            }
            let rows: Vec<[f32; 4]> = admitted.iter().map(|x| x.1).collect();
            report(
                "descriptor-admitted non-copies (cosine ≥ 0.55; originals, hand copies, real set)",
                &rows,
                true,
            );
            admitted.sort_by(|a, b| b.1[3].max(b.1[1]).partial_cmp(&a.1[3].max(a.1[1])).unwrap());
            eprintln!(
                "  the 10 highest under cells or both (cosine; plain / cells / mirror / both):"
            );
            for (c, s, a, b) in admitted.iter().take(10) {
                eprintln!(
                    "  {c:.3};  {:.3} / {:.3} / {:.3} / {:.3}   {a} ~ {b}",
                    s[0], s[1], s[2], s[3]
                );
            }
        }
    }

    // cost per pair of each search
    let probe: Vec<(usize, usize)> = (0..30).map(|i| (i, (i * 7 + 3) % n)).collect();
    for (name, s) in SEARCHES {
        let t = Instant::now();
        for &(i, j) in &probe {
            let _ = fit_with(&real_thumbs[i], &real_thumbs[j], s);
        }
        eprintln!(
            "cost {name:<7} {:.1} ms a pair",
            t.elapsed().as_secs_f64() * 1000.0 / probe.len() as f64
        );
    }
    eprintln!("total {:.0}s", t0.elapsed().as_secs_f64());
}

/// The four searches on named pairs of the real set — the pairs the
/// descriptor admits there (`DEDUPE_NAMED_PAIRS`: lines `cos fit d p q a ~ b`
/// as `real_copies::measure_deep_real_pairs` prints them).
#[test]
#[ignore]
fn measure_fit_on_named_pairs() {
    let (Some(real), Some(named)) = (
        std::env::var_os("DEDUPE_REAL_DIR").map(PathBuf::from),
        std::env::var_os("DEDUPE_NAMED_PAIRS").map(PathBuf::from),
    ) else {
        eprintln!("DEDUPE_REAL_DIR / DEDUPE_NAMED_PAIRS not set; skipped");
        return;
    };
    let thumbs: HashMap<String, Thumb> = files(&real)
        .into_iter()
        .filter_map(|p| {
            dedupe::decode::load_image(&p)
                .ok()
                .map(|i| (stem(&p), thumb(&i)))
        })
        .collect();
    let mut rows: Vec<(f32, [f32; 4], String, String)> = Vec::new();
    for line in std::fs::read_to_string(named).expect("pairs").lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 8 || f[6] != "~" {
            continue;
        }
        let Ok(cos) = f[0].parse::<f32>() else {
            continue;
        };
        if let (Some(a), Some(b)) = (thumbs.get(f[5]), thumbs.get(f[7])) {
            rows.push((cos, scores(a, b), f[5].into(), f[7].into()));
        }
    }
    for (lo, hi) in [(0.70f32, 2.0f32), (0.60, 0.70), (0.45, 0.60)] {
        let band: Vec<[f32; 4]> = rows
            .iter()
            .filter(|r| r.0 >= lo && r.0 < hi)
            .map(|r| r.1)
            .collect();
        report(
            &format!("real set, descriptor cosine in [{lo:.2}, {hi:.2})"),
            &band,
            true,
        );
    }
    rows.sort_by(|a, b| b.1[3].max(b.1[1]).partial_cmp(&a.1[3].max(a.1[1])).unwrap());
    eprintln!("  the 12 highest under cells or both (cosine; plain / cells / mirror / both):");
    for (c, s, a, b) in rows.iter().take(12) {
        eprintln!(
            "  {c:.3};  {:.3} / {:.3} / {:.3} / {:.3}   {a} ~ {b}",
            s[0], s[1], s[2], s[3]
        );
    }
}
