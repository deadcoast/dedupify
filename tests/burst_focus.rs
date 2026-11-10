//! NF-05 (Phase 11): does the focus measure pick the frame a burst
//! pipeline picked? Google's HDR+ dataset (Hasinoff et al., SIGGRAPH Asia
//! 2016) records, for every burst, the reference frame its pipeline chose:
//! the sharpest of the first frames ("lucky imaging"). This compares the
//! variance of the Laplacian (Pech-Pacheco et al., ICPR 2000) — at the
//! scale the near tier hashes at, and at full resolution — with that
//! choice over frames 0–3. The measure is here, not in the crate: it did
//! not agree with the pipeline's choice better than taking the first
//! frame (VERIFICATION § Phase 11, NF-05), so nothing ships on it.
//!
//! ```text
//! DEDUPE_BURSTS=<folder of burst folders, each with payload_N000..N003.dng> \
//! DEDUPE_BURST_REFS=<file of "burst reference" lines> \
//!   cargo test --release --test burst_focus -- --ignored --nocapture
//! ```
//!
//! Nothing here is a unit test; it prints a record for VERIFICATION.md.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

use dedupe::perceptual::prefiltered_luma;

/// How sharply focused a luma plane is: the variance of its 3×3
/// Laplacian over the interior.
fn focus(pre: &[f32], w: usize, h: usize) -> f32 {
    if w < 3 || h < 3 {
        return 0.0;
    }
    let (mut n, mut s, mut ss) = (0.0f64, 0.0f64, 0.0f64);
    for y in 1..h - 1 {
        let r = y * w;
        for x in 1..w - 1 {
            let i = r + x;
            let l = (pre[i - 1] + pre[i + 1] + pre[i - w] + pre[i + w] - 4.0 * pre[i]) as f64;
            n += 1.0;
            s += l;
            ss += l * l;
        }
    }
    ((ss - s * s / n) / n) as f32
}

fn full_luma(img: &image::DynamicImage) -> (Vec<f32>, usize, usize) {
    let g = img.to_luma8();
    let (w, h) = (g.width() as usize, g.height() as usize);
    (g.as_raw().iter().map(|&v| v as f32).collect(), w, h)
}

fn argmax(v: &[f32]) -> usize {
    v.iter()
        .enumerate()
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
        .map(|(i, _)| i)
        .unwrap_or(0)
}

#[test]
#[ignore]
fn focus_against_hdrplus_reference_frames() {
    let (Some(dir), Some(refs)) = (
        std::env::var_os("DEDUPE_BURSTS").map(PathBuf::from),
        std::env::var_os("DEDUPE_BURST_REFS").map(PathBuf::from),
    ) else {
        eprintln!("DEDUPE_BURSTS / DEDUPE_BURST_REFS not set; skipped");
        return;
    };
    let labels: HashMap<String, usize> = std::fs::read_to_string(refs)
        .expect("refs")
        .lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            Some((it.next()?.to_string(), it.next()?.parse().ok()?))
        })
        .collect();
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .expect("bursts")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| labels.contains_key(n))
        .collect();
    names.sort();
    let t0 = Instant::now();
    let (mut n, mut hit_pre, mut hit_full, mut first) = (0, 0, 0, 0);
    let mut rank_pre = [0usize; 4];
    eprintln!();
    eprintln!("burst                          label  hashing-scale focus (×1)          pick  full-resolution pick");
    for name in &names {
        let label = labels[name];
        if label > 3 {
            continue;
        }
        let mut pre_f = Vec::new();
        let mut full_f = Vec::new();
        let mut ok = true;
        for k in 0..4 {
            let p = dir.join(name).join(format!("payload_N{k:03}.dng"));
            match dedupe::decode::load_image(&p) {
                Ok(img) => {
                    let (pre, w, h) = prefiltered_luma(&img);
                    pre_f.push(focus(&pre, w, h));
                    let (full, w, h) = full_luma(&img);
                    full_f.push(focus(&full, w, h));
                }
                Err(e) => {
                    eprintln!("  {name}: frame {k}: {e}");
                    ok = false;
                    break;
                }
            }
        }
        if !ok {
            continue;
        }
        n += 1;
        let (a, b) = (argmax(&pre_f), argmax(&full_f));
        hit_pre += (a == label) as usize;
        hit_full += (b == label) as usize;
        first += (label == 0) as usize;
        let rank = pre_f.iter().filter(|&&v| v > pre_f[label]).count();
        rank_pre[rank] += 1;
        eprintln!(
            "  {name:<28} {label}      {:>8.1} {:>8.1} {:>8.1} {:>8.1}   {a}     {b}",
            pre_f[0], pre_f[1], pre_f[2], pre_f[3]
        );
    }
    eprintln!();
    eprintln!(
        "{n} bursts: the hashing-scale focus picks the reference frame in {hit_pre} ({:.0} %), full resolution in {hit_full} ({:.0} %); always frame 0 would be right in {first} ({:.0} %), a random pick in 25 %",
        100.0 * hit_pre as f64 / n.max(1) as f64,
        100.0 * hit_full as f64 / n.max(1) as f64,
        100.0 * first as f64 / n.max(1) as f64
    );
    eprintln!(
        "the reference frame's rank by hashing-scale focus (0 = sharpest): {:?}   {:.0}s",
        rank_pre,
        t0.elapsed().as_secs_f64()
    );
}
