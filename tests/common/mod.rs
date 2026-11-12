//! Shared by the measurement harnesses (`corpus`, `real_corpus`): the
//! procedural photograph generator, the 21 perturbations a library shows,
//! and the distance helpers that mirror the matcher's view of a pair.
#![allow(dead_code)]

use image::{imageops::FilterType, DynamicImage, ImageBuffer, ImageEncoder, Rgb, RgbImage};

use dedupe::pdq::distance as pdq_distance;
use dedupe::perceptual::{hamming, signature_from_image, Signature, VARIANTS};

// ---------------------------------------------------------------------------
// deterministic RNG
// ---------------------------------------------------------------------------

pub struct Rng(pub u64);
impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    pub fn f(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }
    pub fn range(&mut self, lo: u32, hi: u32) -> u32 {
        lo + (self.next() % (hi - lo) as u64) as u32
    }
}

// ---------------------------------------------------------------------------
// procedural photographs
// ---------------------------------------------------------------------------

pub fn value_noise(rng: &mut Rng, w: u32, h: u32) -> Vec<f32> {
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

pub fn photo(seed: u64) -> RgbImage {
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

pub fn jpeg(img: &RgbImage, q: u8) -> RgbImage {
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

pub fn scale(img: &RgbImage, f: f32) -> RgbImage {
    let w = ((img.width() as f32 * f).round() as u32).max(8);
    let h = ((img.height() as f32 * f).round() as u32).max(8);
    image::imageops::resize(img, w, h, FilterType::Triangle)
}

pub fn crop_center(img: &RgbImage, keep: f32) -> RgbImage {
    let w = (img.width() as f32 * keep) as u32;
    let h = (img.height() as f32 * keep) as u32;
    let x = (img.width() - w) / 2;
    let y = (img.height() - h) / 2;
    image::imageops::crop_imm(img, x, y, w, h).to_image()
}

pub fn rotate_small(img: &RgbImage, deg: f32) -> RgbImage {
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

pub fn brightness(img: &RgbImage, f: f32) -> RgbImage {
    let mut out = img.clone();
    for p in out.pixels_mut() {
        for c in p.0.iter_mut() {
            *c = (*c as f32 * f).clamp(0.0, 255.0) as u8;
        }
    }
    out
}

pub fn gamma(img: &RgbImage, g: f32) -> RgbImage {
    let mut out = img.clone();
    for p in out.pixels_mut() {
        for c in p.0.iter_mut() {
            *c = (255.0 * (*c as f32 / 255.0).powf(g)).clamp(0.0, 255.0) as u8;
        }
    }
    out
}

pub fn watermark(img: &RgbImage) -> RgbImage {
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

pub fn blur(img: &RgbImage) -> RgbImage {
    let b = image::imageops::blur(img, 1.2);
    image::imageops::blur(&b, 1.2)
}

pub fn noise(img: &RgbImage, amp: i32) -> RgbImage {
    let mut rng = Rng(4242);
    let mut out = img.clone();
    for p in out.pixels_mut() {
        for c in p.0.iter_mut() {
            let d = (rng.next() % (2 * amp as u64 + 1)) as i32 - amp;
            *c = (*c as i32 + d).clamp(0, 255) as u8;
        }
    }
    out
}

pub fn letterbox(img: &RgbImage) -> RgbImage {
    let (w, h) = (img.width(), img.height());
    let bar = h / 10;
    let mut out: RgbImage = ImageBuffer::from_pixel(w, h + 2 * bar, Rgb([0, 0, 0]));
    image::imageops::overlay(&mut out, img, 0, bar as i64);
    out
}

pub fn squeeze(img: &RgbImage) -> RgbImage {
    image::imageops::resize(
        img,
        (img.width() * 3 / 4).max(8),
        img.height(),
        FilterType::Triangle,
    )
}

pub const PERTURBATIONS: &[&str] = &[
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

pub fn perturb(img: &RgbImage, which: usize) -> RgbImage {
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

pub fn sig_of(img: &RgbImage) -> Signature {
    signature_from_image(&DynamicImage::ImageRgb8(img.clone()))
}

/// (d, p, q) on the best (lowest d+p) orientation — the relation the
/// matcher reports; q is the PDQ distance on that same orientation.
pub fn best_triple(a: &Signature, b: &Signature) -> (u32, u32, u32) {
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
pub fn best_q(a: &Signature, b: &Signature) -> u32 {
    let mut best = u32::MAX;
    for k in 0..VARIANTS {
        best = best
            .min(pdq_distance(&a.pdq[0], &b.pdq[k]))
            .min(pdq_distance(&b.pdq[0], &a.pdq[k]));
    }
    best
}

/// Best (d, p) over variants and both directions — the matcher's view.
pub fn best_pair(a: &Signature, b: &Signature) -> (u32, u32) {
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

pub fn matches(d: u32, p: u32, td: u32, tp: u32) -> bool {
    d <= td && p <= tp
}

// ---------------------------------------------------------------------------
// geometric fit helpers
// ---------------------------------------------------------------------------

pub fn thumb_of(img: &RgbImage) -> dedupe::geometry::Thumb {
    dedupe::geometry::Thumb::from_image(
        &DynamicImage::ImageRgb8(img.clone()),
        dedupe::geometry::MAX_SIDE,
    )
}

/// Percentile of a sorted-in-place vector (p in [0, 1]).
pub fn pct(v: &mut [f32], p: f64) -> f32 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[((v.len() as f64 * p) as usize).min(v.len() - 1)]
}

/// The geometry measurement shared by both corpora: positives per
/// perturbation (fit min / p5 / median, recall at FIT_MIN) and negatives
/// (fit over unrelated source pairs: max, p99.9, p99, median) with a sweep.
pub fn measure_geometry_on(
    label: &str,
    bases: &[RgbImage],
    positives_cap: usize,
    negative_pairs_cap: usize,
) {
    use dedupe::geometry::{fit, FIT_MIN};
    let t0 = std::time::Instant::now();
    let thumbs: Vec<_> = bases.iter().map(thumb_of).collect();
    let m = bases.len().min(positives_cap);
    eprintln!();
    eprintln!(
        "geometry on {label}: {} sources, positives from the first {m}",
        bases.len()
    );
    eprintln!(
        "{:<14} {:>6} {:>6} {:>6} {:>9}",
        "perturbation", "min", "p5", "median", "rec>=min"
    );
    let mut all_pos: Vec<(usize, f32)> = Vec::new();
    for (k, name) in PERTURBATIONS.iter().enumerate() {
        let mut v: Vec<f32> = (0..m)
            .map(|i| fit(&thumbs[i], &thumb_of(&perturb(&bases[i], k))).ncc)
            .collect();
        for s in &v {
            all_pos.push((k, *s));
        }
        let rec = v.iter().filter(|s| **s >= FIT_MIN).count() as f64 / m as f64;
        let (mn, p5, med) = (pct(&mut v, 0.0), pct(&mut v, 0.05), pct(&mut v, 0.5));
        eprintln!(
            "{name:<14} {mn:>6.3} {p5:>6.3} {med:>6.3} {:>8.1}%",
            rec * 100.0
        );
    }
    // negatives: distinct source pairs, up to the cap (deterministic stride)
    let n = bases.len();
    let total_pairs = n * (n - 1) / 2;
    let stride = (total_pairs / negative_pairs_cap.max(1)).max(1);
    let mut neg: Vec<f32> = Vec::new();
    let mut idx = 0usize;
    let mut worst: Vec<(f32, usize, usize)> = Vec::new();
    for i in 0..n {
        for j in i + 1..n {
            if idx.is_multiple_of(stride) {
                let c = fit(&thumbs[i], &thumbs[j]).ncc;
                neg.push(c);
                worst.push((c, i, j));
            }
            idx += 1;
        }
    }
    worst.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
    let mut sorted = neg.clone();
    eprintln!();
    eprintln!(
        "negatives: {} source pairs   max {:.3}   p99.9 {:.3}   p99 {:.3}   median {:.3}",
        neg.len(),
        pct(&mut sorted, 1.0),
        pct(&mut sorted, 0.999),
        pct(&mut sorted, 0.99),
        pct(&mut sorted, 0.5)
    );
    eprintln!(
        "worst five negatives (fit, source i, source j): {:?}",
        &worst[..worst.len().min(5)]
    );
    eprint!(
        "sweep (false positives | recall over all perturbations | over crops+rotations+letterbox):"
    );
    for t in [0.80f32, 0.85, 0.88, 0.90, 0.92, 0.95] {
        let fp = neg.iter().filter(|s| **s >= t).count();
        let rec = all_pos.iter().filter(|(_, s)| *s >= t).count() as f64 / all_pos.len() as f64;
        let hard: Vec<&(usize, f32)> = all_pos
            .iter()
            .filter(|(k, _)| [6usize, 7, 8, 9, 10, 19].contains(k))
            .collect();
        let rec_hard =
            hard.iter().filter(|(_, s)| *s >= t).count() as f64 / hard.len().max(1) as f64;
        eprint!(
            "  T={t:.2}: {fp} | {:.1}% | {:.1}%",
            rec * 100.0,
            rec_hard * 100.0
        );
    }
    eprintln!();
    eprintln!(
        "shipped FIT_MIN = {FIT_MIN}   {:.0}s",
        t0.elapsed().as_secs_f64()
    );
}

/// PDQ alone, under a chosen input preparation: per perturbation the
/// distance on the matcher's orientation (max, p95, recall at PDQ_MAX),
/// and the negatives' closest pair. Shared by both corpora.
pub fn measure_pdq_prep_on(
    label: &str,
    bases: &[RgbImage],
    positives_cap: usize,
    prep: dedupe::perceptual::PdqPrep,
) {
    use dedupe::pdq::PDQ_MAX;
    use dedupe::perceptual::signature_from_image_with;
    let t0 = std::time::Instant::now();
    let sig = |im: &RgbImage| signature_from_image_with(&DynamicImage::ImageRgb8(im.clone()), prep);
    let sigs: Vec<Signature> = bases.iter().map(sig).collect();
    let m = bases.len().min(positives_cap);
    eprintln!();
    eprintln!(
        "PDQ under {prep:?} on {label}: {} sources, positives from the first {m}",
        bases.len()
    );
    eprintln!(
        "{:<14} {:>5} {:>5} {:>8}   {:>8}",
        "perturbation", "q max", "q p95", "q<=max", "quality"
    );
    for (k, name) in PERTURBATIONS.iter().enumerate() {
        let mut qs: Vec<u32> = Vec::new();
        let mut qual_min = 100u8;
        for i in 0..m {
            let v = sig(&perturb(&bases[i], k));
            let (_, _, q) = best_triple(&sigs[i], &v);
            qs.push(q);
            qual_min = qual_min.min(v.pdq_quality);
        }
        qs.sort();
        let rec = qs.iter().filter(|q| **q <= PDQ_MAX).count() as f64 / m as f64;
        eprintln!(
            "{name:<14} {:>5} {:>5} {:>7.1}%   {:>8}",
            qs[qs.len() - 1],
            qs[(qs.len() as f64 * 0.95) as usize],
            rec * 100.0,
            qual_min
        );
    }
    let n = bases.len();
    let mut min_q = u32::MAX;
    let mut under = 0usize;
    for i in 0..n {
        for j in i + 1..n {
            let q = best_q(&sigs[i], &sigs[j]);
            min_q = min_q.min(q);
            if q <= PDQ_MAX {
                under += 1;
            }
        }
    }
    eprintln!(
        "negatives: {} pairs   closest q {min_q}   at or under {PDQ_MAX}: {under}   {:.0}s",
        n * (n - 1) / 2,
        t0.elapsed().as_secs_f64()
    );
}
