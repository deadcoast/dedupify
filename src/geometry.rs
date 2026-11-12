//! The geometric fit (docs/RESEARCH.md §2.7): where the hashes stop,
//! correlation continues.
//!
//! Two pictures are compared as small aspect-preserving luma thumbnails.
//! One is searched for inside the other — over a range of scales
//! (crops), every offset (where the crop sits), and small rotations — and
//! the best normalised cross-correlation (NCC) is the fit. NCC is
//! invariant to brightness and contrast, so a crop that was also
//! brightened or re-encoded still fits; it is a spatial-domain measure,
//! independent of the frequency-domain hashes, which is why it counts as
//! a second family of evidence.
//!
//! The thumbnails are band-passed before correlating (a narrow blur minus
//! a wide one): the wide blur removes slow gradients, which otherwise let
//! a smooth sky correlate with a smooth ramp; the narrow blur makes thin
//! strokes tolerant of the half-pixel shifts a rescale introduces. What
//! correlates is edges and texture, which is what makes two pictures the
//! same picture.
//!
//! The search is coarse-to-fine: every scale and offset at 64 pixels
//! (a few million multiply-adds per direction), then the best position
//! refined at 128. Both directions are tried (B inside A, A inside B),
//! so a letterbox — the original inside a larger frame — is found too; a
//! stretch of one onto the other's whole frame covers aspect changes.

use image::DynamicImage;

/// Longest side of the thumbnails the fit works on.
pub const MAX_SIDE: usize = 128;
/// Smallest window (as a fraction of the containing side) searched: a
/// crop keeping less than half a side (a quarter of the area) is not
/// claimed.
pub const SCALE_MIN: f32 = 0.5;
/// Small rotations searched at full scale, in degrees.
pub const ANGLES: [f32; 7] = [-6.0, -4.0, -2.0, 0.0, 2.0, 4.0, 6.0];
/// Band-pass radii (box blurs) applied to every thumbnail at `MAX_SIDE`;
/// scaled down with the thumbnail for the coarse pass.
pub const BLUR_NARROW: usize = 1;
pub const BLUR_WIDE: usize = 8;
/// Fit at or above this confirms a pair (measured, docs/VERIFICATION.md
/// Phase 6; held in Phase 11 for the search with cells and mirror).
pub const FIT_MIN: f32 = 0.92;
/// The robust score's grid: the window at the best placement is cut into
/// `GRID` × `GRID` cells, each correlated on its own (CORE-01, Phase 11).
pub const GRID: usize = 4;
/// Share of the textured cells set aside, worst first, before the
/// robust score averages the rest: a caption bar, a sticker, a local
/// retouch covers a few cells of a true copy. [CLAUDE, Phase 11]
pub const TRIM: f32 = 0.25;

/// What the search tries beyond the plain correlation (CORE-01, CORE-02).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Search {
    /// Also score the best placement by its cells, worst quarter set
    /// aside.
    pub robust: bool,
    /// Also search the mirror image of the second picture.
    pub mirror: bool,
}

impl Search {
    pub const PLAIN: Search = Search {
        robust: false,
        mirror: false,
    };
}

/// An aspect-preserving luma thumbnail, values in [0, 255].
#[derive(Debug, Clone)]
pub struct Thumb {
    pub w: usize,
    pub h: usize,
    pub px: Vec<f32>,
}

impl Thumb {
    /// Decode to luma and shrink so the longer side is `max_side`.
    pub fn from_image(img: &DynamicImage, max_side: usize) -> Thumb {
        let (w0, h0) = (img.width() as usize, img.height() as usize);
        let scale = max_side as f32 / w0.max(h0).max(1) as f32;
        let w = ((w0 as f32 * scale).round() as usize).clamp(1, max_side);
        let h = ((h0 as f32 * scale).round() as usize).clamp(1, max_side);
        let small = img.resize_exact(w as u32, h as u32, image::imageops::FilterType::Triangle);
        let gray = small.to_luma8();
        Thumb {
            w,
            h,
            px: gray.as_raw().iter().map(|&v| v as f32).collect(),
        }
    }

    /// Box blur with radius `r` (window 2r+1), edges clamped; separable.
    fn box_blur(&self, r: usize) -> Thumb {
        if r == 0 {
            return self.clone();
        }
        let mut tmp = vec![0.0f32; self.w * self.h];
        for y in 0..self.h {
            for x in 0..self.w {
                let lo = x.saturating_sub(r);
                let hi = (x + r).min(self.w - 1);
                let mut acc = 0.0;
                for xx in lo..=hi {
                    acc += self.at(xx, y);
                }
                tmp[y * self.w + x] = acc / (hi - lo + 1) as f32;
            }
        }
        let mut out = vec![0.0f32; self.w * self.h];
        for x in 0..self.w {
            for y in 0..self.h {
                let lo = y.saturating_sub(r);
                let hi = (y + r).min(self.h - 1);
                let mut acc = 0.0;
                for yy in lo..=hi {
                    acc += tmp[yy * self.w + x];
                }
                out[y * self.w + x] = acc / (hi - lo + 1) as f32;
            }
        }
        Thumb {
            w: self.w,
            h: self.h,
            px: out,
        }
    }

    /// Narrow blur minus wide blur: the band the fit correlates on.
    fn band_pass(&self, narrow: usize, wide: usize) -> Thumb {
        let a = self.box_blur(narrow);
        let b = self.box_blur(wide);
        Thumb {
            w: self.w,
            h: self.h,
            px: a.px.iter().zip(&b.px).map(|(x, y)| x - y).collect(),
        }
    }

    /// Band radii for a containing picture of this size: the shipped radii
    /// are for `MAX_SIDE`, halved for the coarse pass.
    fn radii(&self) -> (usize, usize) {
        let f = self.w.max(self.h) as f32 / MAX_SIDE as f32;
        let narrow = (BLUR_NARROW as f32 * f).round() as usize;
        let wide = ((BLUR_WIDE as f32 * f).round() as usize).max(2);
        (narrow, wide)
    }

    fn at(&self, x: usize, y: usize) -> f32 {
        self.px[y * self.w + x]
    }

    /// Bilinear resize.
    pub fn resized(&self, w: usize, h: usize) -> Thumb {
        let w = w.max(1);
        let h = h.max(1);
        let mut px = Vec::with_capacity(w * h);
        let sx = self.w as f32 / w as f32;
        let sy = self.h as f32 / h as f32;
        for y in 0..h {
            let fy = ((y as f32 + 0.5) * sy - 0.5).clamp(0.0, (self.h - 1) as f32);
            let y0 = fy.floor() as usize;
            let y1 = (y0 + 1).min(self.h - 1);
            let ty = fy - y0 as f32;
            for x in 0..w {
                let fx = ((x as f32 + 0.5) * sx - 0.5).clamp(0.0, (self.w - 1) as f32);
                let x0 = fx.floor() as usize;
                let x1 = (x0 + 1).min(self.w - 1);
                let tx = fx - x0 as f32;
                let v = self.at(x0, y0) * (1.0 - tx) * (1.0 - ty)
                    + self.at(x1, y0) * tx * (1.0 - ty)
                    + self.at(x0, y1) * (1.0 - tx) * ty
                    + self.at(x1, y1) * tx * ty;
                px.push(v);
            }
        }
        Thumb { w, h, px }
    }

    /// Left and right exchanged.
    pub fn mirrored(&self) -> Thumb {
        let mut px = Vec::with_capacity(self.px.len());
        for y in 0..self.h {
            px.extend(self.px[y * self.w..(y + 1) * self.w].iter().rev());
        }
        Thumb {
            w: self.w,
            h: self.h,
            px,
        }
    }

    /// Half size (2×2 box), for the coarse pass.
    fn half(&self) -> Thumb {
        self.resized((self.w / 2).max(1), (self.h / 2).max(1))
    }

    /// Rotate about the centre by `deg`; pixels that leave the frame are
    /// `None` (NaN) and excluded from the correlation.
    fn rotated(&self, deg: f32) -> Thumb {
        let (cx, cy) = (self.w as f32 / 2.0, self.h as f32 / 2.0);
        let (s, c) = deg.to_radians().sin_cos();
        let mut px = Vec::with_capacity(self.w * self.h);
        for y in 0..self.h {
            for x in 0..self.w {
                let dx = x as f32 + 0.5 - cx;
                let dy = y as f32 + 0.5 - cy;
                let sx = cx + dx * c + dy * s - 0.5;
                let sy = cy - dx * s + dy * c - 0.5;
                if sx < 0.0 || sy < 0.0 || sx > (self.w - 1) as f32 || sy > (self.h - 1) as f32 {
                    px.push(f32::NAN);
                    continue;
                }
                let x0 = sx.floor() as usize;
                let y0 = sy.floor() as usize;
                let x1 = (x0 + 1).min(self.w - 1);
                let y1 = (y0 + 1).min(self.h - 1);
                let tx = sx - x0 as f32;
                let ty = sy - y0 as f32;
                px.push(
                    self.at(x0, y0) * (1.0 - tx) * (1.0 - ty)
                        + self.at(x1, y0) * tx * (1.0 - ty)
                        + self.at(x0, y1) * (1.0 - tx) * ty
                        + self.at(x1, y1) * tx * ty,
                );
            }
        }
        Thumb {
            w: self.w,
            h: self.h,
            px,
        }
    }
}

/// Where and how well one picture fits inside the other.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fit {
    /// Normalised cross-correlation in [-1, 1]; 1 is a perfect affine match.
    pub ncc: f32,
    /// Window width as a fraction of the containing picture's width.
    pub scale: f32,
    /// Window offset in thumbnail pixels (at `MAX_SIDE`).
    pub dx: i32,
    pub dy: i32,
    /// Rotation applied to the inner picture, degrees.
    pub angle: f32,
    /// True when the second argument was the inner picture.
    pub inner_is_second: bool,
    /// The cells' score at the same placement (`TRIM` of them set aside);
    /// -1 when not searched or too few cells carry texture.
    pub robust: f32,
    /// The second picture was searched as its mirror image.
    pub mirrored: bool,
}

impl Fit {
    pub fn none() -> Fit {
        Fit {
            ncc: -1.0,
            scale: 0.0,
            dx: 0,
            dy: 0,
            angle: 0.0,
            inner_is_second: true,
            robust: -1.0,
            mirrored: false,
        }
    }

    /// The score a pair is judged by: the plain correlation, or the cells'
    /// when that is higher.
    pub fn score(&self) -> f32 {
        self.ncc.max(self.robust)
    }

    pub fn confirmed(&self) -> bool {
        self.score() >= FIT_MIN
    }

    /// `fit 0.97  crop 80%`, `fit 0.95  rot +4°`, `fit 0.99` — the words a
    /// result line carries. Below `FIT_MIN` only the number is shown: a
    /// placement that did not fit is not a crop.
    pub fn label(&self) -> String {
        let mut s = format!("fit {:.2}", self.score().max(0.0));
        if !self.confirmed() {
            return s;
        }
        if self.mirrored {
            s.push_str("  mirror");
        }
        if self.angle != 0.0 {
            s.push_str(&format!("  rot {:+.0}\u{b0}", self.angle));
        } else if self.scale < 0.995 {
            s.push_str(&format!("  crop {:.0}%", self.scale * 100.0));
        }
        s
    }
}

/// NCC between `inner` placed at (dx, dy) inside `outer`, over the window
/// minus a border of `margin` pixels (where the inner picture's blur saw
/// its own edge and the outer's saw more picture). NaN pixels in `inner`
/// are skipped. Returns -1 for a degenerate (flat) window.
fn ncc_at(outer: &Thumb, inner: &Thumb, dx: usize, dy: usize, margin: usize) -> f32 {
    let margin = if inner.w > 2 * margin + 4 && inner.h > 2 * margin + 4 {
        margin
    } else {
        0
    };
    let mut n = 0.0f64;
    let (mut sa, mut sb, mut saa, mut sbb, mut sab) = (0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64);
    for y in margin..inner.h - margin {
        let orow = (dy + y) * outer.w + dx;
        let irow = y * inner.w;
        for x in margin..inner.w - margin {
            let b = inner.px[irow + x];
            if b.is_nan() {
                continue;
            }
            let a = outer.px[orow + x];
            let (a, b) = (a as f64, b as f64);
            n += 1.0;
            sa += a;
            sb += b;
            saa += a * a;
            sbb += b * b;
            sab += a * b;
        }
    }
    if n < 16.0 {
        return -1.0;
    }
    let va = saa - sa * sa / n;
    let vb = sbb - sb * sb / n;
    if va <= 1e-6 * n || vb <= 1e-6 * n {
        return -1.0;
    }
    ((sab - sa * sb / n) / (va * vb).sqrt()) as f32
}

/// The outer picture's sums for every rectangle in O(1): summed-area
/// tables of its values and their squares (Phase 14, CORE-15).
struct Integral {
    w: usize,
    s: Vec<f64>,
    ss: Vec<f64>,
}

impl Integral {
    fn of(t: &Thumb) -> Integral {
        let w = t.w + 1;
        let mut s = vec![0.0f64; w * (t.h + 1)];
        let mut ss = vec![0.0f64; w * (t.h + 1)];
        for y in 0..t.h {
            let (mut row, mut row2) = (0.0f64, 0.0f64);
            for x in 0..t.w {
                let v = t.px[y * t.w + x] as f64;
                row += v;
                row2 += v * v;
                s[(y + 1) * w + x + 1] = s[y * w + x + 1] + row;
                ss[(y + 1) * w + x + 1] = ss[y * w + x + 1] + row2;
            }
        }
        Integral { w, s, ss }
    }

    /// Sum and sum of squares over x in [x0, x1), y in [y0, y1).
    fn rect(&self, x0: usize, y0: usize, x1: usize, y1: usize) -> (f64, f64) {
        let at = |v: &[f64], x: usize, y: usize| v[y * self.w + x];
        let f = |v: &[f64]| at(v, x1, y1) - at(v, x0, y1) - at(v, x1, y0) + at(v, x0, y0);
        (f(&self.s), f(&self.ss))
    }
}

/// A containment window prepared once for every placement: its margin,
/// and its own sums over what is correlated.
struct Window<'a> {
    t: &'a Thumb,
    margin: usize,
    n: f64,
    sb: f64,
    vb: f64,
}

impl<'a> Window<'a> {
    /// `None` for a window with NaN pixels (a rotation's corners), which
    /// `ncc_at` handles.
    fn of(t: &'a Thumb, margin: usize) -> Option<Window<'a>> {
        let margin = if t.w > 2 * margin + 4 && t.h > 2 * margin + 4 {
            margin
        } else {
            0
        };
        let (mut sb, mut sbb, mut n) = (0.0f64, 0.0f64, 0.0f64);
        for y in margin..t.h - margin {
            for &b in &t.px[y * t.w + margin..y * t.w + t.w - margin] {
                if b.is_nan() {
                    return None;
                }
                let b = b as f64;
                sb += b;
                sbb += b * b;
                n += 1.0;
            }
        }
        Some(Window {
            t,
            margin,
            n,
            sb,
            vb: sbb - sb * sb / n,
        })
    }

    /// The same correlation as `ncc_at` at (dx, dy) inside `outer`: the
    /// outer sums from `integral`, the cross term the only per-pixel work.
    fn ncc(&self, outer: &Thumb, integral: &Integral, dx: usize, dy: usize) -> f32 {
        let (t, m, n) = (self.t, self.margin, self.n);
        if n < 16.0 {
            return -1.0;
        }
        let (sa, saa) = integral.rect(dx + m, dy + m, dx + t.w - m, dy + t.h - m);
        let va = saa - sa * sa / n;
        if va <= 1e-6 * n || self.vb <= 1e-6 * n {
            return -1.0;
        }
        let mut sab = 0.0f64;
        for y in m..t.h - m {
            let o = &outer.px[(dy + y) * outer.w + dx + m..(dy + y) * outer.w + dx + t.w - m];
            let i = &t.px[y * t.w + m..y * t.w + t.w - m];
            // a row in single precision (64 products at most), vectorised
            let row: f32 = o.iter().zip(i).map(|(a, b)| a * b).sum();
            sab += row as f64;
        }
        ((sab - sa * self.sb / n) / (va * self.vb).sqrt()) as f32
    }
}

/// The robust score of `inner` at (dx, dy) in `outer`: the window (less
/// `margin`) cut into `GRID` × `GRID` cells, each correlated on its own;
/// cells with almost no texture (variance under a twentieth of the
/// window's mean per cell, in either picture) carry no evidence and are
/// left out; of the rest the worst `TRIM` are set aside and the others
/// averaged. -1 when fewer than half the cells carry texture.
fn ncc_cells(outer: &Thumb, inner: &Thumb, dx: usize, dy: usize, margin: usize) -> f32 {
    let margin = if inner.w > 2 * margin + 4 && inner.h > 2 * margin + 4 {
        margin
    } else {
        0
    };
    let (w, h) = (inner.w - 2 * margin, inner.h - 2 * margin);
    if w < GRID * 4 || h < GRID * 4 {
        return -1.0;
    }
    // per cell: n, sa, sb, saa, sbb, sab
    let mut acc = vec![[0.0f64; 6]; GRID * GRID];
    for y in 0..h {
        let cy = y * GRID / h;
        let orow = (dy + margin + y) * outer.w + dx + margin;
        let irow = (margin + y) * inner.w + margin;
        for x in 0..w {
            let b = inner.px[irow + x];
            if b.is_nan() {
                continue;
            }
            let a = outer.px[orow + x] as f64;
            let b = b as f64;
            let c = &mut acc[cy * GRID + x * GRID / w];
            c[0] += 1.0;
            c[1] += a;
            c[2] += b;
            c[3] += a * a;
            c[4] += b * b;
            c[5] += a * b;
        }
    }
    let var = |c: &[f64; 6]| {
        let n = c[0].max(1.0);
        ((c[3] - c[1] * c[1] / n) / n, (c[4] - c[2] * c[2] / n) / n)
    };
    let (mut ma, mut mb) = (0.0, 0.0);
    for c in &acc {
        let (va, vb) = var(c);
        ma += va;
        mb += vb;
    }
    let (ma, mb) = (ma / acc.len() as f64, mb / acc.len() as f64);
    let mut cells: Vec<f32> = acc
        .iter()
        .filter(|c| c[0] >= 16.0)
        .filter_map(|c| {
            let (va, vb) = var(c);
            if va < ma / 20.0 || vb < mb / 20.0 || va <= 1e-9 || vb <= 1e-9 {
                return None;
            }
            let n = c[0];
            Some(((c[5] - c[1] * c[2] / n) / n / (va * vb).sqrt()) as f32)
        })
        .collect();
    if cells.len() * 2 < GRID * GRID {
        return -1.0;
    }
    cells.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
    let keep = ((cells.len() as f32 * (1.0 - TRIM)).ceil() as usize).max(1);
    cells[..keep].iter().sum::<f32>() / keep as f32
}

/// Scales searched: from 1.0 down to `SCALE_MIN` in 5 % steps.
fn scales() -> Vec<f32> {
    let mut v = Vec::new();
    let mut s = 1.0f32;
    while s >= SCALE_MIN - 1e-6 {
        v.push(s);
        s -= 0.05;
    }
    v
}

/// Best placement of `inner` inside `outer` over scales and offsets,
/// coarse at half size then refined at full size around the best.
fn best_containment(outer: &Thumb, inner: &Thumb, robust: bool) -> Fit {
    let (o2, i2) = (outer.half(), inner.half());
    let (n2, w2r) = o2.radii();
    let o2b = o2.band_pass(n2, w2r);
    let integral = Integral::of(&o2b);
    let mut best = Fit::none();
    let aspect = inner.h as f32 / inner.w as f32;
    for s in scales() {
        // window at coarse size
        let w2 = ((o2.w as f32 * s).round() as usize).max(4);
        let h2 = ((w2 as f32 * aspect).round() as usize).max(4);
        if w2 > o2.w || h2 > o2.h {
            continue;
        }
        let win = i2.resized(w2, h2).band_pass(n2, w2r);
        let prepared = Window::of(&win, w2r);
        for dy in 0..=(o2.h - h2) {
            for dx in 0..=(o2.w - w2) {
                let c = match &prepared {
                    Some(p) => p.ncc(&o2b, &integral, dx, dy),
                    None => ncc_at(&o2b, &win, dx, dy, w2r),
                };
                if c > best.ncc {
                    best = Fit {
                        ncc: c,
                        scale: s,
                        dx: dx as i32,
                        dy: dy as i32,
                        ..Fit::none()
                    };
                }
            }
        }
    }
    if best.ncc < 0.0 {
        return best;
    }
    // refine at full size: ±1 scale step, ±2 px around the coarse best
    let (n1, w1r) = outer.radii();
    let ob = outer.band_pass(n1, w1r);
    let mut refined = Fit::none();
    for ds in [-0.05f32, 0.0, 0.05] {
        let s = (best.scale + ds).clamp(SCALE_MIN, 1.0);
        let w = ((outer.w as f32 * s).round() as usize).max(8);
        let h = ((w as f32 * aspect).round() as usize).max(8);
        if w > outer.w || h > outer.h {
            continue;
        }
        let win = inner.resized(w, h).band_pass(n1, w1r);
        let cx = best.dx * 2;
        let cy = best.dy * 2;
        for dy in (cy - 2)..=(cy + 2) {
            for dx in (cx - 2)..=(cx + 2) {
                if dx < 0 || dy < 0 || dx as usize + w > outer.w || dy as usize + h > outer.h {
                    continue;
                }
                let c = ncc_at(&ob, &win, dx as usize, dy as usize, w1r);
                if c > refined.ncc {
                    refined = Fit {
                        ncc: c,
                        scale: s,
                        dx,
                        dy,
                        ..Fit::none()
                    };
                }
            }
        }
    }
    let refined_won = refined.ncc > best.ncc;
    let mut out = if refined_won { refined } else { best };
    if robust {
        // the cells at the chosen placement, at the size it was found at
        out.robust = if refined_won {
            let w = ((outer.w as f32 * out.scale).round() as usize).max(8);
            let h = ((w as f32 * aspect).round() as usize).max(8);
            let win = inner.resized(w, h).band_pass(n1, w1r);
            ncc_cells(&ob, &win, out.dx as usize, out.dy as usize, w1r)
        } else {
            let w2 = ((o2.w as f32 * out.scale).round() as usize).max(4);
            let h2 = ((w2 as f32 * aspect).round() as usize).max(4);
            let win = i2.resized(w2, h2).band_pass(n2, w2r);
            ncc_cells(&o2b, &win, out.dx as usize, out.dy as usize, w2r)
        };
    }
    out
}

/// Small rotations of `inner` against `outer` at full frame (both
/// resized to the outer's size), offsets ±2 px.
fn best_rotation(outer: &Thumb, inner: &Thumb, robust: bool) -> Fit {
    let (n1, w1r) = outer.radii();
    let ob = outer.band_pass(n1, w1r);
    let same = inner.resized(outer.w, outer.h).band_pass(n1, w1r);
    let mut best = Fit::none();
    for &deg in ANGLES.iter() {
        if deg == 0.0 {
            continue; // the containment search covers it
        }
        let rot = same.rotated(deg);
        let c = ncc_at(&ob, &rot, 0, 0, w1r);
        if c > best.ncc {
            best = Fit {
                ncc: c,
                scale: 1.0,
                angle: deg,
                robust: if robust {
                    ncc_cells(&ob, &rot, 0, 0, w1r)
                } else {
                    -1.0
                },
                ..Fit::none()
            };
        }
    }
    best
}

/// `inner` stretched onto `outer`'s whole frame, aspect ignored (a
/// squeezed or stretched copy).
fn stretched(outer: &Thumb, inner: &Thumb, robust: bool) -> Fit {
    let (n1, w1r) = outer.radii();
    let ob = outer.band_pass(n1, w1r);
    let ib = inner.resized(outer.w, outer.h).band_pass(n1, w1r);
    Fit {
        ncc: ncc_at(&ob, &ib, 0, 0, w1r),
        scale: 1.0,
        robust: if robust {
            ncc_cells(&ob, &ib, 0, 0, w1r)
        } else {
            -1.0
        },
        ..Fit::none()
    }
}

/// The fit between two pictures: the best of `b` inside `a`, `a` inside
/// `b`, a stretch either way, and small rotations either way. Every
/// comparison band-passes both sides at the containing picture's scale.
pub fn fit(a: &Thumb, b: &Thumb) -> Fit {
    fit_with(a, b, SEARCH)
}

/// The search `fit` ships with: the cells' score and the mirror image too
/// (CORE-01, CORE-02; VERIFICATION § Phase 11). Measured on real pictures
/// against the plain search: a caption bar over the lower 15 % confirms at
/// 98 % instead of 20 %, a mirrored crop at 97 % instead of 3 %; among the
/// pairs the hashes or the descriptor admit, the closest a photograph of
/// the same subject from another position comes is 0.823 (two poses of
/// one chessboard). The fit alone is less specific
/// for it (unrelated photographs at 0.92 or more: 13 of 3,062 instead of
/// 3), which is why it is only ever asked about a pair a first family
/// admitted.
pub const SEARCH: Search = Search {
    robust: true,
    mirror: true,
};

/// `fit` with a chosen search: the plain one, the cells' score, the
/// mirror image, or both. Placements are chosen by the plain correlation;
/// the pair's score is the best `Fit::score` over them.
pub fn fit_with(a: &Thumb, b: &Thumb, search: Search) -> Fit {
    let mut best = one_way(a, b, search.robust);
    if search.mirror {
        let m = one_way(a, &b.mirrored(), search.robust);
        if m.score() > best.score() {
            best = Fit {
                mirrored: true,
                ..m
            };
        }
    }
    best
}

fn one_way(a: &Thumb, b: &Thumb, robust: bool) -> Fit {
    let mut best = best_containment(a, b, robust);
    for (f, second) in [
        (best_containment(b, a, robust), false),
        (stretched(a, b, robust), true),
        (stretched(b, a, robust), false),
        (best_rotation(a, b, robust), true),
        (best_rotation(b, a, robust), false),
    ] {
        if f.score() > best.score() {
            best = Fit {
                inner_is_second: second,
                ..f
            };
        }
    }
    best
}

/// The fit's thumbnail of a file, decoded as small as its format allows
/// with the long side still at four times the thumbnail's (a 12-megapixel
/// JPEG at a quarter): measured against the full decode, copies' scores
/// within 0.003 and no decision changed, where twice the side moved a
/// letterboxed copy 0.022 (VERIFICATION § Phase 14). `None` when it does
/// not decode.
pub fn thumb_of(path: &std::path::Path) -> Option<Thumb> {
    let img = crate::decode::load_small(path, 4 * MAX_SIDE as u32).ok()?;
    Some(Thumb::from_image(&img, MAX_SIDE))
}

/// Decode both files and fit them. `None` when either does not decode.
pub fn fit_files(a: &std::path::Path, b: &std::path::Path) -> Option<Fit> {
    Some(fit(&thumb_of(a)?, &thumb_of(b)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgb, RgbImage};

    /// A picture with structure at several scales (so a crop is locatable).
    fn picture(seed: u32) -> RgbImage {
        let (w, h) = (400u32, 300u32);
        ImageBuffer::from_fn(w, h, |x, y| {
            let s = seed as f32;
            let v = 128.0
                + 60.0 * ((x as f32 / 37.0 + s).sin() * (y as f32 / 23.0 - s).cos())
                + 40.0 * (((x / 50 + y / 50 + seed) % 2) as f32 - 0.5)
                + 20.0 * ((x as f32 * 0.21 + y as f32 * 0.13 + s * 3.0).sin());
            let v = v.clamp(0.0, 255.0) as u8;
            Rgb([v, (v as u16 * 7 / 8) as u8, (v as u16 * 3 / 4) as u8])
        })
    }

    fn thumb(img: &RgbImage) -> Thumb {
        Thumb::from_image(&DynamicImage::ImageRgb8(img.clone()), MAX_SIDE)
    }

    #[test]
    fn identical_fits_perfectly() {
        let a = picture(1);
        let f = fit(&thumb(&a), &thumb(&a));
        assert!(f.ncc > 0.999, "{f:?}");
        assert_eq!(f.angle, 0.0);
    }

    #[test]
    fn crop_is_located_at_its_scale() {
        let a = picture(2);
        let crop = image::imageops::crop_imm(&a, 60, 45, 280, 210).to_image(); // 70 %
        let f = fit(&thumb(&a), &thumb(&crop));
        assert!(f.ncc > 0.95, "{f:?}");
        assert!((f.scale - 0.70).abs() < 0.06, "{f:?}");
        assert!(f.inner_is_second, "{f:?}");
    }

    #[test]
    fn letterbox_is_found_the_other_way_round() {
        let a = picture(3);
        let mut lb: RgbImage = ImageBuffer::from_pixel(400, 380, Rgb([0, 0, 0]));
        image::imageops::overlay(&mut lb, &a, 0, 40);
        let f = fit(&thumb(&a), &thumb(&lb));
        assert!(f.ncc > 0.95, "{f:?}");
        assert!(
            !f.inner_is_second,
            "the original sits inside the letterbox\n{f:?}"
        );
    }

    #[test]
    fn small_rotation_is_recovered() {
        let a = picture(4);
        // rotate by 4° with the same bilinear scheme the corpus uses
        let (w, h) = (a.width(), a.height());
        let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
        let (s, c) = 4.0f32.to_radians().sin_cos();
        let rot: RgbImage = ImageBuffer::from_fn(w, h, |x, y| {
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            let sx = (cx + dx * c + dy * s).round();
            let sy = (cy - dx * s + dy * c).round();
            if sx < 0.0 || sy < 0.0 || sx >= w as f32 || sy >= h as f32 {
                Rgb([128, 112, 96])
            } else {
                *a.get_pixel(sx as u32, sy as u32)
            }
        });
        let f = fit(&thumb(&a), &thumb(&rot));
        assert!(f.ncc > 0.9, "{f:?}");
        assert!(f.angle.abs() >= 2.0, "{f:?}");
    }

    #[test]
    fn different_pictures_do_not_fit() {
        let f = fit(&thumb(&picture(5)), &thumb(&picture(9)));
        assert!(!f.confirmed(), "{f:?}");
    }

    #[test]
    fn a_mirrored_crop_and_a_caption_bar_are_found() {
        let a = picture(7);
        let (w, h) = (a.width(), a.height());
        let mirrored = image::imageops::flip_horizontal(&a);
        let crop =
            image::imageops::crop_imm(&mirrored, w / 10, h / 10, w * 8 / 10, h * 8 / 10).to_image();
        let f = fit(&thumb(&a), &thumb(&crop));
        assert!(f.confirmed() && f.mirrored, "{f:?}");
        assert!(!fit_with(&thumb(&a), &thumb(&crop), Search::PLAIN).confirmed());
        // a dark caption bar with light text over the lower 15 %
        let mut cap = a.clone();
        for y in h - h * 15 / 100..h {
            for x in 0..w {
                let text = y % 9 < 5 && (x / 7) % 3 != 0;
                cap.put_pixel(
                    x,
                    y,
                    if text {
                        Rgb([235, 235, 235])
                    } else {
                        Rgb([18, 18, 22])
                    },
                );
            }
        }
        let f = fit(&thumb(&a), &thumb(&cap));
        assert!(f.confirmed() && f.robust > f.ncc, "{f:?}");
        assert!(f.label().starts_with("fit "), "{}", f.label());
    }

    #[test]
    fn brightness_does_not_matter() {
        let a = picture(6);
        let mut bright = a.clone();
        for p in bright.pixels_mut() {
            for c in p.0.iter_mut() {
                *c = (*c as f32 * 1.3 + 10.0).min(255.0) as u8;
            }
        }
        let f = fit(&thumb(&a), &thumb(&bright));
        assert!(f.ncc > 0.98, "{f:?}");
    }
}
