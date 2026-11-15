//! Separable resampling of one plane, bit for bit what the `image` crate's
//! `imageops::resize` gives for a single-channel image (Phase 19, TECH-16)
//! [CLAUDE]. The crate's resize runs every pixel as four channels through
//! a generic pixel interface; on the 288-cell grid and the PDQ working
//! plane that was 10 ms and 5 ms of every file's 50. This is the same
//! arithmetic — the same kernel, the same weights in the same order, the
//! vertical pass first into floats, then the horizontal, the same clamp
//! and the same rounding — on one contiguous plane of `f32`, so the hashes
//! the thresholds were measured on do not move by a bit
//! (`tests::*_is_the_image_crates_bit_for_bit`).

use std::f32::consts::PI;

/// The kernels the hashes use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kernel {
    Triangle,
    Lanczos3,
}

impl Kernel {
    fn support(self) -> f32 {
        match self {
            Kernel::Triangle => 1.0,
            Kernel::Lanczos3 => 3.0,
        }
    }

    #[inline]
    fn at(self, x: f32) -> f32 {
        match self {
            Kernel::Triangle => {
                if x.abs() < 1.0 {
                    1.0 - x.abs()
                } else {
                    0.0
                }
            }
            Kernel::Lanczos3 => {
                if x.abs() < 3.0 {
                    sinc(x) * sinc(x / 3.0)
                } else {
                    0.0
                }
            }
        }
    }
}

#[inline]
fn sinc(t: f32) -> f32 {
    let a = t * PI;
    if t == 0.0 {
        1.0
    } else {
        a.sin() / a
    }
}

/// For each output coordinate: the first input index and the normalised
/// weights, as the crate computes them.
fn weights(input: usize, output: usize, k: Kernel) -> Vec<(usize, Vec<f32>)> {
    let ratio = input as f32 / output as f32;
    let sratio = if ratio < 1.0 { 1.0 } else { ratio };
    let src_support = k.support() * sratio;
    (0..output)
        .map(|o| {
            let centre = (o as f32 + 0.5) * ratio;
            let left = ((centre - src_support).floor() as i64).clamp(0, input as i64 - 1) as usize;
            let right = ((centre + src_support).ceil() as i64).clamp(left as i64 + 1, input as i64)
                as usize;
            let centre = centre - 0.5;
            let mut ws: Vec<f32> = Vec::with_capacity(right - left);
            let mut sum = 0.0;
            for i in left..right {
                let w = k.at((i as f32 - centre) / sratio);
                ws.push(w);
                sum += w;
            }
            for w in ws.iter_mut() {
                *w /= sum;
            }
            (left, ws)
        })
        .collect()
}

/// The two passes, the horizontal one's sums unclamped.
fn passes(src: &[f32], w: usize, h: usize, nw: usize, nh: usize, k: Kernel) -> Vec<f32> {
    // vertical: every column of the input to `nh` rows
    // (row by row into the output row: each value still sums its taps in
    // the crate's order, first to last, so the floats are the same)
    let mut mid = vec![0.0f32; w * nh];
    for (oy, (left, ws)) in weights(h, nh, k).into_iter().enumerate() {
        let row = &mut mid[oy * w..(oy + 1) * w];
        for (i, wt) in ws.iter().enumerate() {
            let line = &src[(left + i) * w..(left + i + 1) * w];
            for (t, v) in row.iter_mut().zip(line) {
                *t += v * wt;
            }
        }
    }
    // horizontal: every row of that to `nw` columns
    let mut out = vec![0.0f32; nw * nh];
    let hw = weights(w, nw, k);
    for y in 0..nh {
        let line = &mid[y * w..(y + 1) * w];
        for (ox, (left, ws)) in hw.iter().enumerate() {
            let mut acc = 0.0f32;
            for (i, wt) in ws.iter().enumerate() {
                acc += line[left + i] * wt;
            }
            out[y * nw + ox] = acc;
        }
    }
    out
}

/// A `u8` plane resized as `imageops::resize` resizes a `GrayImage`:
/// clamped to 0–255 and rounded.
pub fn resize_u8(src: &[u8], w: usize, h: usize, nw: usize, nh: usize, k: Kernel) -> Vec<u8> {
    if w == 0 || h == 0 {
        return vec![0; nw * nh];
    }
    if (w, h) == (nw, nh) {
        return src.to_vec();
    }
    let f: Vec<f32> = src.iter().map(|&v| v as f32).collect();
    passes(&f, w, h, nw, nh, k)
        .into_iter()
        .map(|t| t.clamp(0.0, 255.0).round() as u8)
        .collect()
}

/// An `f32` plane resized as `imageops::resize` resizes a
/// `Luma<f32>` image: clamped to 0–1, not rounded.
pub fn resize_unit(src: &[f32], w: usize, h: usize, nw: usize, nh: usize, k: Kernel) -> Vec<f32> {
    if w == 0 || h == 0 {
        return vec![0.0; nw * nh];
    }
    if (w, h) == (nw, nh) {
        return src.to_vec();
    }
    passes(src, w, h, nw, nh, k)
        .into_iter()
        .map(|t| t.clamp(0.0, 1.0))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::imageops::{self, FilterType};

    fn noise(w: usize, h: usize, seed: u64) -> Vec<u8> {
        let mut s = seed;
        (0..w * h)
            .map(|i| {
                s ^= s << 13;
                s ^= s >> 7;
                s ^= s << 17;
                // a picture's structure under the noise
                ((((i % w) * 255 / w) as u64 + (s % 64)) % 256) as u8
            })
            .collect()
    }

    #[test]
    fn u8_is_the_image_crates_bit_for_bit() {
        for &(w, h, nw, nh) in &[
            (800, 600, 288, 288),
            (600, 800, 288, 288),
            (577, 1151, 288, 288),
            (288, 288, 288, 288),
            (100, 70, 288, 288),
            (1024, 1024, 64, 64),
        ] {
            for (k, f) in [
                (Kernel::Lanczos3, FilterType::Lanczos3),
                (Kernel::Triangle, FilterType::Triangle),
            ] {
                let src = noise(w, h, (w * 31 + h) as u64);
                let g = image::GrayImage::from_raw(w as u32, h as u32, src.clone()).unwrap();
                let theirs = imageops::resize(&g, nw as u32, nh as u32, f).into_raw();
                assert_eq!(
                    resize_u8(&src, w, h, nw, nh, k),
                    theirs,
                    "{w}x{h} → {nw}x{nh} {k:?}"
                );
            }
        }
    }

    #[test]
    fn unit_floats_are_the_image_crates_bit_for_bit() {
        for &(w, h, side) in &[
            (800, 600, 576),
            (600, 800, 576),
            (1151, 577, 576),
            (300, 200, 576),
        ] {
            for (k, f) in [
                (Kernel::Triangle, FilterType::Triangle),
                (Kernel::Lanczos3, FilterType::Lanczos3),
            ] {
                let src: Vec<f32> = noise(w, h, 7).iter().map(|&v| v as f32 / 255.0).collect();
                let p = image::ImageBuffer::<image::Luma<f32>, Vec<f32>>::from_raw(
                    w as u32,
                    h as u32,
                    src.clone(),
                )
                .unwrap();
                let theirs = imageops::resize(&p, side as u32, side as u32, f).into_raw();
                let ours = resize_unit(&src, w, h, side, side, k);
                assert!(
                    ours.iter()
                        .zip(&theirs)
                        .all(|(a, b)| a.to_bits() == b.to_bits()),
                    "{w}x{h} → {side} {k:?}"
                );
            }
        }
    }
}
