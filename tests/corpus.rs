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

use image::{imageops::FilterType, DynamicImage, ImageBuffer, ImageEncoder, Rgb, RgbImage};

use dedupe::hamming::{linear_pairs, mih_pairs, NoMatchProgress};
use dedupe::pdq::{distance as pdq_distance, PDQ_MAX, PDQ_QUALITY_MIN};
use dedupe::perceptual::{
    hamming, signature_from_image, Signature, DHASH_MAX, PHASH_MAX, VARIANTS,
};

// ---------------------------------------------------------------------------
// deterministic RNG
// ---------------------------------------------------------------------------

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn f(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }
    fn range(&mut self, lo: u32, hi: u32) -> u32 {
        lo + (self.next() % (hi - lo) as u64) as u32
    }
}

// ---------------------------------------------------------------------------
// procedural photographs
// ---------------------------------------------------------------------------

fn value_noise(rng: &mut Rng, w: u32, h: u32) -> Vec<f32> {
    let mut out = vec![0.0f32; (w * h) as usize];
    let mut amp = 1.0f32;
    let mut total = 0.0f32;
    for cells in [3u32, 6, 12, 24, 48] {
        let gw = cells + 1;
        let gh = cells + 1;
        let grid: Vec<f32> = (0..gw * gh).map(|_| rng.f()).collect();
        for y in 0..h {
            let fy = y as f32 / h as f32 * cells as f32;
            let y0 = fy.floor() as u32;
            let ty = fy - y0 as f32;
            let ty = ty * ty * (3.0 - 2.0 * ty);
            for x in 0..w {
                let fx = x as f32 / w as f32 * cells as f32;
                let x0 = fx.floor() as u32;
                let tx = fx - x0 as f32;
                let tx = tx * tx * (3.0 - 2.0 * tx);
                let g = |xx: u32, yy: u32| grid[(yy.min(gh - 1) * gw + xx.min(gw - 1)) as usize];
                let v = g(x0, y0) * (1.0 - tx) * (1.0 - ty)
                    + g(x0 + 1, y0) * tx * (1.0 - ty)
                    + g(x0, y0 + 1) * (1.0 - tx) * ty
                    + g(x0 + 1, y0 + 1) * tx * ty;
                out[(y * w + x) as usize] += v * amp;
            }
        }
        total += amp;
        amp *= 0.5;
    }
    for v in &mut out {
        *v /= total;
    }
    out
}

fn photo(seed: u64) -> RgbImage {
    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let (w, h) = match rng.range(0, 4) {
        0 => (640, 480),
        1 => (800, 600),
        2 => (1024, 768),
        _ => (900, 1200),
    };
    let base = value_noise(&mut rng, w, h);
    let tint = [rng.f(), rng.f(), rng.f()];
    let grad_dir = rng.f() * std::f32::consts::TAU;
    let mut img: RgbImage = ImageBuffer::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let n = base[(y * w + x) as usize];
            let gx = x as f32 / w as f32 - 0.5;
            let gy = y as f32 / h as f32 - 0.5;
            let grad = 0.5 + 0.35 * (gx * grad_dir.cos() + gy * grad_dir.sin());
            let v = 0.55 * n + 0.45 * grad;
            let r = (255.0 * (v * (0.6 + 0.4 * tint[0]))).clamp(0.0, 255.0);
            let g = (255.0 * (v * (0.6 + 0.4 * tint[1]))).clamp(0.0, 255.0);
            let b = (255.0 * (v * (0.6 + 0.4 * tint[2]))).clamp(0.0, 255.0);
            img.put_pixel(x, y, Rgb([r as u8, g as u8, b as u8]));
        }
    }
    // soft ellipses
    for _ in 0..rng.range(2, 6) {
        let cx = rng.f() * w as f32;
        let cy = rng.f() * h as f32;
        let rx = (0.05 + 0.2 * rng.f()) * w as f32;
        let ry = (0.05 + 0.2 * rng.f()) * h as f32;
        let col = [rng.f(), rng.f(), rng.f()];
        let strength = 0.4 + 0.5 * rng.f();
        for y in 0..h {
            for x in 0..w {
                let dx = (x as f32 - cx) / rx;
                let dy = (y as f32 - cy) / ry;
                let d = dx * dx + dy * dy;
                if d < 1.0 {
                    let a = strength * (1.0 - d).powf(0.5);
                    let p = img.get_pixel_mut(x, y);
                    for (ch, c) in p.0.iter_mut().zip(col.iter()) {
                        *ch = (*ch as f32 * (1.0 - a) + 255.0 * c * a) as u8;
                    }
                }
            }
        }
    }
    // hard-edged rectangles
    for _ in 0..rng.range(1, 4) {
        let x0 = rng.range(0, w - 40);
        let y0 = rng.range(0, h - 40);
        let x1 = (x0 + rng.range(20, w / 3)).min(w - 1);
        let y1 = (y0 + rng.range(20, h / 3)).min(h - 1);
        let col = [
            rng.range(0, 255) as u8,
            rng.range(0, 255) as u8,
            rng.range(0, 255) as u8,
        ];
        for y in y0..y1 {
            for x in x0..x1 {
                img.put_pixel(x, y, Rgb(col));
            }
        }
    }
    img
}

// ---------------------------------------------------------------------------
// perturbations
// ---------------------------------------------------------------------------

fn jpeg(img: &RgbImage, q: u8) -> RgbImage {
    let mut buf = std::io::Cursor::new(Vec::new());
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, q)
        .write_image(
            img.as_raw(),
            img.width(),
            img.height(),
            image::ExtendedColorType::Rgb8,
        )
        .unwrap();
    image::load_from_memory(buf.get_ref()).unwrap().to_rgb8()
}

fn scale(img: &RgbImage, f: f32) -> RgbImage {
    let w = ((img.width() as f32 * f).round() as u32).max(8);
    let h = ((img.height() as f32 * f).round() as u32).max(8);
    image::imageops::resize(img, w, h, FilterType::Triangle)
}

fn crop_center(img: &RgbImage, keep: f32) -> RgbImage {
    let w = (img.width() as f32 * keep) as u32;
    let h = (img.height() as f32 * keep) as u32;
    let x = (img.width() - w) / 2;
    let y = (img.height() - h) / 2;
    image::imageops::crop_imm(img, x, y, w, h).to_image()
}

fn rotate_small(img: &RgbImage, deg: f32) -> RgbImage {
    let (w, h) = (img.width(), img.height());
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    let (s, c) = deg.to_radians().sin_cos();
    let mut mean = [0u64; 3];
    for p in img.pixels() {
        for (m, v) in mean.iter_mut().zip(p.0.iter()) {
            *m += *v as u64;
        }
    }
    let n = (w * h) as u64;
    let fill = Rgb([
        (mean[0] / n) as u8,
        (mean[1] / n) as u8,
        (mean[2] / n) as u8,
    ]);
    ImageBuffer::from_fn(w, h, |x, y| {
        let dx = x as f32 - cx;
        let dy = y as f32 - cy;
        let sx = cx + dx * c + dy * s;
        let sy = cy - dx * s + dy * c;
        if sx < 0.0 || sy < 0.0 || sx >= (w - 1) as f32 || sy >= (h - 1) as f32 {
            return fill;
        }
        let x0 = sx.floor() as u32;
        let y0 = sy.floor() as u32;
        let tx = sx - x0 as f32;
        let ty = sy - y0 as f32;
        let p00 = img.get_pixel(x0, y0).0;
        let p10 = img.get_pixel(x0 + 1, y0).0;
        let p01 = img.get_pixel(x0, y0 + 1).0;
        let p11 = img.get_pixel(x0 + 1, y0 + 1).0;
        let mut out = [0u8; 3];
        for k in 0..3 {
            let v = p00[k] as f32 * (1.0 - tx) * (1.0 - ty)
                + p10[k] as f32 * tx * (1.0 - ty)
                + p01[k] as f32 * (1.0 - tx) * ty
                + p11[k] as f32 * tx * ty;
            out[k] = v.round() as u8;
        }
        Rgb(out)
    })
}

fn brightness(img: &RgbImage, f: f32) -> RgbImage {
    let mut out = img.clone();
    for p in out.pixels_mut() {
        for k in 0..3 {
            p.0[k] = (p.0[k] as f32 * f).clamp(0.0, 255.0) as u8;
        }
    }
    out
}

fn gamma(img: &RgbImage, g: f32) -> RgbImage {
    let mut out = img.clone();
    for p in out.pixels_mut() {
        for k in 0..3 {
            p.0[k] = (255.0 * (p.0[k] as f32 / 255.0).powf(g)).clamp(0.0, 255.0) as u8;
        }
    }
    out
}

fn watermark(img: &RgbImage) -> RgbImage {
    let mut out = img.clone();
    let (w, h) = (img.width(), img.height());
    let bw = w * 18 / 100;
    let bh = h * 8 / 100;
    let x0 = w - bw - w / 40;
    let y0 = h - bh - h / 40;
    for y in y0..y0 + bh {
        for x in x0..x0 + bw {
            out.put_pixel(x, y, Rgb([245, 245, 245]));
        }
    }
    // text-like dark strokes
    let mut rng = Rng(77);
    for _ in 0..6 {
        let sx = x0 + rng.range(2, bw - 6);
        let sw = rng.range(2, 5);
        for y in y0 + bh / 4..y0 + bh * 3 / 4 {
            for x in sx..sx + sw {
                out.put_pixel(x, y, Rgb([30, 30, 30]));
            }
        }
    }
    out
}

fn blur(img: &RgbImage) -> RgbImage {
    let b = image::imageops::blur(img, 1.2);
    image::imageops::blur(&b, 1.2)
}

fn noise(img: &RgbImage, amp: i32) -> RgbImage {
    let mut rng = Rng(4242);
    let mut out = img.clone();
    for p in out.pixels_mut() {
        for k in 0..3 {
            let d = (rng.next() % (2 * amp as u64 + 1)) as i32 - amp;
            p.0[k] = (p.0[k] as i32 + d).clamp(0, 255) as u8;
        }
    }
    out
}

fn letterbox(img: &RgbImage) -> RgbImage {
    let (w, h) = (img.width(), img.height());
    let bar = h / 10;
    let mut out: RgbImage = ImageBuffer::from_pixel(w, h + 2 * bar, Rgb([0, 0, 0]));
    image::imageops::overlay(&mut out, img, 0, bar as i64);
    out
}

fn squeeze(img: &RgbImage) -> RgbImage {
    image::imageops::resize(
        img,
        (img.width() * 3 / 4).max(8),
        img.height(),
        FilterType::Triangle,
    )
}

const PERTURBATIONS: &[&str] = &[
    "scale 50%",
    "scale 25%",
    "upscale 150%",
    "jpeg q90",
    "jpeg q60",
    "jpeg q30",
    "crop 90%",
    "crop 80%",
    "crop 60%",
    "rotate 2°",
    "rotate 5°",
    "rotate 90°",
    "mirror",
    "bright +20%",
    "bright -20%",
    "gamma 1.5",
    "watermark",
    "blur",
    "noise ±10",
    "letterbox",
    "squeeze 75%",
];

fn perturb(img: &RgbImage, which: usize) -> RgbImage {
    match which {
        0 => scale(img, 0.5),
        1 => scale(img, 0.25),
        2 => scale(img, 1.5),
        3 => jpeg(img, 90),
        4 => jpeg(img, 60),
        5 => jpeg(img, 30),
        6 => crop_center(img, 0.9),
        7 => crop_center(img, 0.8),
        8 => crop_center(img, 0.6),
        9 => rotate_small(img, 2.0),
        10 => rotate_small(img, 5.0),
        11 => image::imageops::rotate90(img),
        12 => image::imageops::flip_horizontal(img),
        13 => brightness(img, 1.2),
        14 => brightness(img, 0.8),
        15 => gamma(img, 1.5),
        16 => watermark(img),
        17 => blur(img),
        18 => noise(img, 10),
        19 => letterbox(img),
        20 => squeeze(img),
        _ => unreachable!(),
    }
}

// ---------------------------------------------------------------------------

fn sig_of(img: &RgbImage) -> Signature {
    signature_from_image(&DynamicImage::ImageRgb8(img.clone()))
}

/// (d, p, q) on the best (lowest d+p) orientation — the relation the
/// matcher reports; q is the PDQ distance on that same orientation.
fn best_triple(a: &Signature, b: &Signature) -> (u32, u32, u32) {
    let mut best = (u32::MAX, u32::MAX, u32::MAX);
    let mut best_sum = u32::MAX;
    for k in 0..VARIANTS {
        for (d, p, q) in [
            (
                hamming(a.dhash[0], b.dhash[k]),
                hamming(a.phash[0], b.phash[k]),
                pdq_distance(&a.pdq[0], &b.pdq[k]),
            ),
            (
                hamming(b.dhash[0], a.dhash[k]),
                hamming(b.phash[0], a.phash[k]),
                pdq_distance(&b.pdq[0], &a.pdq[k]),
            ),
        ] {
            if d + p < best_sum {
                best_sum = d + p;
                best = (d, p, q);
            }
        }
    }
    best
}

/// Lowest PDQ distance over orientations and directions — PDQ on its own.
fn best_q(a: &Signature, b: &Signature) -> u32 {
    let mut best = u32::MAX;
    for k in 0..VARIANTS {
        best = best
            .min(pdq_distance(&a.pdq[0], &b.pdq[k]))
            .min(pdq_distance(&b.pdq[0], &a.pdq[k]));
    }
    best
}

/// Best (d, p) over variants and both directions — the matcher's view.
fn best_pair(a: &Signature, b: &Signature) -> (u32, u32) {
    let mut best = (u32::MAX, u32::MAX);
    let mut best_sum = u32::MAX;
    for k in 0..VARIANTS {
        for (d, p) in [
            (
                hamming(a.dhash[0], b.dhash[k]),
                hamming(a.phash[0], b.phash[k]),
            ),
            (
                hamming(b.dhash[0], a.dhash[k]),
                hamming(b.phash[0], a.phash[k]),
            ),
        ] {
            if d + p < best_sum {
                best_sum = d + p;
                best = (d, p);
            }
        }
    }
    best
}

fn matches(d: u32, p: u32, td: u32, tp: u32) -> bool {
    d <= td && p <= tp
}

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
