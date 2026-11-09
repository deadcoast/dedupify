//! Measurement on real photographs.
//!
//! `DEDUPE_REAL_DIR=<folder> cargo test --release --test real_corpus -- --ignored --nocapture`
//!
//! Two questions the procedural corpus cannot answer:
//!
//! 1. The library as it is: among every pair of distinct real pictures,
//!    how close do unrelated ones come to the shipped thresholds, and
//!    which pairs the rule would group — printed so a person can label
//!    them (derivative, sibling capture, unrelated).
//! 2. Derivatives of real sources: the 21 perturbations applied to each
//!    real picture — recall per perturbation and false positives against
//!    every other real source, with the same rule the crate ships.
//!
//! Nothing here is a unit test; it prints a record for VERIFICATION.md.

mod common;
use common::*;

use std::path::{Path, PathBuf};
use std::time::Instant;

use image::RgbImage;

use dedupe::pdq::PDQ_MAX;
use dedupe::perceptual::{DHASH_MAX, PHASH_MAX};

fn real_dir() -> Option<PathBuf> {
    std::env::var_os("DEDUPE_REAL_DIR").map(PathBuf::from)
}

fn load_all(dir: &Path) -> Vec<(String, RgbImage)> {
    let mut out = Vec::new();
    let mut paths: Vec<PathBuf> = walkdir::WalkDir::new(dir)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file())
        .map(|e| e.into_path())
        .filter(|p| dedupe::decode::decodable(p))
        .collect();
    paths.sort();
    for p in paths {
        if let Ok(img) = image::open(&p) {
            let name = p
                .strip_prefix(dir)
                .unwrap_or(&p)
                .to_string_lossy()
                .into_owned();
            out.push((name, img.to_rgb8()));
        }
    }
    out
}

#[test]
#[ignore]
fn measure_real() {
    let Some(dir) = real_dir() else {
        eprintln!("DEDUPE_REAL_DIR not set; skipped");
        return;
    };
    let t0 = Instant::now();
    let pics = load_all(&dir);
    let sigs: Vec<_> = pics.iter().map(|(_, im)| sig_of(im)).collect();
    let n = pics.len();
    eprintln!();
    eprintln!(
        "real corpus: {n} pictures from {}   decoded + hashed in {:.1}s",
        dir.display(),
        t0.elapsed().as_secs_f64()
    );

    // ---- 1. the library as it is -------------------------------------------
    let mut pairs: Vec<(u32, u32, u32, usize, usize)> = Vec::new();
    for i in 0..n {
        for j in i + 1..n {
            let (d, p, q) = best_triple(&sigs[i], &sigs[j]);
            pairs.push((d, p, q, i, j));
        }
    }
    pairs.sort_by_key(|(d, p, _, _, _)| d + p);
    eprintln!();
    eprintln!(
        "all {} pairs of distinct pictures; the 24 closest by d+p (shipped rule: d <={DHASH_MAX} and p <={PHASH_MAX}; PDQ confirms at q <={PDQ_MAX}):",
        pairs.len()
    );
    eprintln!("{:>3} {:>3} {:>4}   pair", "d", "p", "q");
    for (d, p, q, i, j) in pairs.iter().take(24) {
        let rule = if *d <= DHASH_MAX && *p <= PHASH_MAX {
            if *q <= PDQ_MAX {
                "GROUP + CONFIRM"
            } else {
                "GROUP (q over)"
            }
        } else {
            ""
        };
        eprintln!(
            "{d:>3} {p:>3} {q:>4}   {}  ~  {}   {rule}",
            pics[*i].0, pics[*j].0
        );
    }
    let grouped = pairs
        .iter()
        .filter(|(d, p, _, _, _)| *d <= DHASH_MAX && *p <= PHASH_MAX)
        .count();
    let confirmed = pairs
        .iter()
        .filter(|(d, p, q, _, _)| *d <= DHASH_MAX && *p <= PHASH_MAX && *q <= PDQ_MAX)
        .count();
    eprintln!();
    eprintln!("pairs the shipped rule groups: {grouped}   of which PDQ confirms: {confirmed}");
    // how the sum d+p is distributed just outside the rule
    let mut hist = [0usize; 8];
    for (d, p, _, _, _) in &pairs {
        let s = (d + p) as usize;
        let bucket = match s {
            0..=24 => 0,
            25..=28 => 1,
            29..=32 => 2,
            33..=36 => 3,
            37..=40 => 4,
            41..=48 => 5,
            49..=56 => 6,
            _ => 7,
        };
        hist[bucket] += 1;
    }
    eprintln!(
        "d+p histogram: <=24 {}  25-28 {}  29-32 {}  33-36 {}  37-40 {}  41-48 {}  49-56 {}  >56 {}",
        hist[0], hist[1], hist[2], hist[3], hist[4], hist[5], hist[6], hist[7]
    );

    // ---- 2. derivatives of real sources -------------------------------------
    let cap: usize = std::env::var("DEDUPE_REAL_CAP")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(60);
    let m = n.min(cap);
    let t1 = Instant::now();
    let mut var_sigs: Vec<Vec<_>> = Vec::new();
    for (_, im) in pics.iter().take(m) {
        var_sigs.push(
            (0..PERTURBATIONS.len())
                .map(|k| sig_of(&perturb(im, k)))
                .collect(),
        );
    }
    eprintln!();
    eprintln!(
        "derivatives: {m} sources x {} perturbations in {:.0}s",
        PERTURBATIONS.len(),
        t1.elapsed().as_secs_f64()
    );
    eprintln!(
        "{:<14} {:>5} {:>5} {:>5}   {:>9} {:>9}",
        "perturbation", "d max", "p max", "q max", "d&p", "d&p&q"
    );
    for (k, name) in PERTURBATIONS.iter().enumerate() {
        let mut dm = 0;
        let mut pm = 0;
        let mut qm = 0;
        let mut hit = 0;
        let mut conf = 0;
        for i in 0..m {
            let (d, p, q) = best_triple(&sigs[i], &var_sigs[i][k]);
            dm = dm.max(d);
            pm = pm.max(p);
            qm = qm.max(q);
            if d <= DHASH_MAX && p <= PHASH_MAX {
                hit += 1;
                if q <= PDQ_MAX {
                    conf += 1;
                }
            }
        }
        eprintln!(
            "{name:<14} {dm:>5} {pm:>5} {qm:>5}   {:>8.1}% {:>8.1}%",
            hit as f64 * 100.0 / m as f64,
            conf as f64 * 100.0 / m as f64
        );
        // name the sources a plain rescale or re-encode misses
        if k < 6 {
            for i in 0..m {
                let (d, p, q) = best_triple(&sigs[i], &var_sigs[i][k]);
                if !(d <= DHASH_MAX && p <= PHASH_MAX && q <= PDQ_MAX) {
                    eprintln!("               miss: {}  d {d} p {p} q {q}", pics[i].0);
                }
            }
        }
    }
    // negatives: every variant against every other source
    let mut neg_grouped = 0usize;
    let mut neg_confirmed = 0usize;
    let mut neg_total = 0usize;
    let mut closest: Vec<(u32, u32, u32, String)> = Vec::new();
    for i in 0..m {
        for (k, vs) in var_sigs[i].iter().enumerate() {
            for j in 0..n {
                if j == i {
                    continue;
                }
                let (d, p, q) = best_triple(vs, &sigs[j]);
                neg_total += 1;
                if d <= DHASH_MAX && p <= PHASH_MAX {
                    neg_grouped += 1;
                    if q <= PDQ_MAX {
                        neg_confirmed += 1;
                    }
                    closest.push((
                        d,
                        p,
                        q,
                        format!("{} [{}]  ~  {}", pics[i].0, PERTURBATIONS[k], pics[j].0),
                    ));
                }
            }
        }
    }
    closest.sort_by_key(|(d, p, _, _)| d + p);
    eprintln!();
    eprintln!(
        "negatives (variant vs other source): {neg_total} pairs   grouped by the rule: {neg_grouped}   PDQ-confirmed: {neg_confirmed}"
    );
    // the same source pair reached through several perturbations is one
    // relation; list each source pair once with its best (d, p, q)
    let mut by_pair: std::collections::BTreeMap<(String, String), (u32, u32, u32, usize)> =
        std::collections::BTreeMap::new();
    for (d, p, q, s) in &closest {
        let (left, right) = s.split_once("  ~  ").unwrap();
        let src = left.split(" [").next().unwrap().to_string();
        let key = if src.as_str() < right {
            (src, right.to_string())
        } else {
            (right.to_string(), src)
        };
        let e = by_pair
            .entry(key)
            .or_insert((u32::MAX, u32::MAX, u32::MAX, 0));
        if d + p < e.0 + e.1 {
            e.0 = *d;
            e.1 = *p;
            e.2 = *q;
        }
        e.3 += 1;
    }
    eprintln!("as source pairs ({}):", by_pair.len());
    for ((a, b), (d, p, q, k)) in &by_pair {
        eprintln!("{d:>3} {p:>3} {q:>4}   {a}  ~  {b}   via {k} variants");
    }
}

/// `DEDUPE_REAL_DIR=<folder> cargo test --release --test real_corpus -- --ignored measure_geometry_real --nocapture`
#[test]
#[ignore]
fn measure_geometry_real() {
    let Some(dir) = real_dir() else {
        eprintln!("DEDUPE_REAL_DIR not set; skipped");
        return;
    };
    let pics = load_all(&dir);
    let bases: Vec<RgbImage> = pics.into_iter().map(|(_, im)| im).collect();
    measure_geometry_on("real photographs", &bases, 60, 6000);
}

/// The fit on the pairs that matter in the real set: everything the hash
/// rule groups (confirmed or not), plus the worst-fitting unrelated pairs
/// and the worst-fitting positives, by name.
#[test]
#[ignore]
fn geometry_real_named() {
    use dedupe::geometry::fit;
    let Some(dir) = real_dir() else {
        eprintln!("DEDUPE_REAL_DIR not set; skipped");
        return;
    };
    let pics = load_all(&dir);
    let sigs: Vec<_> = pics.iter().map(|(_, im)| sig_of(im)).collect();
    let thumbs: Vec<_> = pics.iter().map(|(_, im)| thumb_of(im)).collect();
    let n = pics.len();
    eprintln!();
    eprintln!("hash-grouped pairs and their fit:");
    let mut fits: Vec<(f32, usize, usize)> = Vec::new();
    for i in 0..n {
        for j in i + 1..n {
            let (d, p, q) = best_triple(&sigs[i], &sigs[j]);
            let f = fit(&thumbs[i], &thumbs[j]);
            fits.push((f.ncc, i, j));
            if d <= DHASH_MAX && p <= PHASH_MAX {
                eprintln!(
                    "  d {d:>2} p {p:>2} q {q:>3}   {}   {}  ~  {}",
                    f.label(),
                    pics[i].0,
                    pics[j].0
                );
            }
        }
    }
    fits.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
    eprintln!();
    eprintln!("the twelve best-fitting pairs of distinct files (whatever the hashes said):");
    for (c, i, j) in fits.iter().take(12) {
        let (d, p, q) = best_triple(&sigs[*i], &sigs[*j]);
        eprintln!(
            "  fit {c:.3}   d {d:>2} p {p:>2} q {q:>3}   {}  ~  {}",
            pics[*i].0, pics[*j].0
        );
    }
    eprintln!();
    eprintln!("sources whose plain 50 % rescale fits worst:");
    let mut worst: Vec<(f32, usize)> = (0..n)
        .map(|i| {
            let v = perturb(&pics[i].1, 0);
            (fit(&thumbs[i], &thumb_of(&v)).ncc, i)
        })
        .collect();
    worst.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    for (c, i) in worst.iter().take(5) {
        eprintln!(
            "  fit {c:.3}   {}   {}x{}",
            pics[*i].0,
            pics[*i].1.width(),
            pics[*i].1.height()
        );
    }
}

/// `DEDUPE_REAL_DIR=<folder> cargo test --release --test real_corpus -- --ignored measure_pdq_prep_real --nocapture`
#[test]
#[ignore]
fn measure_pdq_prep_real() {
    let Some(dir) = real_dir() else {
        eprintln!("DEDUPE_REAL_DIR not set; skipped");
        return;
    };
    let bases: Vec<RgbImage> = load_all(&dir).into_iter().map(|(_, im)| im).collect();
    for prep in [
        dedupe::perceptual::PdqPrep::Prefiltered,
        dedupe::perceptual::PdqPrep::Tool,
    ] {
        measure_pdq_prep_on("real photographs", &bases, 60, prep);
    }
}
