//! Perceptual image signatures (docs/RESEARCH.md §2).
//!
//! One decode, one resize to a 288×288 luminance grid, then every hash and
//! every dihedral variant is an exact box average of that grid:
//! 288 = 9·32 = 8·36 = 32·9, so the 9×8 dHash grid and the 32×32 pHash grid
//! are integer block means with no further resampling.
//!
//! dHash: 9×8 grid, bit = right pixel brighter than left (Krawetz 2013).
//! pHash: 32×32 grid → 2-D DCT-II → top-left 8×8 (DC included, as in the
//! `imagehash` reference the published thresholds were measured with) →
//! bit = coefficient above the median.
//!
//! Eight dihedral variants (identity, three rotations, two flips, two
//! transposes) are hashed from the same grid so a mirrored or rotated copy
//! is found without decoding anything twice.

use std::path::Path;
use std::sync::OnceLock;

use image::imageops::FilterType;
use image::{DynamicImage, GrayImage};

use crate::pdq::{self, Hash256, Pdq};

/// Side of the luminance grid every hash is derived from.
pub const GRID: usize = 288;
pub const VARIANTS: usize = 8;

/// dHash Hamming threshold (of 64). Measured on the generated corpus — see
/// docs/VERIFICATION.md §Phase 2 — starting from the normalised 0.159
/// reported in arXiv 2206.00282.
pub const DHASH_MAX: u32 = 10;
/// pHash Hamming threshold (of 64), from the normalised 0.224 in the same
/// benchmark, confirmed on the corpus.
pub const PHASH_MAX: u32 = 14;
/// Minimum feature quality (0–100) for a signature to take part in
/// matching. Below this an image is close to flat and every flat image
/// hashes alike; PDQ gates the same way with its quality score.
pub const QUALITY_MIN: u8 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    Identity,
    Rotate90,
    Rotate180,
    Rotate270,
    FlipH,
    FlipV,
    Transpose,
    AntiTranspose,
}

pub const VARIANT_ORDER: [Variant; VARIANTS] = [
    Variant::Identity,
    Variant::Rotate90,
    Variant::Rotate180,
    Variant::Rotate270,
    Variant::FlipH,
    Variant::FlipV,
    Variant::Transpose,
    Variant::AntiTranspose,
];

impl Variant {
    pub fn label(self) -> &'static str {
        match self {
            Variant::Identity => "",
            Variant::Rotate90 => "rot 90",
            Variant::Rotate180 => "rot 180",
            Variant::Rotate270 => "rot 270",
            Variant::FlipH => "mirror",
            Variant::FlipV => "flip",
            Variant::Transpose => "transpose",
            Variant::AntiTranspose => "transpose",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signature {
    /// dHash per variant, `VARIANT_ORDER` order.
    pub dhash: [u64; VARIANTS],
    /// pHash per variant.
    pub phash: [u64; VARIANTS],
    /// PDQ per variant (256 bits each).
    pub pdq: [Hash256; VARIANTS],
    /// PDQ's own quality (0–100); hashes at or below 49 are unreliable.
    pub pdq_quality: u8,
    pub width: u32,
    pub height: u32,
    /// 0–100 share of grid gradients that carry a visible edge.
    pub quality: u8,
}

impl Signature {
    pub fn pixels(&self) -> u64 {
        self.width as u64 * self.height as u64
    }
}

#[inline]
pub fn hamming(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

// ---------------------------------------------------------------------------
// decode → 288×288 luminance grid
// ---------------------------------------------------------------------------

pub use crate::decode::DecodeError;

/// Decode `path` upright (`decode::load`) and compute both planes: the
/// 288×288 grid (dHash, pHash) and the PDQ result.
pub fn load_planes(path: &Path) -> Result<(Vec<f32>, Pdq, u32, u32), DecodeError> {
    let img = crate::decode::load_image(path)?;
    let (w, h) = (img.width(), img.height());
    let (grid, p) = planes_from_image(&img);
    Ok((grid, p, w, h))
}

/// Decode `path` for the grid only (tests and tools that need no PDQ).
pub fn load_grid(path: &Path) -> Result<(Vec<f32>, u32, u32), DecodeError> {
    let (g, _, w, h) = load_planes(path)?;
    Ok((g, w, h))
}

/// Rec. 601 luminance, box-prefiltered by an integer factor so the short
/// side lands in [2·GRID, 4·GRID). One pass over the pixels.
pub fn prefiltered_luma(img: &DynamicImage) -> (Vec<f32>, usize, usize) {
    let rgb = img.to_rgb8();
    let (w, h) = (rgb.width() as usize, rgb.height() as usize);
    let k = (w.min(h) / (2 * GRID)).max(1);
    let (pw, ph) = (w / k, h / k);
    let raw = rgb.as_raw();
    let norm = 1.0 / (k * k) as f32;
    let mut pre = vec![0.0f32; pw * ph];
    for py in 0..ph {
        for px in 0..pw {
            let mut acc = 0.0f32;
            for y in py * k..py * k + k {
                let row = y * w * 3;
                for x in px * k..px * k + k {
                    let i = row + x * 3;
                    acc += 0.299 * raw[i] as f32
                        + 0.587 * raw[i + 1] as f32
                        + 0.114 * raw[i + 2] as f32;
                }
            }
            pre[py * pw + px] = acc * norm;
        }
    }
    (pre, pw, ph)
}

/// Lanczos3 from the prefiltered plane to GRID×GRID.
pub fn grid_from_luma(pre: &[f32], pw: usize, ph: usize) -> Vec<f32> {
    let mut g = GrayImage::new(pw as u32, ph as u32);
    for (dst, src) in g.as_mut().iter_mut().zip(pre.iter()) {
        *dst = src.round().clamp(0.0, 255.0) as u8;
    }
    let small = image::imageops::resize(&g, GRID as u32, GRID as u32, FilterType::Lanczos3);
    small.as_raw().iter().map(|&v| v as f32).collect()
}

/// Both planes from one decoded image.
pub fn planes_from_image(img: &DynamicImage) -> (Vec<f32>, Pdq) {
    planes_from_image_with(img, PDQ_PREP)
}

/// How the PDQ input is prepared from the decoded image. The hash
/// algorithm itself is the reference's (bit-identical given the same
/// input, `examples/pdq_ref.rs`); the two differ only in what they hand
/// it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PdqPrep {
    /// The box-prefiltered luma the grid uses, with the small-input
    /// upscale floor of Phase 2b. Shipped: on real photographs it keeps
    /// 98.3 % of half-size copies and 100 % of rotations and mirrors
    /// within PDQ_MAX, where the tool's input keeps 91.7 % and 93–95 %
    /// (VERIFICATION.md, Phase 7). Hashes land 2–8 bits from the tool's
    /// on the reference regression images — inside PDQ_MAX, so a hash
    /// published by the reference tool still matches.
    Prefiltered,
    /// The reference tool's input (nearest-neighbour squash to 512×512
    /// when larger): bit-identical to published PDQ hashes. Kept for the
    /// conformance check and the measurement harness; nearest-neighbour
    /// decimation aliases, which is what costs it recall above.
    Tool,
}

/// The preparation this build ships (measured in VERIFICATION.md, Phase 7).
pub const PDQ_PREP: PdqPrep = PdqPrep::Prefiltered;

pub fn planes_from_image_with(img: &DynamicImage, prep: PdqPrep) -> (Vec<f32>, Pdq) {
    let (pre, pw, ph) = prefiltered_luma(img);
    let grid = grid_from_luma(&pre, pw, ph);
    let p = match prep {
        PdqPrep::Prefiltered => pdq::compute(&pdq::Luma {
            data: &pre,
            width: pw,
            height: ph,
        }),
        PdqPrep::Tool => pdq::compute_tool(img),
    };
    (grid, p)
}

/// The signature of a picture decoded reduced: `full` is the full
/// picture's upright size, which the signature records (the reduced JPEG
/// route measured in `tests/decode_reduced.rs`, TECH-02, not shipped).
pub fn signature_full(img: &DynamicImage, full: (u32, u32)) -> Signature {
    let (grid, p) = planes_from_image(img);
    signature_from_planes(&grid, &p, full.0, full.1)
}

/// Signature with a chosen PDQ preparation (the measurement harness).
pub fn signature_from_image_with(img: &DynamicImage, prep: PdqPrep) -> Signature {
    let (grid, p) = planes_from_image_with(img, prep);
    signature_from_planes(&grid, &p, img.width(), img.height())
}

/// The grid alone (tests).
pub fn grid_from_image(img: &DynamicImage) -> Vec<f32> {
    let (pre, pw, ph) = prefiltered_luma(img);
    grid_from_luma(&pre, pw, ph)
}

// ---------------------------------------------------------------------------
// grid operations
// ---------------------------------------------------------------------------

/// Apply a dihedral transform to a GRID×GRID array.
pub fn variant(src: &[f32], v: Variant) -> Vec<f32> {
    let n = GRID;
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

/// Block means of a GRID×GRID array into `cols`×`rows` (both must divide GRID).
pub fn block_grid(src: &[f32], cols: usize, rows: usize) -> Vec<f32> {
    debug_assert!(GRID.is_multiple_of(cols) && GRID.is_multiple_of(rows));
    let bw = GRID / cols;
    let bh = GRID / rows;
    let norm = 1.0 / (bw * bh) as f32;
    let mut out = vec![0.0f32; cols * rows];
    for r in 0..rows {
        for c in 0..cols {
            let mut acc = 0.0f32;
            for y in r * bh..r * bh + bh {
                let row = &src[y * GRID + c * bw..y * GRID + c * bw + bw];
                acc += row.iter().sum::<f32>();
            }
            out[r * cols + c] = acc * norm;
        }
    }
    out
}

/// dHash of a GRID array: 9 columns × 8 rows, bit set when the right block
/// is brighter than the left. Bit (r·8 + c) is stored MSB-first.
pub fn dhash(grid: &[f32]) -> u64 {
    let g = block_grid(grid, 9, 8);
    let mut bits = 0u64;
    for r in 0..8 {
        for c in 0..8 {
            if g[r * 9 + c + 1] > g[r * 9 + c] {
                bits |= 1u64 << (63 - (r * 8 + c));
            }
        }
    }
    bits
}

/// Share (0–100) of the 9×8 horizontal and vertical gradients whose
/// magnitude exceeds 2/255 of full scale — a flat image scores near 0.
pub fn quality(grid: &[f32]) -> u8 {
    let g = block_grid(grid, 9, 8);
    let mut strong = 0u32;
    let mut total = 0u32;
    for r in 0..8 {
        for c in 0..8 {
            total += 1;
            if (g[r * 9 + c + 1] - g[r * 9 + c]).abs() >= 2.0 {
                strong += 1;
            }
        }
    }
    for r in 0..7 {
        for c in 0..9 {
            total += 1;
            if (g[(r + 1) * 9 + c] - g[r * 9 + c]).abs() >= 2.0 {
                strong += 1;
            }
        }
    }
    ((strong * 100) / total) as u8
}

fn dct_matrix() -> &'static [f32] {
    static M: OnceLock<Vec<f32>> = OnceLock::new();
    M.get_or_init(|| dct_matrix_n(32))
}

/// Orthonormal DCT-II matrix: C[u][x] = α(u)·cos(π(2x+1)u / 2N).
pub fn dct_matrix_n(n: usize) -> Vec<f32> {
    let mut m = vec![0.0f32; n * n];
    let nf = n as f64;
    for u in 0..n {
        let alpha = if u == 0 {
            (1.0 / nf).sqrt()
        } else {
            (2.0 / nf).sqrt()
        };
        for x in 0..n {
            let v = alpha
                * (std::f64::consts::PI * (2.0 * x as f64 + 1.0) * u as f64 / (2.0 * nf)).cos();
            m[u * n + x] = v as f32;
        }
    }
    m
}

/// 2-D DCT-II as D = C·G·Cᵀ for an n×n block, row-major.
pub fn dct2d(g: &[f32], c: &[f32], n: usize) -> Vec<f32> {
    let mut tmp = vec![0.0f32; n * n]; // C·G
    for u in 0..n {
        for x in 0..n {
            let mut acc = 0.0f32;
            for k in 0..n {
                acc += c[u * n + k] * g[k * n + x];
            }
            tmp[u * n + x] = acc;
        }
    }
    let mut out = vec![0.0f32; n * n]; // tmp·Cᵀ
    for u in 0..n {
        for v in 0..n {
            let mut acc = 0.0f32;
            for k in 0..n {
                acc += tmp[u * n + k] * c[v * n + k];
            }
            out[u * n + v] = acc;
        }
    }
    out
}

/// pHash of a GRID array: 32×32 block means → DCT → 8×8 low band → median.
pub fn phash(grid: &[f32]) -> u64 {
    let g = block_grid(grid, 32, 32);
    let d = dct2d(&g, dct_matrix(), 32);
    let mut low = [0.0f32; 64];
    for u in 0..8 {
        for v in 0..8 {
            low[u * 8 + v] = d[u * 32 + v];
        }
    }
    let mut sorted = low;
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median = 0.5 * (sorted[31] + sorted[32]);
    let mut bits = 0u64;
    for (i, v) in low.iter().enumerate() {
        if *v > median {
            bits |= 1u64 << (63 - i);
        }
    }
    bits
}

/// Full signature from both planes and the source dimensions.
pub fn signature_from_planes(grid: &[f32], p: &Pdq, width: u32, height: u32) -> Signature {
    let mut dh = [0u64; VARIANTS];
    let mut ph = [0u64; VARIANTS];
    for (i, v) in VARIANT_ORDER.iter().enumerate() {
        let g = if *v == Variant::Identity {
            grid.to_vec()
        } else {
            variant(grid, *v)
        };
        dh[i] = dhash(&g);
        ph[i] = phash(&g);
    }
    Signature {
        dhash: dh,
        phash: ph,
        pdq: p.hashes,
        pdq_quality: p.quality,
        width,
        height,
        quality: quality(grid),
    }
}

/// Signature with an empty PDQ (grid-only paths and tests).
pub fn signature_from_grid(grid: &[f32], width: u32, height: u32) -> Signature {
    let p = Pdq {
        hashes: [[0; 4]; VARIANTS],
        quality: 0,
    };
    signature_from_planes(grid, &p, width, height)
}

/// Signature from a decoded image: one decode, both planes.
pub fn signature_from_image(img: &DynamicImage) -> Signature {
    let (grid, p) = planes_from_image(img);
    signature_from_planes(&grid, &p, img.width(), img.height())
}

pub fn signature(path: &Path) -> Result<Signature, DecodeError> {
    let (grid, p, w, h) = load_planes(path)?;
    Ok(signature_from_planes(&grid, &p, w, h))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use image::{ImageBuffer, ImageEncoder, Rgb, RgbImage};

    /// A deterministic textured picture: gradient + disc + stripes.
    pub(crate) fn synth(seed: u32, w: u32, h: u32) -> RgbImage {
        let s = seed as f32;
        ImageBuffer::from_fn(w, h, |x, y| {
            let fx = x as f32 / w as f32;
            let fy = y as f32 / h as f32;
            let dx = fx - (0.3 + 0.4 * ((s * 0.37).sin() * 0.5 + 0.5));
            let dy = fy - (0.3 + 0.4 * ((s * 0.71).cos() * 0.5 + 0.5));
            let disc = if dx * dx + dy * dy < 0.04 { 1.0 } else { 0.0 };
            let stripes =
                ((fx * (6.0 + (seed % 5) as f32) + fy * 2.0) * std::f32::consts::PI).sin() * 0.5
                    + 0.5;
            let r = (255.0 * (0.6 * fx + 0.4 * disc)).clamp(0.0, 255.0) as u8;
            let g = (255.0 * (0.5 * fy + 0.5 * stripes * (1.0 - disc))).clamp(0.0, 255.0) as u8;
            let b = (255.0 * (0.3 + 0.7 * ((s * 0.11).sin() * 0.5 + 0.5) * (1.0 - fy)))
                .clamp(0.0, 255.0) as u8;
            Rgb([r, g, b])
        })
    }

    #[test]
    fn dct_matches_naive_definition() {
        let n = 8;
        let c = dct_matrix_n(n);
        let g: Vec<f32> = (0..n * n).map(|i| ((i * 7919) % 97) as f32).collect();
        let fast = dct2d(&g, &c, n);
        for u in 0..n {
            for v in 0..n {
                let au = if u == 0 {
                    (1.0 / n as f64).sqrt()
                } else {
                    (2.0 / n as f64).sqrt()
                };
                let av = if v == 0 {
                    (1.0 / n as f64).sqrt()
                } else {
                    (2.0 / n as f64).sqrt()
                };
                let mut acc = 0.0f64;
                for x in 0..n {
                    for y in 0..n {
                        let cx = (std::f64::consts::PI * (2.0 * x as f64 + 1.0) * u as f64
                            / (2.0 * n as f64))
                            .cos();
                        let cy = (std::f64::consts::PI * (2.0 * y as f64 + 1.0) * v as f64
                            / (2.0 * n as f64))
                            .cos();
                        acc += g[x * n + y] as f64 * cx * cy;
                    }
                }
                let naive = (au * av * acc) as f32;
                assert!(
                    (fast[u * n + v] - naive).abs() < 1e-2,
                    "({u},{v}) {} vs {}",
                    fast[u * n + v],
                    naive
                );
            }
        }
    }

    #[test]
    fn dihedral_group_relations() {
        let src: Vec<f32> = (0..GRID * GRID).map(|i| (i % 251) as f32).collect();
        assert_eq!(variant(&variant(&src, Variant::FlipH), Variant::FlipH), src);
        assert_eq!(
            variant(&variant(&src, Variant::Transpose), Variant::Transpose),
            src
        );
        let r = variant(
            &variant(
                &variant(&variant(&src, Variant::Rotate90), Variant::Rotate90),
                Variant::Rotate90,
            ),
            Variant::Rotate90,
        );
        assert_eq!(r, src);
        assert_eq!(
            variant(&variant(&src, Variant::Rotate90), Variant::Rotate90),
            variant(&src, Variant::Rotate180)
        );
        assert_eq!(
            variant(&src, Variant::Rotate270),
            variant(&variant(&src, Variant::Rotate180), Variant::Rotate90)
        );
    }

    #[test]
    fn mirrored_file_matches_mirror_variant() {
        let img = synth(3, 640, 480);
        let flipped = image::imageops::flip_horizontal(&img);
        let a = signature_from_grid(&grid_from_image(&DynamicImage::ImageRgb8(img)), 640, 480);
        let b = signature_from_grid(
            &grid_from_image(&DynamicImage::ImageRgb8(flipped)),
            640,
            480,
        );
        // identity of the flipped file ≈ FlipH variant of the original
        assert!(
            hamming(a.dhash[4], b.dhash[0]) <= 2,
            "d {}",
            hamming(a.dhash[4], b.dhash[0])
        );
        assert!(
            hamming(a.phash[4], b.phash[0]) <= 2,
            "p {}",
            hamming(a.phash[4], b.phash[0])
        );
    }

    #[test]
    fn rotated_file_matches_rotation_variant() {
        let img = synth(5, 600, 600);
        let rot = image::imageops::rotate90(&img);
        let a = signature_from_grid(&grid_from_image(&DynamicImage::ImageRgb8(img)), 600, 600);
        let b = signature_from_grid(&grid_from_image(&DynamicImage::ImageRgb8(rot)), 600, 600);
        assert!(
            hamming(a.dhash[1], b.dhash[0]) <= 2,
            "d {}",
            hamming(a.dhash[1], b.dhash[0])
        );
        assert!(
            hamming(a.phash[1], b.phash[0]) <= 2,
            "p {}",
            hamming(a.phash[1], b.phash[0])
        );
    }

    #[test]
    fn rescale_and_jpeg_stay_close_while_other_images_stay_far() {
        let img = synth(11, 800, 600);
        let a = signature_from_grid(
            &grid_from_image(&DynamicImage::ImageRgb8(img.clone())),
            800,
            600,
        );
        let small = image::imageops::resize(&img, 200, 150, FilterType::Triangle);
        let b = signature_from_grid(&grid_from_image(&DynamicImage::ImageRgb8(small)), 200, 150);
        assert!(hamming(a.dhash[0], b.dhash[0]) <= DHASH_MAX);
        assert!(hamming(a.phash[0], b.phash[0]) <= PHASH_MAX);

        let mut buf = std::io::Cursor::new(Vec::new());
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 30)
            .encode_image(&img)
            .unwrap();
        let decoded = image::load_from_memory(buf.get_ref()).unwrap();
        let c = signature_from_grid(&grid_from_image(&decoded), 800, 600);
        assert!(
            hamming(a.dhash[0], c.dhash[0]) <= DHASH_MAX,
            "{}",
            hamming(a.dhash[0], c.dhash[0])
        );
        assert!(
            hamming(a.phash[0], c.phash[0]) <= PHASH_MAX,
            "{}",
            hamming(a.phash[0], c.phash[0])
        );

        let other = synth(12, 800, 600);
        let d = signature_from_grid(&grid_from_image(&DynamicImage::ImageRgb8(other)), 800, 600);
        let dd = hamming(a.dhash[0], d.dhash[0]);
        let dp = hamming(a.phash[0], d.phash[0]);
        assert!(dd > DHASH_MAX || dp > PHASH_MAX, "d {dd} p {dp}");
    }

    #[test]
    fn flat_image_has_no_quality() {
        let flat: RgbImage = ImageBuffer::from_pixel(100, 100, Rgb([120, 120, 120]));
        let s = signature_from_grid(&grid_from_image(&DynamicImage::ImageRgb8(flat)), 100, 100);
        assert!(s.quality < QUALITY_MIN, "{}", s.quality);
        let textured = synth(1, 100, 100);
        let s = signature_from_grid(
            &grid_from_image(&DynamicImage::ImageRgb8(textured)),
            100,
            100,
        );
        assert!(s.quality >= QUALITY_MIN, "{}", s.quality);
    }

    #[test]
    fn exif_orientation_is_applied() {
        // a JPEG with orientation 6 (rotate 90 CW) must hash like the upright picture
        let dir = tempfile::tempdir().unwrap();
        let upright = synth(7, 400, 300);
        let stored = image::imageops::rotate270(&upright); // stored sideways, EXIF says rotate 90 CW
        let p = dir.path().join("o6.jpg");
        let mut bytes = std::io::Cursor::new(Vec::new());
        let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 92);
        enc.set_exif_metadata(exif_orientation(6)).unwrap();
        enc.encode_image(&DynamicImage::ImageRgb8(stored)).unwrap();
        std::fs::write(&p, bytes.get_ref()).unwrap();
        let s = signature(&p).unwrap();
        let ref_sig = signature_from_grid(
            &grid_from_image(&DynamicImage::ImageRgb8(upright)),
            400,
            300,
        );
        assert_eq!((s.width, s.height), (400, 300));
        assert!(
            hamming(s.dhash[0], ref_sig.dhash[0]) <= 2,
            "{}",
            hamming(s.dhash[0], ref_sig.dhash[0])
        );
    }

    /// Minimal little-endian TIFF-structured EXIF blob with one IFD entry: Orientation (0x0112).
    fn exif_orientation(o: u16) -> Vec<u8> {
        let mut v = vec![b'I', b'I', 0x2A, 0x00, 8, 0, 0, 0]; // header, IFD0 at offset 8
        v.extend_from_slice(&1u16.to_le_bytes()); // one entry
        v.extend_from_slice(&0x0112u16.to_le_bytes()); // tag
        v.extend_from_slice(&3u16.to_le_bytes()); // SHORT
        v.extend_from_slice(&1u32.to_le_bytes()); // count
        v.extend_from_slice(&o.to_le_bytes());
        v.extend_from_slice(&0u16.to_le_bytes()); // pad value to 4 bytes
        v.extend_from_slice(&0u32.to_le_bytes()); // next IFD
        v
    }
}
