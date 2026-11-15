//! A palette for one picture and the picture mapped to it, for the sixel
//! encoder (Phase 19) [CLAUDE]. Sixel draws from at most 256 colour
//! registers, so a photograph has to be told in a palette of its own.
//!
//! The palette: the picture's colours in a 32 × 32 × 32 histogram (each
//! bin keeps its pixels' true mean, not its centre); the bins split as
//! median cut does, but always the box with the largest squared error,
//! along its widest channel, at the cut that leaves the least error on
//! both sides (Wu 1991's criterion, on a list of bins rather than his
//! moment tables); then three rounds of k-means over the bins (Lloyd),
//! which move each colour to the mean of the bins nearest it.
//!
//! The mapping: Floyd–Steinberg error diffusion on a serpentine path, each
//! pixel to the nearest colour. A sky is a slow gradient of near colours:
//! a palette that leaves gaps in it shows the dither as a pattern and as
//! contour lines where the pattern changes; NeuQuant's palette (Dekker
//! 1994), sampled for speed, did on the test photographs (VERIFICATION
//! Phase 19).

use image::RgbImage;

#[derive(Clone, Copy)]
struct Bin {
    c: [f32; 3],
    n: f32,
}

/// Up to `max` colours that tell `img`.
pub fn palette(img: &RgbImage, max: usize) -> Vec<[f32; 3]> {
    let max = max.clamp(1, 256);
    let mut hist = vec![(0u32, [0u64; 3]); 1 << 15];
    for p in img.pixels() {
        let [r, g, b] = p.0;
        let k = ((r as usize >> 3) << 10) | ((g as usize >> 3) << 5) | (b as usize >> 3);
        let h = &mut hist[k];
        h.0 += 1;
        h.1[0] += r as u64;
        h.1[1] += g as u64;
        h.1[2] += b as u64;
    }
    let mut bins: Vec<Bin> = hist
        .iter()
        .filter(|h| h.0 > 0)
        .map(|h| {
            let n = h.0 as f32;
            Bin {
                c: [h.1[0] as f32 / n, h.1[1] as f32 / n, h.1[2] as f32 / n],
                n,
            }
        })
        .collect();
    if bins.is_empty() {
        return vec![[0.0; 3]];
    }
    if bins.len() <= max {
        return bins.iter().map(|b| b.c).collect();
    }
    // boxes: ranges of `bins`, each with its error
    let mut boxes: Vec<(usize, usize, f32)> = vec![(0, bins.len(), sse(&bins))];
    while boxes.len() < max {
        let Some((i, _)) = boxes
            .iter()
            .enumerate()
            .filter(|(_, b)| b.1 - b.0 > 1)
            .max_by(|a, b| a.1 .2.total_cmp(&b.1 .2))
        else {
            break;
        };
        let (lo, hi, _) = boxes[i];
        let part = &mut bins[lo..hi];
        let axis = widest(part);
        part.sort_unstable_by(|a, b| a.c[axis].total_cmp(&b.c[axis]));
        let cut = lo + best_cut(part, axis);
        boxes[i] = (lo, cut, sse(&bins[lo..cut]));
        boxes.push((cut, hi, sse(&bins[cut..hi])));
    }
    let mut pal: Vec<[f32; 3]> = boxes
        .iter()
        .map(|&(lo, hi, _)| mean(&bins[lo..hi]))
        .collect();
    // Lloyd: each colour to the mean of the bins nearest it
    for _ in 0..3 {
        let mut acc = vec![([0f64; 3], 0f64); pal.len()];
        for b in &bins {
            let k = nearest(&pal, b.c);
            for (ch, a) in acc[k].0.iter_mut().enumerate() {
                *a += b.c[ch] as f64 * b.n as f64;
            }
            acc[k].1 += b.n as f64;
        }
        for (p, (s, n)) in pal.iter_mut().zip(&acc) {
            if *n > 0.0 {
                *p = [(s[0] / n) as f32, (s[1] / n) as f32, (s[2] / n) as f32];
            }
        }
    }
    pal
}

fn mean(bins: &[Bin]) -> [f32; 3] {
    let mut s = [0f64; 3];
    let mut n = 0f64;
    for b in bins {
        for (a, c) in s.iter_mut().zip(b.c) {
            *a += c as f64 * b.n as f64;
        }
        n += b.n as f64;
    }
    [(s[0] / n) as f32, (s[1] / n) as f32, (s[2] / n) as f32]
}

/// The squared error of the bins about their mean, pixels counted.
fn sse(bins: &[Bin]) -> f32 {
    let m = mean(bins);
    bins.iter()
        .map(|b| {
            let d = [b.c[0] - m[0], b.c[1] - m[1], b.c[2] - m[2]];
            b.n * (d[0] * d[0] + d[1] * d[1] + d[2] * d[2])
        })
        .sum()
}

/// The channel with the largest spread, pixels counted.
fn widest(bins: &[Bin]) -> usize {
    let m = mean(bins);
    (0..3)
        .max_by(|&a, &b| {
            let v = |ch: usize| {
                bins.iter()
                    .map(|x| x.n * (x.c[ch] - m[ch]).powi(2))
                    .sum::<f32>()
            };
            v(a).total_cmp(&v(b))
        })
        .unwrap_or(0)
}

/// Where to cut bins sorted along `axis`: the split, 1 ..= len − 1, that
/// leaves the least squared error along it on the two sides together.
fn best_cut(bins: &[Bin], axis: usize) -> usize {
    let (mut n_all, mut s_all, mut q_all) = (0f64, 0f64, 0f64);
    for b in bins {
        let (n, c) = (b.n as f64, b.c[axis] as f64);
        n_all += n;
        s_all += n * c;
        q_all += n * c * c;
    }
    let (mut n, mut s, mut q) = (0f64, 0f64, 0f64);
    let mut best = (f64::MAX, 1);
    for (i, b) in bins[..bins.len() - 1].iter().enumerate() {
        let (bn, c) = (b.n as f64, b.c[axis] as f64);
        n += bn;
        s += bn * c;
        q += bn * c * c;
        let (n2, s2, q2) = (n_all - n, s_all - s, q_all - q);
        let e = (q - s * s / n) + (q2 - s2 * s2 / n2);
        if e < best.0 {
            best = (e, i + 1);
        }
    }
    best.1
}

#[inline]
fn dist(a: [f32; 3], b: [f32; 3]) -> f32 {
    let d = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    d[0] * d[0] + d[1] * d[1] + d[2] * d[2]
}

fn nearest(pal: &[[f32; 3]], c: [f32; 3]) -> usize {
    let mut best = (f32::MAX, 0);
    for (i, p) in pal.iter().enumerate() {
        let d = dist(*p, c);
        if d < best.0 {
            best = (d, i);
        }
    }
    best.1
}

/// `img` as indices into `pal`, by Floyd–Steinberg error diffusion on a
/// serpentine path. The nearest colour of a value is looked up once per
/// 64 × 64 × 64 cell of the cube and remembered.
pub fn dither(img: &RgbImage, pal: &[[f32; 3]]) -> Vec<u8> {
    let (w, h) = (img.width() as usize, img.height() as usize);
    let mut idx = vec![0u8; w * h];
    let mut cache = vec![u8::MAX; 1 << 18];
    let mut cached = vec![false; 1 << 18];
    // the error carried into this row and the next
    let mut cur = vec![[0f32; 3]; w + 2];
    let mut next = vec![[0f32; 3]; w + 2];
    for y in 0..h {
        let ltr = y % 2 == 0;
        for i in 0..w {
            let x = if ltr { i } else { w - 1 - i };
            let p = img.get_pixel(x as u32, y as u32).0;
            let e = cur[x + 1];
            let want = [
                (p[0] as f32 + e[0]).clamp(0.0, 255.0),
                (p[1] as f32 + e[1]).clamp(0.0, 255.0),
                (p[2] as f32 + e[2]).clamp(0.0, 255.0),
            ];
            let key = ((want[0] as usize >> 2) << 12)
                | ((want[1] as usize >> 2) << 6)
                | (want[2] as usize >> 2);
            let k = if cached[key] {
                cache[key] as usize
            } else {
                // the cell's centre decides for the cell
                let centre = [
                    ((want[0] as usize >> 2) * 4) as f32 + 1.5,
                    ((want[1] as usize >> 2) * 4) as f32 + 1.5,
                    ((want[2] as usize >> 2) * 4) as f32 + 1.5,
                ];
                let k = nearest(pal, centre);
                cache[key] = k as u8;
                cached[key] = true;
                k
            };
            idx[y * w + x] = k as u8;
            let got = pal[k];
            let d = [want[0] - got[0], want[1] - got[1], want[2] - got[2]];
            let (fwd, back): (isize, isize) = if ltr { (1, -1) } else { (-1, 1) };
            let xi = x as isize + 1;
            let add = |row: &mut Vec<[f32; 3]>, at: isize, f: f32| {
                let c = &mut row[at as usize];
                for ch in 0..3 {
                    c[ch] += d[ch] * f;
                }
            };
            add(&mut cur, xi + fwd, 7.0 / 16.0);
            add(&mut next, xi + back, 3.0 / 16.0);
            add(&mut next, xi, 5.0 / 16.0);
            add(&mut next, xi + fwd, 1.0 / 16.0);
        }
        std::mem::swap(&mut cur, &mut next);
        next.iter_mut().for_each(|c| *c = [0.0; 3]);
    }
    idx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn few_colours_are_kept_exactly() {
        let mut img = RgbImage::new(4, 1);
        img.put_pixel(1, 0, image::Rgb([255, 255, 255]));
        img.put_pixel(2, 0, image::Rgb([200, 10, 30]));
        let pal = palette(&img, 255);
        assert_eq!(pal.len(), 3);
        let idx = dither(&img, &pal);
        let back: Vec<[u8; 3]> = idx
            .iter()
            .map(|&k| pal[k as usize].map(|v| v.round() as u8))
            .collect();
        assert_eq!(
            back,
            vec![[0, 0, 0], [255, 255, 255], [200, 10, 30], [0, 0, 0]]
        );
    }

    #[test]
    #[allow(clippy::needless_range_loop)]
    fn a_smooth_gradient_is_told_without_jumps() {
        // a sky: 400 × 200, a slow blue gradient with a little green
        let mut img = RgbImage::new(400, 200);
        for (x, y, p) in img.enumerate_pixels_mut() {
            *p = image::Rgb([
                (10 + x / 20) as u8,
                (80 + y / 8) as u8,
                (150 + x / 12) as u8,
            ]);
        }
        let pal = palette(&img, 255);
        let idx = dither(&img, &pal);
        // no far colour mixed in (NeuQuant's palette mixed colours 28
        // levels apart into the photographs' skies) ...
        let mut worst = 0f32;
        for (i, p) in img.pixels().enumerate() {
            let q = pal[idx[i] as usize];
            for (a, b) in q.iter().zip(p.0) {
                worst = worst.max((a - b as f32).abs());
            }
        }
        assert!(worst <= 12.0, "a pixel {worst} levels off");
        // ... and what the eye averages, 4 × 4 pixels, is the gradient to
        // within 4 levels (inside the edge, where diffusion starts with no
        // error to carry). The worst is 3.5, the gradient's darkest green
        // rows: the palette's colours are means of boxes, so nothing lies
        // beyond them for the error to reach
        // (the dithering cannot leave the palette's hull).
        let mut far = 0f32;
        for by in 1..49 {
            for bx in 1..99 {
                for ch in 0..3 {
                    let mut d = 0f32;
                    for y in by * 4..by * 4 + 4 {
                        for x in bx * 4..bx * 4 + 4 {
                            d += pal[idx[y * 400 + x] as usize][ch]
                                - img.get_pixel(x as u32, y as u32).0[ch] as f32;
                        }
                    }
                    far = far.max((d / 16.0).abs());
                }
            }
        }
        assert!(far <= 4.0, "a 4 × 4 block {far} levels off");
    }
}
