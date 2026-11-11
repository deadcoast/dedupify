//! Bars around a video's picture: black ones through ffmpeg's
//! `cropdetect` (Phase 7), bars of any flat colour through the detail
//! along each edge band (Phase 11, CORE-03).

use std::path::Path;
use std::process::Stdio;

use super::{Meta, FFMPEG_THREADS};

/// The picture inside bars: a pillarbox (bars left and right) or a
/// letterbox (bars above and below), in source pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bars {
    pub width: u32,
    pub height: u32,
    pub x: u32,
    pub y: u32,
}

/// Luma at or under this (of 255) counts as black — ffmpeg's `cropdetect`
/// default (24/255). Limited-range video black is 16; compression noise
/// stays well under 24 on a bar.
pub const BAR_LIMIT: u32 = 24;

/// Bars thinner than this share of a side are left alone: a 2 px edge is
/// an encoder's, not a bar's. [CLAUDE]
pub const BAR_MIN_SHARE: f64 = 0.02;

/// A detected picture smaller than this share of either side is not a
/// picture inside bars but a dark video; nothing is cropped. [CLAUDE]
pub const BAR_KEEP_MIN: f64 = 0.25;

/// Parse the last `crop=W:H:X:Y` ffmpeg's `cropdetect` printed. With
/// `reset=0` the box only grows across frames, so the last line is the
/// union over every frame seen.
pub fn parse_cropdetect(stderr: &str, width: u32, height: u32) -> Option<Bars> {
    let last = stderr.lines().rev().find_map(|l| {
        let i = l.find("crop=")?;
        Some(&l[i + 5..])
    })?;
    let nums: Vec<u32> = last
        .split(':')
        .take(4)
        .map(|s| s.trim_end().parse().ok())
        .collect::<Option<Vec<u32>>>()?;
    if nums.len() != 4 {
        return None;
    }
    let (w, h, x, y) = (nums[0], nums[1], nums[2], nums[3]);
    if w == 0 || h == 0 || x + w > width || y + h > height {
        return None;
    }
    let removed_w = (width - w) as f64 / width as f64;
    let removed_h = (height - h) as f64 / height as f64;
    if removed_w < BAR_MIN_SHARE && removed_h < BAR_MIN_SHARE {
        return None; // no bars worth the name
    }
    if (w as f64) < BAR_KEEP_MIN * width as f64 || (h as f64) < BAR_KEEP_MIN * height as f64 {
        return None; // a dark video, not a picture inside bars
    }
    Some(Bars {
        width: w,
        height: h,
        x,
        y,
    })
}

/// Find black bars with ffmpeg's `cropdetect` over the keyframes alone
/// (`-skip_frame nokey`: no inter-frame decoding, so a fraction of the
/// main pass), the box accumulated over the whole file so a dark opening
/// or a dark edge in one scene cannot pass for a bar. `None` when there
/// are none, or when ffmpeg says nothing usable.
pub fn detect_bars(path: &Path, meta: &Meta) -> Option<Bars> {
    let out = crate::tools::ffmpeg()
        .args([
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "info",
            "-threads",
            &FFMPEG_THREADS.to_string(),
            "-skip_frame",
            "nokey",
        ])
        .arg("-i")
        .arg(path)
        .args([
            "-an",
            "-sn",
            "-dn",
            "-vf",
            &format!("cropdetect=limit={BAR_LIMIT}:round=2:reset=0:skip=0"),
            "-f",
            "null",
            "-",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .ok()?;
    parse_cropdetect(
        &String::from_utf8_lossy(&out.stderr),
        meta.width,
        meta.height,
    )
}

/// A band at the frame's edge counts as flat when its detail, in every
/// sampled frame, is under this share of the picture's (the median of
/// the middle half). Measured on 25 real clips framed as platforms post
/// them and 79 clips with no bars (docs/VERIFICATION.md § Phase 11,
/// CORE-03): a white, grey or dark-blue pillarbox carries almost no
/// detail (the sweep found the same of them at 0.15 as at 0.25), a
/// blurred "ambient" one 0.10–0.97 of the picture's (7 of 25 found here,
/// 11 at 0.30); the first clip without bars to show some, a thermal
/// camera's fixed overlay strips, does at 0.33.
pub const FLAT_RATIO: f64 = 0.25;

/// A frame whose picture has less detail than this (mean grey-level step
/// per pixel at 256 wide) — a fade, a black or a uniform frame — cannot
/// tell bars from picture and is set aside; the other frames decide.
/// [CLAUDE]
pub const FLAT_PICTURE_MIN: f64 = 1.0;

/// The picture starts at once: its first line (or the next) carries at
/// least this share of the picture's detail, so a flat stretch of the
/// picture itself (a sky, a wall) that fades into texture is not a bar.
/// Measured with `FLAT_RATIO` (VERIFICATION § Phase 11, CORE-03): 0.5
/// missed two clips whose edge column is dark, 0.4 finds them, and the
/// first clip without bars to show some does at 0.3.
pub const FLAT_START: f64 = 0.4;

/// Keyframes sampled for the flat-bar pass.
pub const FLAT_FRAMES: usize = 16;

/// Width the flat-bar pass samples at.
pub const FLAT_WIDTH: u32 = 256;

/// Bars of any colour — white, grey, a brand colour, or the blurred,
/// enlarged copy of the picture a platform puts around a portrait video
/// (CORE-03, Phase 11) — as the inner picture's edges in fractions of
/// the frame (left, top, right, bottom). Two opposite edge bands count
/// when, in every frame whose picture has detail, their detail stays
/// under `FLAT_RATIO` of the picture's; each band ends where the
/// picture's detail starts at once (the first picture line, or the next
/// when the edge falls inside a sampled pixel, carries `FLAT_START` of the
/// picture's detail);
/// the two bands are within 2 % of the side of each other's width; and
/// they leave `BAR_KEEP_MIN` of the side. Detail in a column is measured
/// along the column (vertical steps), in a row along the row, so the
/// step at a bar's own edge never counts as the bar's.
pub fn flat_bars(frames: &[image::GrayImage]) -> Option<(f64, f64, f64, f64)> {
    let first = frames.first()?;
    let (w, h) = (first.width() as usize, first.height() as usize);
    if w < 16
        || h < 16
        || frames
            .iter()
            .any(|f| f.width() as usize != w || f.height() as usize != h)
    {
        return None;
    }
    // per frame: detail of each column and of each row
    let mut cols: Vec<Vec<f64>> = Vec::new();
    let mut rows: Vec<Vec<f64>> = Vec::new();
    for f in frames {
        let px = f.as_raw();
        let at = |x: usize, y: usize| px[y * w + x] as f64;
        cols.push(
            (0..w)
                .map(|x| {
                    (0..h - 1)
                        .map(|y| (at(x, y + 1) - at(x, y)).abs())
                        .sum::<f64>()
                        / (h - 1) as f64
                })
                .collect(),
        );
        rows.push(
            (0..h)
                .map(|y| {
                    (0..w - 1)
                        .map(|x| (at(x + 1, y) - at(x, y)).abs())
                        .sum::<f64>()
                        / (w - 1) as f64
                })
                .collect(),
        );
    }
    let median_mid = |v: &[f64]| {
        let n = v.len();
        let mut mid = v[n / 4..n - n / 4].to_vec();
        mid.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        mid[mid.len() / 2]
    };
    // the leading and trailing flat runs along one axis, and whether they
    // are bars
    let bands = |per: &[Vec<f64>], n: usize| -> Option<(usize, usize)> {
        // frames whose picture has too little detail to judge are set aside
        let used: Vec<(&Vec<f64>, f64)> = per
            .iter()
            .map(|v| (v, median_mid(v)))
            .filter(|&(_, p)| p >= FLAT_PICTURE_MIN)
            .collect();
        if used.is_empty() {
            return None;
        }
        let flat = |i: usize| used.iter().all(|&(v, p)| v[i] < FLAT_RATIO * p);
        let lead = (0..n).take_while(|&i| flat(i)).count();
        let trail = (0..n).rev().take_while(|&i| flat(i)).count();
        let min = ((BAR_MIN_SHARE * n as f64).ceil() as usize).max(2);
        if lead < min || trail < min || lead + trail >= n {
            return None;
        }
        let tol = ((0.02 * n as f64).ceil() as usize).max(2);
        if lead.abs_diff(trail) > tol || ((n - lead - trail) as f64) < BAR_KEEP_MIN * n as f64 {
            return None;
        }
        // the picture starts at once on both sides: its first line, or the
        // next when the edge fell inside a sampled pixel
        let mean_at =
            |i: usize| used.iter().map(|&(v, p)| v[i] / p).sum::<f64>() / used.len() as f64;
        let starts = |a: usize, b: usize| mean_at(a).max(mean_at(b)) >= FLAT_START;
        if !starts(lead, lead + 1) || !starts(n - 1 - trail, n - 2 - trail) {
            return None;
        }
        Some((lead, trail))
    };
    let x = bands(&cols, w);
    let y = bands(&rows, h);
    if x.is_none() && y.is_none() {
        return None;
    }
    let (l, r) = x.unwrap_or((0, 0));
    let (t, b) = y.unwrap_or((0, 0));
    Some((
        l as f64 / w as f64,
        t as f64 / h as f64,
        (w - r) as f64 / w as f64,
        (h - b) as f64 / h as f64,
    ))
}

/// The flat-bar pass on a file: up to `FLAT_FRAMES` keyframes at
/// `FLAT_WIDTH` wide, upright as the frame pass sees them; the inner
/// picture mapped onto the decoded frame's size (the probed size, turned
/// when the decoder turned the picture).
pub fn detect_flat_bars(path: &Path, meta: &Meta) -> Option<Bars> {
    let out = crate::tools::ffmpeg()
        .args([
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "error",
            "-threads",
            &FFMPEG_THREADS.to_string(),
            "-skip_frame",
            "nokey",
        ])
        .arg("-i")
        .arg(path)
        .args([
            "-an",
            "-sn",
            "-dn",
            "-vf",
            &format!("scale={FLAT_WIDTH}:-2:flags=area"),
            "-frames:v",
            &FLAT_FRAMES.to_string(),
            // each keyframe once: without it the output's frame rate
            // repeats the first keyframe in place of the others
            "-fps_mode",
            "passthrough",
            "-f",
            "image2pipe",
            "-c:v",
            "ppm",
            "pipe:1",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let frames: Vec<image::GrayImage> = crate::decode::ppm_frames(&out.stdout)?
        .into_iter()
        .map(|f| image::DynamicImage::ImageRgb8(f).to_luma8())
        .collect();
    let (fw, fh) = (frames.first()?.width(), frames.first()?.height());
    let (l, t, r, b) = flat_bars(&frames)?;
    // the decoded size: the probed one, turned if the frames are
    let (w, h) = if (fw >= fh) == (meta.width >= meta.height) {
        (meta.width, meta.height)
    } else {
        (meta.height, meta.width)
    };
    let even = |v: f64| ((v / 2.0).round() as u32) * 2;
    let (x0, y0) = (even(l * w as f64), even(t * h as f64));
    let (x1, y1) = (even(r * w as f64).min(w), even(b * h as f64).min(h));
    (x1 > x0 && y1 > y0).then_some(Bars {
        width: x1 - x0,
        height: y1 - y0,
        x: x0,
        y: y0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 256×144 frame: textured picture in `[x0, x1)` × `[y0, y1)`, the
    /// rest filled by `bar(x, y)`.
    fn framed(
        x0: u32,
        x1: u32,
        y0: u32,
        y1: u32,
        seed: u32,
        bar: &dyn Fn(u32, u32) -> u8,
    ) -> image::GrayImage {
        image::GrayImage::from_fn(256, 144, |x, y| {
            if x >= x0 && x < x1 && y >= y0 && y < y1 {
                let v = ((x * 37 + y * 91 + seed * 13) % 97) as f32 / 97.0;
                let w = ((x as f32 * 0.4 + seed as f32).sin() * (y as f32 * 0.3).cos() + 1.0) / 2.0;
                image::Luma([(40.0 + 170.0 * (0.5 * v + 0.5 * w)) as u8])
            } else {
                image::Luma([bar(x, y)])
            }
        })
    }

    #[test]
    fn flat_bars_find_white_and_blurred_bars_and_leave_pictures() {
        // a portrait picture in a 16:9 frame: white bars, then a blurred
        // band (a slow ramp, as an enlarged and blurred copy looks)
        let frames: Vec<_> = (0..4)
            .map(|s| framed(88, 168, 0, 144, s, &|_, _| 250))
            .collect();
        let (l, t, r, b) = flat_bars(&frames).expect("white bars");
        assert!(
            (l - 88.0 / 256.0).abs() < 0.01 && (r - 168.0 / 256.0).abs() < 0.01,
            "{l} {r}"
        );
        assert_eq!((t, b), (0.0, 1.0));
        let ambient: Vec<_> = (0..4)
            .map(|s| {
                framed(88, 168, 0, 144, s, &|x, y| {
                    (90 + (x / 6 + y / 9 + s) % 40) as u8
                })
            })
            .collect();
        let (l, _, r, _) = flat_bars(&ambient).expect("blurred bars");
        assert!(
            (l - 88.0 / 256.0).abs() < 0.02 && (r - 168.0 / 256.0).abs() < 0.02,
            "{l} {r}"
        );
        // a letterbox in grey
        let lb: Vec<_> = (0..3)
            .map(|s| framed(0, 256, 18, 126, s, &|_, _| 128))
            .collect();
        let (_, t, _, b) = flat_bars(&lb).expect("grey letterbox");
        assert!(
            (t - 18.0 / 144.0).abs() < 0.02 && (b - 126.0 / 144.0).abs() < 0.02,
            "{t} {b}"
        );
        // no bars; a flat sky along one edge only; a flat picture
        let plain: Vec<_> = (0..4)
            .map(|s| framed(0, 256, 0, 144, s, &|_, _| 0))
            .collect();
        assert_eq!(flat_bars(&plain), None);
        let sky: Vec<_> = (0..4)
            .map(|s| framed(0, 256, 30, 144, s, &|_, _| 200))
            .collect();
        assert_eq!(flat_bars(&sky), None);
        let flat: Vec<_> = (0..2)
            .map(|_| image::GrayImage::from_pixel(256, 144, image::Luma([60])))
            .collect();
        assert_eq!(flat_bars(&flat), None);
        // bands of very different widths are not bars around a picture
        let lopsided: Vec<_> = (0..3)
            .map(|s| framed(20, 200, 0, 144, s, &|_, _| 250))
            .collect();
        assert_eq!(flat_bars(&lopsided), None);
        // a black frame among them (a fade) is set aside, not a veto
        let mut faded = frames.clone();
        faded.insert(1, image::GrayImage::from_pixel(256, 144, image::Luma([0])));
        assert_eq!(flat_bars(&faded), flat_bars(&frames));
        // the picture's edge inside a sampled pixel: its first and last
        // columns carry a third of the detail, the next ones all of it
        let soft: Vec<_> = frames
            .iter()
            .map(|f| {
                let mut f = f.clone();
                for y in 0..144 {
                    for x in [88u32, 167] {
                        let v = f.get_pixel(x, y)[0] as f32;
                        f.put_pixel(x, y, image::Luma([(250.0 + 0.33 * (v - 250.0)) as u8]));
                    }
                }
                f
            })
            .collect();
        let (l, _, r, _) = flat_bars(&soft).expect("a soft edge");
        assert!(
            (l - 88.0 / 256.0).abs() < 0.01 && (r - 168.0 / 256.0).abs() < 0.01,
            "{l} {r}"
        );
    }
}
