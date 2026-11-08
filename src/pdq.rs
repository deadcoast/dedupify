//! PDQ — 256-bit perceptual hash (Meta, 2019), implemented from the
//! reference C++ (`pdq/cpp/hashing/pdqhashing.cpp`, `downscaling.cpp`):
//!
//! 1. luminance (Rec. 601, the same coefficients as `perceptual.rs`)
//! 2. Jarosz filter: two passes of a horizontal then vertical box filter,
//!    window = ⌈dim / 128⌉ per axis — two box passes make a tent filter
//! 3. decimate to 64×64 by sampling the centre of each output cell
//! 4. quality: sum of |⌊(u − v)·100/255⌋| over adjacent pairs, / 90, cap 100
//! 5. DCT: 16 rows of the 64-point DCT-II basis, rows 1..=16 (DC skipped),
//!    B = D·A·Dᵀ, 16×16
//! 6. bit = value > median of the 256 values; bit index row·16 + col
//! 7. seven dihedral variants in the DCT domain by sign flips and transposes
//!
//! Where this differs from the reference **[CLAUDE]**: the Jarosz filter
//! runs on the integer-box-prefiltered luminance that `perceptual.rs`
//! already produces (short side kept between 576 and 1151) instead of the
//! full-resolution image. A 24-megapixel file would otherwise need two
//! 96 MB float buffers per image under a parallel decode. The prefilter is
//! itself a box average, which the Jarosz passes would mostly subsume;
//! hashes are therefore not bit-identical to the reference tool's, and
//! `dedupe` never needs them to be — it compares its own hashes with its
//! own hashes.

use crate::perceptual::{Variant, VARIANTS, VARIANT_ORDER};

pub const SIDE: usize = 64;
pub const BAND: usize = 16;
pub const BITS: usize = 256;
pub const JAROSZ_PASSES: usize = 2;
pub const MIN_DIM: usize = 5;

/// Recommended starting threshold in the PDQ documentation ("<=31"), kept
/// after the corpus measurement in docs/VERIFICATION.md.
pub const PDQ_MAX: u32 = 31;
/// Inputs with a short side below this are Lanczos-upscaled by an integer
/// factor first **[CLAUDE]**: below 128 px the reference window is 1 (no
/// filtering) and centre-of-cell sampling lands up to half an output cell
/// off. On the corpus (docs/VERIFICATION.md) this took 25 % thumbnails
/// from a worst case of 32 bits (one miss at the threshold) to 14, and
/// 384 measured better than 256 on every row and no worse than 512 on the
/// rest.
pub const MIN_SIDE: usize = 384;
/// The documentation recommends discarding hashes of quality ≤ 49.
pub const PDQ_QUALITY_MIN: u8 = 50;

/// 256 bits as four words; bit b lives at word b / 64, position b % 64
/// counted from the most significant end.
pub type Hash256 = [u64; 4];

#[inline]
pub fn distance(a: &Hash256, b: &Hash256) -> u32 {
    (a[0] ^ b[0]).count_ones()
        + (a[1] ^ b[1]).count_ones()
        + (a[2] ^ b[2]).count_ones()
        + (a[3] ^ b[3]).count_ones()
}

pub fn hex(h: &Hash256) -> String {
    h.iter().map(|w| format!("{w:016x}")).collect()
}

/// Luminance plane, row-major f32.
pub struct Luma<'a> {
    pub data: &'a [f32],
    pub width: usize,
    pub height: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pdq {
    /// One hash per `VARIANT_ORDER` entry.
    pub hashes: [Hash256; VARIANTS],
    pub quality: u8,
}

/// `(old + 2·new − 1) / (2·new)` — ceiling of old / (2·new).
pub fn window_size(old: usize, new: usize) -> usize {
    old.div_ceil(2 * new)
}

/// One box pass along a vector, reference edge handling: output i averages
/// the inputs from i − (w − h) to i + (h − 1), clipped to the vector, where
/// h = (w + 2) / 2, divided by the count actually included.
pub fn box_1d(input: &[f32], output: &mut [f32], w: usize) {
    let n = input.len();
    if w <= 1 || n == 0 {
        output.copy_from_slice(input);
        return;
    }
    let h = (w + 2) / 2;
    let left = w - h; // samples to the left of i
    let right = h - 1; // samples to the right of i
    let mut sum = 0.0f32;
    let mut lo = 0usize; // window start (inclusive)
    let mut hi = 0usize; // window end (exclusive)
    for (i, out) in output.iter_mut().enumerate() {
        let want_lo = i.saturating_sub(left);
        let want_hi = (i + right + 1).min(n);
        while hi < want_hi {
            sum += input[hi];
            hi += 1;
        }
        while lo < want_lo {
            sum -= input[lo];
            lo += 1;
        }
        *out = sum * (1.0 / (hi - lo) as f32);
    }
}

fn box_rows(input: &[f32], output: &mut [f32], rows: usize, cols: usize, w: usize) {
    for r in 0..rows {
        box_1d(
            &input[r * cols..(r + 1) * cols],
            &mut output[r * cols..(r + 1) * cols],
            w,
        );
    }
}

/// Vertical box pass over all columns at once: one running sum per column,
/// rows added and removed in order, so memory is touched sequentially.
fn box_cols(input: &[f32], output: &mut [f32], rows: usize, cols: usize, w: usize) {
    if w <= 1 || rows == 0 {
        output.copy_from_slice(input);
        return;
    }
    let h = (w + 2) / 2;
    let left = w - h;
    let right = h - 1;
    let mut sum = vec![0.0f32; cols];
    let mut lo = 0usize;
    let mut hi = 0usize;
    for i in 0..rows {
        let want_lo = i.saturating_sub(left);
        let want_hi = (i + right + 1).min(rows);
        while hi < want_hi {
            let row = &input[hi * cols..(hi + 1) * cols];
            for (s, v) in sum.iter_mut().zip(row) {
                *s += *v;
            }
            hi += 1;
        }
        while lo < want_lo {
            let row = &input[lo * cols..(lo + 1) * cols];
            for (s, v) in sum.iter_mut().zip(row) {
                *s -= *v;
            }
            lo += 1;
        }
        let inv = 1.0 / (hi - lo) as f32;
        let out = &mut output[i * cols..(i + 1) * cols];
        for (o, s) in out.iter_mut().zip(&sum) {
            *o = *s * inv;
        }
    }
}

/// Jarosz filter then decimation to SIDE×SIDE.
pub fn downsample(l: &Luma) -> Vec<f32> {
    let (rows, cols) = (l.height, l.width);
    if rows == SIDE && cols == SIDE {
        return l.data.to_vec();
    }
    let w_rows = window_size(cols, SIDE); // along a row: horizontal extent
    let w_cols = window_size(rows, SIDE);
    let mut a = l.data.to_vec();
    let mut b = vec![0.0f32; rows * cols];
    for _ in 0..JAROSZ_PASSES {
        box_rows(&a, &mut b, rows, cols, w_rows);
        box_cols(&b, &mut a, rows, cols, w_cols);
    }
    let mut out = vec![0.0f32; SIDE * SIDE];
    for (oi, row) in out.chunks_mut(SIDE).enumerate() {
        let ii = ((oi as f64 + 0.5) * rows as f64 / SIDE as f64) as usize;
        for (oj, v) in row.iter_mut().enumerate() {
            let ij = ((oj as f64 + 0.5) * cols as f64 / SIDE as f64) as usize;
            *v = a[ii.min(rows - 1) * cols + ij.min(cols - 1)];
        }
    }
    out
}

/// Reference quality: gradient energy of the 64×64 buffer on a 0–100 scale.
pub fn quality(buf: &[f32]) -> u8 {
    let mut sum: i64 = 0;
    for i in 0..SIDE - 1 {
        for j in 0..SIDE {
            let d = ((buf[i * SIDE + j] - buf[(i + 1) * SIDE + j]) * 100.0 / 255.0) as i32;
            sum += d.unsigned_abs() as i64;
        }
    }
    for i in 0..SIDE {
        for j in 0..SIDE - 1 {
            let d = ((buf[i * SIDE + j] - buf[i * SIDE + j + 1]) * 100.0 / 255.0) as i32;
            sum += d.unsigned_abs() as i64;
        }
    }
    (sum / 90).min(100) as u8
}

/// 16×64 DCT basis, rows 1..=16: D[i][j] = √(2/64)·cos(π/128·(i+1)·(2j+1)).
fn dct_basis() -> &'static [f32] {
    static M: std::sync::OnceLock<Vec<f32>> = std::sync::OnceLock::new();
    M.get_or_init(|| {
        let scale = (2.0f64 / SIDE as f64).sqrt();
        let mut m = vec![0.0f32; BAND * SIDE];
        for i in 0..BAND {
            for j in 0..SIDE {
                let v = scale
                    * (std::f64::consts::PI / (2.0 * SIDE as f64)
                        * (i as f64 + 1.0)
                        * (2.0 * j as f64 + 1.0))
                        .cos();
                m[i * SIDE + j] = v as f32;
            }
        }
        m
    })
}

/// B = D·A·Dᵀ for the 64×64 buffer A, giving the 16×16 band.
pub fn dct16(buf: &[f32]) -> [f32; BAND * BAND] {
    let d = dct_basis();
    let mut t = vec![0.0f32; BAND * SIDE]; // D·A
    for i in 0..BAND {
        for j in 0..SIDE {
            let mut acc = 0.0f32;
            for k in 0..SIDE {
                acc += d[i * SIDE + k] * buf[k * SIDE + j];
            }
            t[i * SIDE + j] = acc;
        }
    }
    let mut b = [0.0f32; BAND * BAND]; // T·Dᵀ
    for i in 0..BAND {
        for j in 0..BAND {
            let mut acc = 0.0f32;
            for k in 0..SIDE {
                acc += t[i * SIDE + k] * d[j * SIDE + k];
            }
            b[i * BAND + j] = acc;
        }
    }
    b
}

fn median256(v: &[f32; 256]) -> f32 {
    let mut s = *v;
    s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    // the reference uses Torben's median; for an even count it returns the
    // lower-middle element, so the same bit count results on either side
    s[127]
}

pub fn bits(band: &[f32; 256]) -> Hash256 {
    let m = median256(band);
    let mut h: Hash256 = [0; 4];
    for (b, v) in band.iter().enumerate() {
        if *v > m {
            h[b / 64] |= 1u64 << (63 - (b % 64));
        }
    }
    h
}

/// The seven other dihedral orientations, from the 16×16 band alone.
/// Row index i is vertical frequency, j horizontal. Reversing an axis of a
/// signal negates the odd-numbered basis rows; with rows numbered from 1
/// here, that is every even i (or j). A rotation is a transpose plus one
/// axis reversal; a 180° turn reverses both.
pub fn band_variant(a: &[f32; 256], v: Variant) -> [f32; 256] {
    let mut b = [0.0f32; 256];
    for i in 0..BAND {
        for j in 0..BAND {
            let x = a[i * BAND + j];
            match v {
                Variant::Identity => b[i * BAND + j] = x,
                // columns reversed (x → n−1−x): negate even j
                Variant::FlipH => b[i * BAND + j] = if j % 2 == 0 { -x } else { x },
                // rows reversed (y → n−1−y): negate even i
                Variant::FlipV => b[i * BAND + j] = if i % 2 == 0 { -x } else { x },
                Variant::Rotate180 => b[i * BAND + j] = if (i + j) % 2 == 1 { -x } else { x },
                Variant::Transpose => b[j * BAND + i] = x,
                Variant::AntiTranspose => b[j * BAND + i] = if (i + j) % 2 == 1 { -x } else { x },
                // rotate 90° as defined in perceptual::variant — verified
                // against the pixel domain in the tests below
                Variant::Rotate90 => b[j * BAND + i] = if i % 2 == 0 { -x } else { x },
                Variant::Rotate270 => b[j * BAND + i] = if j % 2 == 0 { -x } else { x },
            }
        }
    }
    b
}

/// Side the reference tool (`pdq-photo-hasher`) squashes to before
/// hashing: any image wider or taller than this is resampled to
/// `TOOL_SIDE`×`TOOL_SIDE` by nearest neighbour (CImg `resize`,
/// interpolation 1, source index ⌊x·W/512⌋), aspect discarded. The
/// published PDQ hashes — Meta's regression vectors among them — are
/// hashes of that squashed image, so this is the input that makes ours
/// bit-identical to theirs (VERIFICATION.md, Phase 7).
pub const TOOL_SIDE: usize = 512;

/// The reference tool's hashing input: Rec. 601 luma of the image, after
/// the 512 squash when either side exceeds it.
pub fn tool_luma(img: &image::DynamicImage) -> (Vec<f32>, usize, usize) {
    let rgb = img.to_rgb8();
    let (w, h) = (rgb.width() as usize, rgb.height() as usize);
    let raw = rgb.as_raw();
    let luma =
        |i: usize| 0.299 * raw[i] as f32 + 0.587 * raw[i + 1] as f32 + 0.114 * raw[i + 2] as f32;
    if w <= TOOL_SIDE && h <= TOOL_SIDE {
        return ((0..w * h).map(|i| luma(i * 3)).collect(), w, h);
    }
    let mut data = Vec::with_capacity(TOOL_SIDE * TOOL_SIDE);
    for y in 0..TOOL_SIDE {
        let sy = (y as f64 * h as f64 / TOOL_SIDE as f64) as usize;
        for x in 0..TOOL_SIDE {
            let sx = (x as f64 * w as f64 / TOOL_SIDE as f64) as usize;
            data.push(luma((sy * w + sx) * 3));
        }
    }
    (data, TOOL_SIDE, TOOL_SIDE)
}

/// PDQ exactly as the reference tool computes it (`tool_luma`, then the
/// published algorithm with no upscale floor).
pub fn compute_tool(img: &image::DynamicImage) -> Pdq {
    let (l, w, h) = tool_luma(img);
    compute_plain(&Luma {
        data: &l,
        width: w,
        height: h,
    })
}

/// The reference algorithm exactly as published: Jarosz filter, decimate,
/// DCT, median — no upscale floor. `compute` adds the floor measured in
/// Phase 2b for small inputs.
pub fn compute_plain(l: &Luma) -> Pdq {
    if l.width < MIN_DIM || l.height < MIN_DIM {
        return Pdq {
            hashes: [[0; 4]; VARIANTS],
            quality: 0,
        };
    }
    finish(downsample(l))
}

fn finish(buf: Vec<f32>) -> Pdq {
    let q = quality(&buf);
    let band = dct16(&buf);
    let mut hashes = [[0u64; 4]; VARIANTS];
    for (k, v) in VARIANT_ORDER.iter().enumerate() {
        let bv = if *v == Variant::Identity {
            band
        } else {
            band_variant(&band, *v)
        };
        hashes[k] = bits(&bv);
    }
    Pdq { hashes, quality: q }
}

/// Full PDQ from a luminance plane: all eight orientations and the quality.
pub fn compute(l: &Luma) -> Pdq {
    if l.width < MIN_DIM || l.height < MIN_DIM {
        return Pdq {
            hashes: [[0; 4]; VARIANTS],
            quality: 0,
        };
    }
    let short = l.width.min(l.height);
    let buf = if short < MIN_SIDE {
        let f = MIN_SIDE.div_ceil(short) as u32;
        let (w, h) = (l.width as u32 * f, l.height as u32 * f);
        let g = image::GrayImage::from_fn(l.width as u32, l.height as u32, |x, y| {
            image::Luma([l.data[y as usize * l.width + x as usize]
                .round()
                .clamp(0.0, 255.0) as u8])
        });
        let up = image::imageops::resize(&g, w, h, image::imageops::FilterType::Lanczos3);
        let data: Vec<f32> = up.as_raw().iter().map(|&v| v as f32).collect();
        downsample(&Luma {
            data: &data,
            width: w as usize,
            height: h as usize,
        })
    } else {
        downsample(l)
    };
    finish(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synth_luma(seed: u32, w: usize, h: usize) -> Vec<f32> {
        let s = seed as f32;
        let mut v = vec![0.0f32; w * h];
        for y in 0..h {
            for x in 0..w {
                let fx = x as f32 / w as f32;
                let fy = y as f32 / h as f32;
                let dx = fx - (0.3 + 0.4 * ((s * 0.37).sin() * 0.5 + 0.5));
                let dy = fy - (0.3 + 0.4 * ((s * 0.71).cos() * 0.5 + 0.5));
                let disc = if dx * dx + dy * dy < 0.04 { 60.0 } else { 0.0 };
                let stripes = ((fx * (6.0 + (seed % 5) as f32) + fy * 2.0) * std::f32::consts::PI)
                    .sin()
                    * 40.0;
                let block = if fx > 0.6 && fx < 0.8 && fy > 0.1 && fy < 0.3 {
                    90.0
                } else {
                    0.0
                };
                v[y * w + x] = (120.0 * fx + 60.0 * fy + stripes + disc + block).clamp(0.0, 255.0);
            }
        }
        v
    }

    /// Apply a dihedral transform to a square buffer in the pixel domain
    /// (same conventions as `perceptual::variant`).
    fn pixel_variant(src: &[f32], n: usize, v: Variant) -> Vec<f32> {
        let mut out = vec![0.0f32; n * n];
        for y in 0..n {
            for x in 0..n {
                let (sx, sy) = match v {
                    Variant::Identity => (x, y),
                    Variant::Rotate90 => (y, n - 1 - x),
                    Variant::Rotate180 => (n - 1 - x, n - 1 - y),
                    Variant::Rotate270 => (n - 1 - y, x),
                    Variant::FlipH => (n - 1 - x, y),
                    Variant::FlipV => (x, n - 1 - y),
                    Variant::Transpose => (y, x),
                    Variant::AntiTranspose => (n - 1 - y, n - 1 - x),
                };
                out[y * n + x] = src[sy * n + sx];
            }
        }
        out
    }

    #[test]
    fn window_sizes_match_reference_formula() {
        assert_eq!(window_size(64, 64), 1);
        assert_eq!(window_size(128, 64), 1);
        assert_eq!(window_size(129, 64), 2);
        assert_eq!(window_size(1000, 64), 8);
        assert_eq!(window_size(4000, 64), 32);
    }

    #[test]
    fn box_filter_matches_direct_window_average() {
        let input: Vec<f32> = (0..23).map(|i| ((i * 37) % 11) as f32).collect();
        for w in [2usize, 3, 7, 8] {
            let mut out = vec![0.0f32; input.len()];
            box_1d(&input, &mut out, w);
            let h = (w + 2) / 2;
            for (i, got) in out.iter().enumerate() {
                let lo = i.saturating_sub(w - h);
                let hi = (i + h).min(input.len());
                let direct: f32 = input[lo..hi].iter().sum::<f32>() / (hi - lo) as f32;
                assert!((got - direct).abs() < 1e-4, "w={w} i={i} {got} vs {direct}");
            }
        }
        // the reference's worked example: w = 8, output 0 averages indices 0..=4, output 4 averages 1..=8
        let input: Vec<f32> = (0..20).map(|i| i as f32).collect();
        let mut out = vec![0.0f32; 20];
        box_1d(&input, &mut out, 8);
        assert!((out[0] - 2.0).abs() < 1e-6);
        assert!((out[4] - 4.5).abs() < 1e-6);
    }

    #[test]
    fn column_pass_equals_per_column_1d_filter() {
        let (rows, cols) = (37usize, 23usize);
        let input: Vec<f32> = (0..rows * cols)
            .map(|i| ((i * 7919) % 251) as f32)
            .collect();
        for w in [2usize, 3, 5, 8] {
            let mut fast = vec![0.0f32; rows * cols];
            box_cols(&input, &mut fast, rows, cols, w);
            let mut col_in = vec![0.0f32; rows];
            let mut col_out = vec![0.0f32; rows];
            for c in 0..cols {
                for r in 0..rows {
                    col_in[r] = input[r * cols + c];
                }
                box_1d(&col_in, &mut col_out, w);
                for r in 0..rows {
                    assert!(
                        (fast[r * cols + c] - col_out[r]).abs() < 1e-3,
                        "w={w} r={r} c={c}"
                    );
                }
            }
        }
    }

    #[test]
    fn downsample_of_a_smooth_ramp_is_the_ramp() {
        // a horizontal ramp survives filtering and decimation as a ramp
        let (w, h) = (640usize, 480usize);
        let data: Vec<f32> = (0..w * h)
            .map(|i| (i % w) as f32 * 255.0 / (w - 1) as f32)
            .collect();
        let out = downsample(&Luma {
            data: &data,
            width: w,
            height: h,
        });
        for i in 0..SIDE {
            for j in 1..SIDE {
                assert!(out[i * SIDE + j] > out[i * SIDE + j - 1], "row {i} col {j}");
            }
            // every row identical
            assert!((out[i * SIDE] - out[0]).abs() < 1e-3);
        }
    }

    #[test]
    fn dct_domain_variants_equal_pixel_domain_variants() {
        let buf = synth_luma(3, SIDE, SIDE);
        let band = dct16(&buf);
        let base = bits(&band);
        for v in VARIANT_ORDER {
            let from_dct = bits(&band_variant(&band, v));
            let from_pixels = bits(&dct16(&pixel_variant(&buf, SIDE, v)));
            let d = distance(&from_dct, &from_pixels);
            assert!(d <= 2, "{v:?}: dct-domain vs pixel-domain distance {d}");
            if v != Variant::Identity {
                assert!(
                    distance(&base, &from_pixels) > 40,
                    "{v:?} should differ from identity"
                );
            }
        }
    }

    #[test]
    fn quality_scale() {
        let flat = vec![100.0f32; SIDE * SIDE];
        assert_eq!(quality(&flat), 0);
        let textured = synth_luma(1, SIDE, SIDE);
        assert!(
            quality(&textured) >= PDQ_QUALITY_MIN,
            "{}",
            quality(&textured)
        );
        // checkerboard of 0/255 saturates
        let mut cb = vec![0.0f32; SIDE * SIDE];
        for i in 0..SIDE {
            for j in 0..SIDE {
                cb[i * SIDE + j] = if (i + j) % 2 == 0 { 255.0 } else { 0.0 };
            }
        }
        assert_eq!(quality(&cb), 100);
    }

    #[test]
    fn hash_has_128_set_bits_and_is_stable_under_rescale() {
        let full = synth_luma(7, 640, 480);
        let p = compute(&Luma {
            data: &full,
            width: 640,
            height: 480,
        });
        let ones: u32 = p.hashes[0].iter().map(|w| w.count_ones()).sum();
        assert_eq!(ones, 128, "median split must give exactly half the bits");
        // the same picture resampled to half size and upscaled 1.5×; small
        // thumbnails are measured on the corpus (docs/VERIFICATION.md)
        let g = image::GrayImage::from_fn(640, 480, |x, y| {
            image::Luma([full[(y * 640 + x) as usize] as u8])
        });
        for (w, h) in [(320u32, 240u32), (960, 720)] {
            let small = image::imageops::resize(&g, w, h, image::imageops::FilterType::Triangle);
            let data: Vec<f32> = small.as_raw().iter().map(|&v| v as f32).collect();
            let q = compute(&Luma {
                data: &data,
                width: w as usize,
                height: h as usize,
            });
            let d = distance(&p.hashes[0], &q.hashes[0]);
            // hard-edged synthetic content aliases differently at each
            // size; the corpus (docs/VERIFICATION.md) gives the real rates
            assert!(d <= PDQ_MAX / 2, "rescale to {w}x{h}: distance {d}");
        }
        let other = synth_luma(8, 640, 480);
        let o = compute(&Luma {
            data: &other,
            width: 640,
            height: 480,
        });
        let far = distance(&p.hashes[0], &o.hashes[0]);
        assert!(far > 2 * PDQ_MAX, "related-structure picture at {far}");
    }

    #[test]
    fn tiny_input_has_no_hash() {
        let p = compute(&Luma {
            data: &[0.0; 16],
            width: 4,
            height: 4,
        });
        assert_eq!(p.quality, 0);
    }
}
