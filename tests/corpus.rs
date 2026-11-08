//! Threshold measurement corpus for the near-duplicate pass.
//!
//! `cargo test --release --test corpus -- --ignored --nocapture`
//!
//! Generates procedural photographs (layered value noise, soft shapes,
//! hard edges, tints), applies the perturbations a real library shows
//! (rescale, JPEG, crop, rotate, mirror, brightness, gamma, watermark,
//! blur, noise, letterbox, aspect squeeze), and measures, for the exact
//! matching rule the crate ships:
//!
//! - recall per perturbation (did the variant match its source?)
//! - false positives across every pair of distinct sources, and every
//!   variant against every other source
//! - a small threshold sweep so the shipped thresholds are a choice, not a
//!   guess
//!
//! The pictures are procedural, so the absolute rates are a floor for
//! photographs, which have more features and are harder to confuse; the
//! relative ranking of perturbations transfers.

use std::time::Instant;

use image::{DynamicImage, RgbImage};

use dedupe::hamming::{linear_pairs, mih_pairs, NoMatchProgress};
use dedupe::pdq::{PDQ_MAX, PDQ_QUALITY_MIN};
use dedupe::perceptual::{Signature, DHASH_MAX, PHASH_MAX};

mod common;
use common::*;

#[test]
#[ignore]
fn measure_thresholds() {
    const N: u64 = 150;
    let t0 = Instant::now();
    let bases: Vec<RgbImage> = (0..N).map(photo).collect();
    let base_sigs: Vec<Signature> = bases.iter().map(sig_of).collect();
    let gen_time = t0.elapsed();

    // variants: [base][perturbation]
    let t1 = Instant::now();
    let mut var_sigs: Vec<Vec<Signature>> = Vec::new();
    for b in &bases {
        var_sigs.push(
            (0..PERTURBATIONS.len())
                .map(|k| sig_of(&perturb(b, k)))
                .collect(),
        );
    }
    let hash_count = N as usize * (PERTURBATIONS.len() + 1);
    let per_image = t1.elapsed().as_secs_f64() / (N as f64 * PERTURBATIONS.len() as f64);

    // per-perturbation distances to source
    eprintln!();
    eprintln!("corpus: {N} sources × {} perturbations, {} signatures, generation {:.1}s, {:.1} ms per perturbed image incl. transform",
        PERTURBATIONS.len(), hash_count, gen_time.as_secs_f64(), per_image * 1000.0);
    eprintln!();
    eprintln!(
        "{:<14} {:>7} {:>7} {:>7} {:>7} {:>8}",
        "perturbation", "d max", "d p95", "p max", "p p95", "recall"
    );
    let mut all_var_dist: Vec<Vec<(u32, u32)>> = vec![Vec::new(); PERTURBATIONS.len()];
    for (i, vs) in var_sigs.iter().enumerate() {
        for (k, v) in vs.iter().enumerate() {
            all_var_dist[k].push(best_pair(&base_sigs[i], v));
        }
    }
    let mut recall_by_k = Vec::new();
    for (k, name) in PERTURBATIONS.iter().enumerate() {
        let mut ds: Vec<u32> = all_var_dist[k].iter().map(|x| x.0).collect();
        let mut ps: Vec<u32> = all_var_dist[k].iter().map(|x| x.1).collect();
        ds.sort_unstable();
        ps.sort_unstable();
        let p95 = |v: &Vec<u32>| v[(v.len() * 95 / 100).min(v.len() - 1)];
        let hit = all_var_dist[k]
            .iter()
            .filter(|(d, p)| matches(*d, *p, DHASH_MAX, PHASH_MAX))
            .count();
        let recall = hit as f64 / N as f64;
        recall_by_k.push(recall);
        eprintln!(
            "{:<14} {:>7} {:>7} {:>7} {:>7} {:>7.1}%",
            name,
            ds[ds.len() - 1],
            p95(&ds),
            ps[ps.len() - 1],
            p95(&ps),
            recall * 100.0
        );
    }

    // false positives: source vs source, and every variant vs every other source
    let mut neg: Vec<(u32, u32)> = Vec::new();
    for i in 0..N as usize {
        for j in i + 1..N as usize {
            neg.push(best_pair(&base_sigs[i], &base_sigs[j]));
            for v in &var_sigs[i] {
                neg.push(best_pair(v, &base_sigs[j]));
            }
            for v in &var_sigs[j] {
                neg.push(best_pair(v, &base_sigs[i]));
            }
        }
    }
    let fp = neg
        .iter()
        .filter(|(d, p)| matches(*d, *p, DHASH_MAX, PHASH_MAX))
        .count();
    let min_d = neg.iter().map(|x| x.0).min().unwrap();
    let min_p = neg.iter().map(|x| x.1).min().unwrap();
    let min_sum = neg.iter().map(|x| x.0 + x.1).min().unwrap();
    eprintln!();
    eprintln!("negatives: {} pairs   false positives at shipped thresholds (d≤{DHASH_MAX} ∧ p≤{PHASH_MAX}): {}   min d {}   min p {}   min d+p {}",
        neg.len(), fp, min_d, min_p, min_sum);

    // sweep
    eprintln!();
    eprintln!("threshold sweep (recall over all perturbations except rotate 2°/5° and crop 60%, which no block hash is expected to survive | false positives):");
    eprint!("{:<6}", "d\\p");
    for tp in [10u32, 12, 14, 16, 18, 20] {
        eprint!("{:>16}", format!("p≤{tp}"));
    }
    eprintln!();
    let survivable: Vec<usize> = (0..PERTURBATIONS.len())
        .filter(|k| ![8usize, 9, 10].contains(k))
        .collect();
    for td in [6u32, 8, 10, 12, 14, 16] {
        eprint!("d≤{:<4}", td);
        for tp in [10u32, 12, 14, 16, 18, 20] {
            let mut hit = 0usize;
            let mut tot = 0usize;
            for &k in &survivable {
                for (d, p) in &all_var_dist[k] {
                    tot += 1;
                    if matches(*d, *p, td, tp) {
                        hit += 1;
                    }
                }
            }
            let fp = neg.iter().filter(|(d, p)| matches(*d, *p, td, tp)).count();
            eprint!(
                "{:>16}",
                format!("{:.1}% | {}", hit as f64 / tot as f64 * 100.0, fp)
            );
        }
        eprintln!();
    }

    // ---- PDQ ----------------------------------------------------------
    eprintln!();
    eprintln!("PDQ (256-bit) on the same corpus; q = distance on the orientation d∧p chose, q* = best over orientations");
    eprintln!(
        "{:<14} {:>6} {:>6} {:>6} {:>8} {:>8} {:>9} {:>9}",
        "perturbation", "q max", "q p95", "q* max", "rec q≤31", "rec q*≤31", "d∧p∧q", "d∧p only"
    );
    let mut qual_below = 0usize;
    for s in &base_sigs {
        if s.pdq_quality < PDQ_QUALITY_MIN {
            qual_below += 1;
        }
    }
    let mut triples: Vec<Vec<(u32, u32, u32)>> = vec![Vec::new(); PERTURBATIONS.len()];
    let mut qstars: Vec<Vec<u32>> = vec![Vec::new(); PERTURBATIONS.len()];
    for (i, vs) in var_sigs.iter().enumerate() {
        for (k, v) in vs.iter().enumerate() {
            triples[k].push(best_triple(&base_sigs[i], v));
            qstars[k].push(best_q(&base_sigs[i], v));
        }
    }
    for (k, name) in PERTURBATIONS.iter().enumerate() {
        let mut qs: Vec<u32> = triples[k].iter().map(|t| t.2).collect();
        qs.sort_unstable();
        let mut qst = qstars[k].clone();
        qst.sort_unstable();
        let p95 = qs[(qs.len() * 95 / 100).min(qs.len() - 1)];
        let rec_q = triples[k].iter().filter(|t| t.2 <= PDQ_MAX).count() as f64 / N as f64;
        let rec_qs = qstars[k].iter().filter(|q| **q <= PDQ_MAX).count() as f64 / N as f64;
        let rec_all = triples[k]
            .iter()
            .filter(|t| matches(t.0, t.1, DHASH_MAX, PHASH_MAX) && t.2 <= PDQ_MAX)
            .count() as f64
            / N as f64;
        let rec_dp = triples[k]
            .iter()
            .filter(|t| matches(t.0, t.1, DHASH_MAX, PHASH_MAX))
            .count() as f64
            / N as f64;
        eprintln!(
            "{:<14} {:>6} {:>6} {:>6} {:>7.1}% {:>8.1}% {:>8.1}% {:>8.1}%",
            name,
            qs[qs.len() - 1],
            p95,
            qst[qst.len() - 1],
            rec_q * 100.0,
            rec_qs * 100.0,
            rec_all * 100.0,
            rec_dp * 100.0
        );
    }
    eprintln!("sources with PDQ quality below {PDQ_QUALITY_MIN}: {qual_below} of {N}");

    // PDQ negatives: q* over every negative pair
    let mut neg_q: Vec<u32> = Vec::with_capacity(neg.len());
    for i in 0..N as usize {
        for j in i + 1..N as usize {
            neg_q.push(best_q(&base_sigs[i], &base_sigs[j]));
            for v in &var_sigs[i] {
                neg_q.push(best_q(v, &base_sigs[j]));
            }
            for v in &var_sigs[j] {
                neg_q.push(best_q(v, &base_sigs[i]));
            }
        }
    }
    let mut sorted_q = neg_q.clone();
    sorted_q.sort_unstable();
    let mean_q = neg_q.iter().map(|q| *q as f64).sum::<f64>() / neg_q.len() as f64;
    eprintln!();
    eprintln!(
        "PDQ negatives: {} pairs   min q* {}   p0.1 {}   p1 {}   median {}   mean {:.1}",
        neg_q.len(),
        sorted_q[0],
        sorted_q[sorted_q.len() / 1000],
        sorted_q[sorted_q.len() / 100],
        sorted_q[sorted_q.len() / 2],
        mean_q
    );
    eprint!("PDQ-alone sweep (false positives at q*≤T):");
    for t in [24u32, 28, 31, 36, 40, 48, 56, 64] {
        eprint!("  T={t}: {}", neg_q.iter().filter(|q| **q <= t).count());
    }
    eprintln!();
    // PDQ-alone recall over survivable perturbations at the same thresholds
    eprint!("PDQ-alone recall over survivable perturbations:         ");
    for t in [24u32, 28, 31, 36, 40, 48, 56, 64] {
        let mut hit = 0usize;
        let mut tot = 0usize;
        for &k in &survivable {
            for q in &qstars[k] {
                tot += 1;
                if *q <= t {
                    hit += 1;
                }
            }
        }
        eprint!("  T={t}: {:.1}%", hit as f64 / tot as f64 * 100.0);
    }
    eprintln!();

    // engine agreement on real signatures
    let mut all: Vec<Signature> = base_sigs.clone();
    for vs in &var_sigs {
        all.extend(vs.iter().cloned());
    }
    let t = Instant::now();
    let lin = linear_pairs(&all, DHASH_MAX, PHASH_MAX, &NoMatchProgress);
    let tl = t.elapsed();
    let t = Instant::now();
    let mih = mih_pairs(&all, DHASH_MAX, PHASH_MAX, &NoMatchProgress);
    let tm = t.elapsed();
    eprintln!();
    eprintln!(
        "engines on {} real signatures: linear {:.3}s ({} pairs)   mih {:.3}s ({} pairs)",
        all.len(),
        tl.as_secs_f64(),
        lin.len(),
        tm.as_secs_f64(),
        mih.len()
    );
    let lk: Vec<(usize, usize)> = lin.iter().map(|m| (m.a, m.b)).collect();
    let mk: Vec<(usize, usize)> = mih.iter().map(|m| (m.a, m.b)).collect();
    assert_eq!(lk, mk, "both engines must agree on real signatures");
    eprintln!();
}

/// Deep tier: SSCD cosine on the same generator, fewer sources (each
/// embedding is ~0.3 s here). Runs only with the model installed:
/// `DEDUPE_MODEL=/path/sscd_disc_mixup.onnx cargo test --release --test corpus -- --ignored measure_deep --nocapture`
#[test]
#[ignore]
fn measure_deep() {
    use dedupe::deep::{cosine, installed, Deep, DEEP_MIN};
    let Some(path) = installed() else {
        eprintln!("no model installed; skipped");
        return;
    };
    let model = Deep::load(&path).expect("load model");
    const N: u64 = 40;
    let t0 = Instant::now();
    let bases: Vec<RgbImage> = (0..N).map(photo).collect();
    let embed = |img: &RgbImage| {
        model
            .embed(&DynamicImage::ImageRgb8(img.clone()))
            .expect("embed")
    };
    let base_e: Vec<_> = bases.iter().map(embed).collect();
    let mut var_e: Vec<Vec<[f32; 512]>> = Vec::new();
    for b in &bases {
        var_e.push(
            (0..PERTURBATIONS.len())
                .map(|k| embed(&perturb(b, k)))
                .collect(),
        );
    }
    let n_emb = N as usize * (PERTURBATIONS.len() + 1);
    eprintln!();
    eprintln!(
        "deep corpus: {N} sources × {} perturbations, {n_emb} embeddings in {:.0}s ({:.0} ms each incl. transform)",
        PERTURBATIONS.len(),
        t0.elapsed().as_secs_f64(),
        t0.elapsed().as_secs_f64() * 1000.0 / n_emb as f64
    );
    eprintln!();
    eprintln!(
        "{:<14} {:>7} {:>7} {:>7} {:>9}",
        "perturbation", "min", "p5", "median", "rec ≥min"
    );
    let pct = |v: &mut Vec<f32>, p: f64| {
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        v[((v.len() as f64 * p) as usize).min(v.len() - 1)]
    };
    let mut all_pos: Vec<(usize, f32)> = Vec::new();
    for (k, name) in PERTURBATIONS.iter().enumerate() {
        let mut v: Vec<f32> = (0..N as usize)
            .map(|i| cosine(&base_e[i], &var_e[i][k]))
            .collect();
        for s in &v {
            all_pos.push((k, *s));
        }
        let rec = v.iter().filter(|s| **s >= DEEP_MIN).count() as f64 / N as f64;
        let (mn, p5, med) = (pct(&mut v, 0.0), pct(&mut v, 0.05), pct(&mut v, 0.5));
        eprintln!(
            "{:<14} {:>7.3} {:>7.3} {:>7.3} {:>8.1}%",
            name,
            mn,
            p5,
            med,
            rec * 100.0
        );
    }
    let mut neg: Vec<f32> = Vec::new();
    for i in 0..N as usize {
        for j in i + 1..N as usize {
            neg.push(cosine(&base_e[i], &base_e[j]));
            for v in &var_e[i] {
                neg.push(cosine(v, &base_e[j]));
            }
            for v in &var_e[j] {
                neg.push(cosine(v, &base_e[i]));
            }
        }
    }
    let mut sorted = neg.clone();
    let (mx, p999, p99, med) = (
        pct(&mut sorted, 1.0),
        pct(&mut sorted, 0.999),
        pct(&mut sorted, 0.99),
        pct(&mut sorted, 0.5),
    );
    eprintln!();
    eprintln!(
        "deep negatives: {} pairs   max {:.3}   p99.9 {:.3}   p99 {:.3}   median {:.3}",
        neg.len(),
        mx,
        p999,
        p99,
        med
    );
    eprint!("sweep (false positives | recall over all perturbations | recall over crops+rotations only):");
    for t in [0.45f32, 0.5, 0.55, 0.6, 0.65, 0.7, 0.75, 0.8] {
        let fp = neg.iter().filter(|s| **s >= t).count();
        let rec = all_pos.iter().filter(|(_, s)| *s >= t).count() as f64 / all_pos.len() as f64;
        let hard: Vec<&(usize, f32)> = all_pos
            .iter()
            .filter(|(k, _)| [6usize, 7, 8, 9, 10].contains(k))
            .collect();
        let rec_hard = hard.iter().filter(|(_, s)| *s >= t).count() as f64 / hard.len() as f64;
        eprint!(
            "  T={t:.2}: {fp} | {:.1}% | {:.1}%",
            rec * 100.0,
            rec_hard * 100.0
        );
    }
    eprintln!();
    eprintln!("shipped DEEP_MIN = {DEEP_MIN}");
}

/// `cargo test --release --test corpus -- --ignored measure_geometry --nocapture`
#[test]
#[ignore]
fn measure_geometry() {
    let bases: Vec<RgbImage> = (0..150u64).map(photo).collect();
    measure_geometry_on("procedural", &bases, 60, 4000);
}

/// `cargo test --release --test corpus -- --ignored measure_pdq_prep --nocapture`
#[test]
#[ignore]
fn measure_pdq_prep() {
    let bases: Vec<RgbImage> = (0..150u64).map(photo).collect();
    for prep in [
        dedupe::perceptual::PdqPrep::Prefiltered,
        dedupe::perceptual::PdqPrep::Tool,
    ] {
        measure_pdq_prep_on("procedural", &bases, 60, prep);
    }
}
