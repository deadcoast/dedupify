//! Near-duplicate video (docs/RESEARCH.md §3).
//!
//! One `ffprobe` call for metadata, one short `ffmpeg` pass over the
//! keyframes alone to find black bars around the picture (and, when it
//! finds none, a second for bars of any flat colour), then one
//! `ffmpeg` process per file streaming 15 fps, 288×288 grey frames of the
//! picture inside the bars through a pipe. Every frame feeds the TMK
//! descriptor (Temporal Matching Kernel on PDQ float features — Poullot
//! et al. 2015; Meta's `tmkfv.cpp` for the constants); twelve frames at
//! fixed fractions of the duration also get a full image signature, which
//! gives the fast positional tier for same-length copies.
//!
//! Nothing is written to disk; frames are hashed as they arrive.

use std::io::{self, Read};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use rayon::prelude::*;

use crate::exact::{Progress, Stage};
use crate::pdq::{self, PDQ_MAX};
use crate::perceptual::{self, Signature, Variant, DHASH_MAX, GRID, PHASH_MAX, VARIANT_ORDER};

/// Frames per second the stream is resampled to (TMK default).
pub const FPS: u32 = 15;
/// Side of the grey frame requested from ffmpeg; equals the image grid so
/// the same dHash/pHash code runs on frames.
pub const FRAME: usize = GRID;
/// Positions sampled for the positional tier.
pub const POSITIONS: usize = 12;
/// Fewest positions both videos must share for the positional tier to speak.
pub const POSITIONS_MIN: usize = 8;

/// TMK: periods in frames at 15 fps (Poullot's list) and the 32 Fourier
/// coefficients a_j = I_j(β)/sinh(β), a_0 = (I_0(β) − e^−β)/(2 sinh β), β = 32,
/// which make Σ a_j cos(jδ) a von Mises kernel in the time offset δ.
pub const PERIODS: [u32; 4] = [2731, 4391, 9767, 14653];
/// Copied digit for digit from the reference table.
#[allow(clippy::excessive_precision)]
pub const COEFFS: [f32; 32] = [
    0.0708041893112,
    0.13937789309,
    0.132897260304,
    0.122765735552,
    0.109878684888,
    0.09529606433,
    0.0800986647852,
    0.0652590650356,
    0.0515478238322,
    0.0394851531195,
    0.0293374252025,
    0.0211492623679,
    0.0147973073245,
    0.0100512818746,
    0.0066306408014,
    0.00424947117334,
    0.0026467615764,
    0.00160270959695,
    0.000943882629639,
    0.000540841638603,
    0.000301633183798,
    0.000163800158855,
    8.66454753015e-05,
    4.46626303151e-05,
    2.24429442235e-05,
    1.09982139799e-05,
    5.25823487999e-06,
    2.45358229988e-06,
    1.11781474895e-06,
    4.97406489221e-07,
    2.16265487234e-07,
    9.19087006565e-08,
];
pub const DIM: usize = 256;

/// TMK thresholds. The reference README suggests 0.7 for both levels; on
/// the generated corpus (docs/VERIFICATION.md) 0.7/0.7 produced 19 false
/// groups among 3,000 negative pairs — two different zooms into the same
/// fractal share a mean appearance — while level 2 alone separated every
/// negative (max 0.769) from every trim (min 0.785). Level 1 is kept as a
/// cheap gate before the level-2 offset search **[CLAUDE]**.
pub const LEVEL1_MIN: f32 = 0.5;
/// Grouping (`find`, `milk`): 0 false groups, 99.3 % of TMK-carried edits.
pub const LEVEL2_MIN: f32 = 0.8;
/// Binning (`purge`): a 0.13 margin over the closest negative; trims that
/// score between 0.8 and 0.9 are grouped, listed as `kept~`, and left to `milk`.
pub const LEVEL2_CONFIRM: f32 = 0.9;

/// ffmpeg threads per process and concurrent processes.
pub const FFMPEG_THREADS: usize = 2;
pub fn concurrency() -> usize {
    (std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2)
        / 2)
    .clamp(1, 4)
}

// ---------------------------------------------------------------------------
// probing
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct Meta {
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    pub codec: String,
    pub fps: f64,
}

#[derive(Debug)]
pub enum VideoError {
    Spawn(io::Error),
    Probe(String),
    NoVideoStream,
    NoFrames,
    Io(io::Error),
}

impl std::fmt::Display for VideoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VideoError::Spawn(e) => write!(f, "cannot start ffmpeg/ffprobe: {e}"),
            VideoError::Probe(s) => write!(f, "ffprobe: {s}"),
            VideoError::NoVideoStream => write!(f, "no video stream"),
            VideoError::NoFrames => write!(f, "no frames decoded"),
            VideoError::Io(e) => write!(f, "{e}"),
        }
    }
}

pub fn tools_present() -> (bool, bool) {
    let ok = |b: &str| {
        Command::new(b)
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    };
    (ok("ffmpeg"), ok("ffprobe"))
}

fn json_str<'a>(v: &'a serde_json::Value, keys: &[&str]) -> Option<&'a str> {
    let mut cur = v;
    for k in keys {
        cur = cur.get(*k)?;
    }
    cur.as_str()
}

fn json_u64(v: &serde_json::Value, keys: &[&str]) -> Option<u64> {
    let mut cur = v;
    for k in keys {
        cur = cur.get(*k)?;
    }
    cur.as_u64()
}

fn parse_rate(s: &str) -> f64 {
    match s.split_once('/') {
        Some((a, b)) => {
            let a: f64 = a.parse().unwrap_or(0.0);
            let b: f64 = b.parse().unwrap_or(1.0);
            if b == 0.0 {
                0.0
            } else {
                a / b
            }
        }
        None => s.parse().unwrap_or(0.0),
    }
}

/// `ffprobe` the first video stream.
pub fn probe(path: &Path) -> Result<Meta, VideoError> {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=codec_name,width,height,r_frame_rate,avg_frame_rate,duration:format=duration",
            "-of",
            "json",
        ])
        .arg(path)
        .stdin(Stdio::null())
        .output()
        .map_err(VideoError::Spawn)?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(VideoError::Probe(if err.is_empty() {
            "exit status".into()
        } else {
            err
        }));
    }
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).map_err(|e| VideoError::Probe(e.to_string()))?;
    let stream = v
        .get("streams")
        .and_then(|s| s.as_array())
        .and_then(|a| a.first())
        .ok_or(VideoError::NoVideoStream)?;
    let width = json_u64(stream, &["width"]).unwrap_or(0) as u32;
    let height = json_u64(stream, &["height"]).unwrap_or(0) as u32;
    if width == 0 || height == 0 {
        return Err(VideoError::NoVideoStream);
    }
    let codec = json_str(stream, &["codec_name"]).unwrap_or("?").to_string();
    let fps = json_str(stream, &["avg_frame_rate"])
        .map(parse_rate)
        .filter(|f| *f > 0.0)
        .or_else(|| json_str(stream, &["r_frame_rate"]).map(parse_rate))
        .unwrap_or(0.0);
    let duration = json_str(&v, &["format", "duration"])
        .and_then(|s| s.parse::<f64>().ok())
        .or_else(|| json_str(stream, &["duration"]).and_then(|s| s.parse::<f64>().ok()))
        .unwrap_or(0.0);
    Ok(Meta {
        duration,
        width,
        height,
        codec,
        fps,
    })
}

// ---------------------------------------------------------------------------
// TMK descriptor
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct Tmk {
    /// Mean raw PDQ float feature, DIM values.
    pub pure: Vec<f32>,
    /// cos[(period·32 + j)·DIM + k], normalised and scaled by √a_j.
    pub cos: Vec<f32>,
    pub sin: Vec<f32>,
    pub frames: u32,
}

pub struct TmkBuilder {
    pure: Vec<f32>,
    cos: Vec<f32>,
    sin: Vec<f32>,
    frames: u32,
}

impl Default for TmkBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl TmkBuilder {
    pub fn new() -> TmkBuilder {
        let n = PERIODS.len() * COEFFS.len() * DIM;
        TmkBuilder {
            pure: vec![0.0; DIM],
            cos: vec![0.0; n],
            sin: vec![0.0; n],
            frames: 0,
        }
    }

    /// Ingest frame `t` (index at FPS) with its raw PDQ float feature.
    pub fn ingest(&mut self, t: u32, feature: &[f32; DIM]) {
        for (p, f) in self.pure.iter_mut().zip(feature) {
            *p += *f;
        }
        let norm = feature.iter().map(|x| x * x).sum::<f32>().sqrt();
        let inv = if norm > 0.0 { 1.0 / norm } else { 0.0 };
        let mut unit = [0.0f32; DIM];
        for (u, f) in unit.iter_mut().zip(feature) {
            *u = f * inv;
        }
        for (pi, period) in PERIODS.iter().enumerate() {
            let theta = std::f64::consts::TAU * t as f64 / *period as f64;
            let (s1, c1) = theta.sin_cos();
            // cos(jθ), sin(jθ) by rotation
            let mut cj = 1.0f64;
            let mut sj = 0.0f64;
            for j in 0..COEFFS.len() {
                let base = (pi * COEFFS.len() + j) * DIM;
                if j == 0 {
                    for (c, u) in self.cos[base..base + DIM].iter_mut().zip(&unit) {
                        *c += *u;
                    }
                } else {
                    let (cjf, sjf) = (cj as f32, sj as f32);
                    let cs = &mut self.cos[base..base + DIM];
                    let ss = &mut self.sin[base..base + DIM];
                    for k in 0..DIM {
                        cs[k] += unit[k] * cjf;
                        ss[k] += unit[k] * sjf;
                    }
                }
                let ncj = cj * c1 - sj * s1;
                let nsj = sj * c1 + cj * s1;
                cj = ncj;
                sj = nsj;
            }
        }
        self.frames += 1;
    }

    pub fn finish(mut self) -> Tmk {
        if self.frames > 0 {
            let inv = 1.0 / self.frames as f32;
            for p in &mut self.pure {
                *p *= inv;
            }
        }
        for pi in 0..PERIODS.len() {
            for (j, a) in COEFFS.iter().enumerate() {
                let base = (pi * COEFFS.len() + j) * DIM;
                let scale = a.sqrt();
                l2_normalize_scale(&mut self.cos[base..base + DIM], scale);
                l2_normalize_scale(&mut self.sin[base..base + DIM], scale);
            }
        }
        Tmk {
            pure: self.pure,
            cos: self.cos,
            sin: self.sin,
            frames: self.frames,
        }
    }
}

fn l2_normalize_scale(v: &mut [f32], scale: f32) {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if n > 0.0 {
        let f = scale / n;
        for x in v {
            *x *= f;
        }
    }
}

#[inline]
fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let na = dot(a, a).sqrt();
    let nb = dot(b, b).sqrt();
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot(a, b) / (na * nb)
    }
}

/// Level 1: cosine similarity of the mean features.
pub fn level1(a: &Tmk, b: &Tmk) -> f32 {
    cosine(&a.pure, &b.pure)
}

/// Level 2: the kernel maximised over time offsets, per period, then the
/// best period; normalised to [−1, 1]. Returns (score, offset in seconds
/// from the best period, modulo that period).
pub fn level2(a: &Tmk, b: &Tmk) -> (f32, f32) {
    let m = COEFFS.len();
    let normalizer: f32 = COEFFS[0] + 2.0 * COEFFS[1..].iter().sum::<f32>();
    let mut best = f32::MIN;
    let mut best_offset = 0.0f32;
    for (pi, period) in PERIODS.iter().enumerate() {
        let mut cc = [0.0f32; 32];
        let mut ss = [0.0f32; 32];
        let mut sc = [0.0f32; 32];
        let mut cs = [0.0f32; 32];
        for j in 0..m {
            let base = (pi * m + j) * DIM;
            let (ca, cb) = (&a.cos[base..base + DIM], &b.cos[base..base + DIM]);
            let (sa, sb) = (&a.sin[base..base + DIM], &b.sin[base..base + DIM]);
            cc[j] = dot(ca, cb);
            ss[j] = dot(sa, sb);
            sc[j] = dot(sa, cb);
            cs[j] = dot(ca, sb);
        }
        let mut period_best = f32::MIN;
        let mut period_offset = 0u32;
        for offset in 0..*period {
            let delta = std::f64::consts::TAU * offset as f64 / *period as f64;
            let (sd, cd) = delta.sin_cos();
            let mut k = cc[0];
            let mut cj = 1.0f64;
            let mut sj = 0.0f64;
            for j in 1..m {
                let ncj = cj * cd - sj * sd;
                let nsj = sj * cd + cj * sd;
                cj = ncj;
                sj = nsj;
                k += cj as f32 * (cc[j] + ss[j]) + sj as f32 * (sc[j] - cs[j]);
            }
            if k > period_best {
                period_best = k;
                period_offset = offset;
            }
        }
        if period_best > best {
            best = period_best;
            best_offset = period_offset as f32 / FPS as f32;
        }
    }
    (best / normalizer, best_offset)
}

// ---------------------------------------------------------------------------
// hashing a file
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// bars
// ---------------------------------------------------------------------------

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
    let out = Command::new("ffmpeg")
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
    let out = Command::new("ffmpeg")
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

#[derive(Debug, Clone)]
pub struct VideoSig {
    pub meta: Meta,
    /// Full image signatures at `POSITIONS` fractions of the duration; may
    /// be shorter when the stream ended before the estimate.
    pub positional: Vec<Signature>,
    pub tmk: Tmk,
    pub frames: u32,
    /// Bars found around the picture (black, or since Phase 11 of a flat
    /// colour); every frame was cropped to the picture inside them before
    /// hashing.
    pub bars: Option<Bars>,
}

/// Frame indices (at FPS) for the positional tier.
pub fn position_indices(duration: f64) -> Vec<u32> {
    let total = (duration * FPS as f64).round().max(1.0);
    (0..POSITIONS)
        .map(|i| (((i as f64 + 0.5) / POSITIONS as f64) * total).floor() as u32)
        .collect()
}

/// Stream and hash one file: the bar pass, then the frame pass.
pub fn hash_video(path: &Path, meta: &Meta) -> Result<VideoSig, VideoError> {
    // black bars first (cropdetect, measured in Phase 7); bars of any
    // other colour when there are none
    let bars = detect_bars(path, meta).or_else(|| detect_flat_bars(path, meta));
    hash_video_inside(path, meta, bars)
}

/// The frame pass alone, cropping to `bars` when given.
pub fn hash_video_inside(
    path: &Path,
    meta: &Meta,
    bars: Option<Bars>,
) -> Result<VideoSig, VideoError> {
    // the picture's own size, for the signature's aspect
    let (pic_w, pic_h) = bars.map_or((meta.width, meta.height), |b| (b.width, b.height));
    let crop = bars.map_or(String::new(), |b| {
        format!("crop={}:{}:{}:{},", b.width, b.height, b.x, b.y)
    });
    let mut child = Command::new("ffmpeg")
        .args([
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "error",
            "-threads",
            &FFMPEG_THREADS.to_string(),
        ])
        .arg("-i")
        .arg(path)
        .args([
            "-an",
            "-sn",
            "-dn",
            "-vf",
            &format!("fps={FPS},{crop}scale={FRAME}:{FRAME}:flags=area,format=gray"),
            "-f",
            "rawvideo",
            "-pix_fmt",
            "gray",
            "pipe:1",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(VideoError::Spawn)?;
    let mut out = child.stdout.take().expect("piped stdout");

    let want = position_indices(meta.duration);
    let mut positional: Vec<Signature> = Vec::with_capacity(POSITIONS);
    let mut builder = TmkBuilder::new();
    let mut raw = vec![0u8; FRAME * FRAME];
    let mut grid = vec![0.0f32; FRAME * FRAME];
    let mut t: u32 = 0;
    let mut next_pos = 0usize;
    loop {
        match read_exact_or_eof(&mut out, &mut raw) {
            Ok(true) => {}
            Ok(false) => break,
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(VideoError::Io(e));
            }
        }
        for (g, b) in grid.iter_mut().zip(&raw) {
            *g = *b as f32;
        }
        // PDQ on the 288 frame: Jarosz window ⌈288/128⌉ = 3, then 64×64
        let buf = pdq::downsample(&pdq::Luma {
            data: &grid,
            width: FRAME,
            height: FRAME,
        });
        let band = pdq::dct16(&buf);
        builder.ingest(t, &band);
        if next_pos < want.len() && t == want[next_pos] {
            let q = pdq::quality(&buf);
            let mut hashes = [[0u64; 4]; perceptual::VARIANTS];
            for (k, v) in VARIANT_ORDER.iter().enumerate() {
                let bv = if *v == Variant::Identity {
                    band
                } else {
                    pdq::band_variant(&band, *v)
                };
                hashes[k] = pdq::bits(&bv);
            }
            let p = pdq::Pdq { hashes, quality: q };
            positional.push(perceptual::signature_from_planes(&grid, &p, pic_w, pic_h));
            next_pos += 1;
        }
        t += 1;
    }
    let _ = child.wait();
    if t == 0 {
        return Err(VideoError::NoFrames);
    }
    Ok(VideoSig {
        meta: meta.clone(),
        positional,
        tmk: builder.finish(),
        frames: t,
        bars,
    })
}

/// Fill `buf` completely; `Ok(false)` on a clean EOF before any byte.
fn read_exact_or_eof(r: &mut impl Read, buf: &mut [u8]) -> io::Result<bool> {
    let mut got = 0;
    while got < buf.len() {
        let n = r.read(&mut buf[got..])?;
        if n == 0 {
            return if got == 0 {
                Ok(false)
            } else {
                Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "partial frame",
                ))
            };
        }
        got += n;
    }
    Ok(true)
}

// ---------------------------------------------------------------------------
// comparing two videos
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VideoRelation {
    /// Positional medians over the shared positions on the modal
    /// orientation; `None` when fewer than `POSITIONS_MIN` positions exist.
    pub pos: Option<(u32, u32, u32)>,
    pub variant: Variant,
    pub level1: f32,
    pub level2: f32,
    /// Best time offset from level 2, seconds.
    pub offset: f32,
}

impl VideoRelation {
    /// Grouping rule for `find`/`milk`: positional agreement on the two
    /// 64-bit hashes, or the TMK two-phase rule.
    pub fn similar(&self) -> bool {
        self.positional_ok() || self.tmk_ok()
    }

    pub fn positional_ok(&self) -> bool {
        matches!(self.pos, Some((d, p, _)) if d <= DHASH_MAX && p <= PHASH_MAX)
    }

    pub fn tmk_ok(&self) -> bool {
        self.level1 >= LEVEL1_MIN && self.level2 >= LEVEL2_MIN
    }

    /// `purge` rule: TMK level 2 at the stricter bar, or positional
    /// agreement on all three hashes including PDQ. The positional test
    /// needs the lengths to line up, the TMK one does not.
    pub fn confirmed(&self) -> bool {
        (self.level1 >= LEVEL1_MIN && self.level2 >= LEVEL2_CONFIRM)
            || matches!(self.pos, Some((d, p, q)) if d <= DHASH_MAX && p <= PHASH_MAX && q <= PDQ_MAX)
    }

    /// What the member is, in words: `re-encoded`, `scaled 768→480`,
    /// `trimmed 0:30 of 1:20`, `pillarboxed`, `letterboxed`, `mirrored`,
    /// `similar, unconfirmed`; two words at most. [CLAUDE]
    pub fn words(&self, keeper: &VideoSig, member: &VideoSig) -> String {
        if !self.confirmed() {
            return "similar, unconfirmed".to_string();
        }
        let mut parts: Vec<String> = Vec::new();
        // bars one side has and the other has not
        match (member.bars, keeper.bars) {
            (Some(b), None) => parts.push(
                if b.width < member.meta.width {
                    "pillarboxed"
                } else {
                    "letterboxed"
                }
                .to_string(),
            ),
            (None, Some(_)) => parts.push("bars removed".to_string()),
            _ => {}
        }
        // orientation, from the sampled frames
        if self.positional_ok() {
            let o = match self.variant {
                Variant::Identity => "",
                Variant::Rotate90 => "rotated 90°",
                Variant::Rotate180 => "rotated 180°",
                Variant::Rotate270 => "rotated 270°",
                Variant::FlipH => "mirrored",
                Variant::FlipV => "flipped",
                Variant::Transpose | Variant::AntiTranspose => "rotated and mirrored",
            };
            if !o.is_empty() {
                parts.push(o.to_string());
            }
        }
        // length
        let (kd, md) = (keeper.meta.duration, member.meta.duration);
        if kd > 0.0 && (md - kd).abs() / kd > 0.02 {
            parts.push(if md < kd {
                format!(
                    "trimmed {} of {}",
                    crate::util::fmt_clock(md),
                    crate::util::fmt_clock(kd)
                )
            } else {
                format!(
                    "longer, {} of {}",
                    crate::util::fmt_clock(kd),
                    crate::util::fmt_clock(md)
                )
            });
        }
        // picture size, inside any bars
        let pic = |s: &VideoSig| s.bars.map_or(s.meta.width, |b| b.width);
        let (kw, mw) = (pic(keeper), pic(member));
        if kw != mw && (kw as f32 - mw as f32).abs() / kw.max(1) as f32 > 0.03 {
            parts.push(format!("scaled {kw}\u{2192}{mw}"));
        }
        if parts.is_empty() {
            parts.push("re-encoded".to_string());
        }
        parts.truncate(2);
        parts.join(", ")
    }

    /// The evidence, for the dim trailing column.
    pub fn numbers(&self) -> String {
        let tmk = if self.level1 >= LEVEL1_MIN {
            format!("tmk {:.2}", self.level2)
        } else {
            "tmk    -".to_string()
        };
        match self.pos {
            Some((d, p, q)) => format!("d {d:>2} p {p:>2}  q {q:>3}  {tmk}"),
            None => format!("frames     -  {tmk}"),
        }
    }

    pub fn unconfirmed_reason(&self) -> String {
        let tmk = format!("TMK {:.2} under {LEVEL2_CONFIRM:.1}", self.level2);
        match self.pos {
            Some((d, p, q)) if d <= DHASH_MAX && p <= PHASH_MAX && q > PDQ_MAX => {
                format!("frames q {q} over {PDQ_MAX}; {tmk}")
            }
            _ => tmk,
        }
    }
}

fn hamming_pair(k: &Signature, m: &Signature, vi: usize) -> (u32, u32, u32) {
    let d1 = perceptual::hamming(k.dhash[0], m.dhash[vi]);
    let p1 = perceptual::hamming(k.phash[0], m.phash[vi]);
    let q1 = pdq::distance(&k.pdq[0], &m.pdq[vi]);
    let d2 = perceptual::hamming(m.dhash[0], k.dhash[vi]);
    let p2 = perceptual::hamming(m.phash[0], k.phash[vi]);
    let q2 = pdq::distance(&m.pdq[0], &k.pdq[vi]);
    if d1 + p1 <= d2 + p2 {
        (d1, p1, q1)
    } else {
        (d2, p2, q2)
    }
}

fn median(v: &mut [u32]) -> u32 {
    v.sort_unstable();
    v[v.len() / 2]
}

pub fn relate(a: &VideoSig, b: &VideoSig) -> VideoRelation {
    let n = a.positional.len().min(b.positional.len());
    let mut variant = Variant::Identity;
    let mut pos = None;
    if n >= POSITIONS_MIN {
        // modal best orientation over positions
        let mut votes = [0u32; perceptual::VARIANTS];
        for i in 0..n {
            let mut best = (u32::MAX, 0usize);
            for vi in 0..perceptual::VARIANTS {
                let (d, p, _) = hamming_pair(&a.positional[i], &b.positional[i], vi);
                if d + p < best.0 {
                    best = (d + p, vi);
                }
            }
            votes[best.1] += 1;
        }
        let vi = votes
            .iter()
            .enumerate()
            .max_by_key(|(_, v)| **v)
            .map(|(i, _)| i)
            .unwrap_or(0);
        variant = VARIANT_ORDER[vi];
        let mut ds = Vec::with_capacity(n);
        let mut ps = Vec::with_capacity(n);
        let mut qs = Vec::with_capacity(n);
        for i in 0..n {
            let (d, p, q) = hamming_pair(&a.positional[i], &b.positional[i], vi);
            ds.push(d);
            ps.push(p);
            qs.push(q);
        }
        pos = Some((median(&mut ds), median(&mut ps), median(&mut qs)));
    }
    let l1 = level1(&a.tmk, &b.tmk);
    let (l2, offset) = if l1 >= LEVEL1_MIN {
        level2(&a.tmk, &b.tmk)
    } else {
        (0.0, 0.0)
    };
    VideoRelation {
        pos,
        variant,
        level1: l1,
        level2: l2,
        offset,
    }
}

// ---------------------------------------------------------------------------
// the pass
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Clone, Copy)]
pub struct VideoStats {
    pub probed: u64,
    pub unreadable: u64,
    pub hashed: u64,
    pub frames: u64,
    pub pairs: u64,
    pub grouped: u64,
}

/// Probe and hash every path, `concurrency()` at a time.
pub fn hash_all<T: Send + Sync>(
    items: Vec<(T, std::path::PathBuf)>,
    progress: &dyn Progress,
) -> (Vec<(T, VideoSig)>, VideoStats) {
    let total = items.len() as u64;
    let done = AtomicU64::new(0);
    let frames = AtomicU64::new(0);
    progress.tick(Stage::Frames, 0, total);
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(concurrency())
        .build();
    let run = |items: Vec<(T, std::path::PathBuf)>| -> Vec<(T, Result<VideoSig, VideoError>)> {
        items
            .into_par_iter()
            .map(|(tag, path)| {
                let r = probe(&path).and_then(|m| hash_video(&path, &m));
                if let Ok(s) = &r {
                    frames.fetch_add(s.frames as u64, Ordering::Relaxed);
                }
                let d = done.fetch_add(1, Ordering::Relaxed) + 1;
                progress.tick(Stage::Frames, d, total);
                (tag, r)
            })
            .collect()
    };
    let results = match pool {
        Ok(pool) => pool.install(|| run(items)),
        Err(_) => run(items),
    };
    let mut stats = VideoStats {
        probed: total,
        frames: frames.load(Ordering::Relaxed),
        ..Default::default()
    };
    let mut out = Vec::with_capacity(results.len());
    for (tag, r) in results {
        match r {
            Ok(s) => {
                stats.hashed += 1;
                out.push((tag, s));
            }
            Err(_) => stats.unreadable += 1,
        }
    }
    (out, stats)
}

#[cfg(test)]
mod tests {

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

    use super::*;

    fn h(seed: u32, k: usize, salt: u32) -> f32 {
        // deterministic pseudo-random in [−1, 1)
        let mut x = (seed as u64) << 40 ^ (k as u64) << 8 ^ salt as u64;
        x ^= x >> 33;
        x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
        x ^= x >> 33;
        ((x >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0) as f32
    }

    /// Synthetic per-frame feature: a per-seed random base vector plus, per
    /// dimension, a sinusoid of random frequency and phase, so the signal
    /// decorrelates over a few hundred frames and distinct seeds are near
    /// orthogonal.
    fn feature(seed: u32, t: u32) -> [f32; DIM] {
        let mut f = [0.0f32; DIM];
        for (k, v) in f.iter_mut().enumerate() {
            let base = h(seed, k, 1);
            let omega = 0.02 + 0.18 * (h(seed, k, 2) * 0.5 + 0.5);
            let phi = std::f32::consts::PI * h(seed, k, 3);
            // a steady component (what a frame mostly looks like) plus variation
            *v = 0.3 * base + 0.6 * (omega * t as f32 + phi).sin();
        }
        f
    }

    fn descriptor(seed: u32, start: u32, frames: u32) -> Tmk {
        let mut b = TmkBuilder::new();
        for t in 0..frames {
            b.ingest(t, &feature(seed, start + t));
        }
        b.finish()
    }

    #[test]
    fn cropdetect_lines_become_bars_or_nothing() {
        // the last line wins (reset=0 accumulates); 32 px pillars of 480
        let log = "[Parsed_cropdetect_0 @ 0x1] x1:40 x2:439 y1:0 y2:719 w:400 h:720 x:40 y:0 pts:0 t:0.000000 crop=400:720:40:0\n\
                   [Parsed_cropdetect_0 @ 0x1] x1:32 x2:447 y1:0 y2:719 w:416 h:720 x:32 y:0 pts:512 t:1.000000 crop=416:720:32:0\n";
        assert_eq!(
            parse_cropdetect(log, 480, 720),
            Some(Bars {
                width: 416,
                height: 720,
                x: 32,
                y: 0
            })
        );
        // the whole frame: no bars
        assert_eq!(parse_cropdetect("crop=720:1280:0:0\n", 720, 1280), None);
        // a 2 px edge is not a bar (under BAR_MIN_SHARE)
        assert_eq!(parse_cropdetect("crop=1916:1080:2:0\n", 1920, 1080), None);
        // a dark video: the box keeps under a quarter of a side
        assert_eq!(parse_cropdetect("crop=200:1080:860:0\n", 1920, 1080), None);
        // nonsense and nothing
        assert_eq!(parse_cropdetect("crop=2000:1080:0:0\n", 1920, 1080), None);
        assert_eq!(parse_cropdetect("no frames here\n", 1920, 1080), None);
        // letterbox
        assert_eq!(
            parse_cropdetect("crop=1920:816:0:132\n", 1920, 1080),
            Some(Bars {
                width: 1920,
                height: 816,
                x: 0,
                y: 132
            })
        );
    }

    #[test]
    fn position_indices_are_inside_and_increasing() {
        let idx = position_indices(10.0); // 150 frames
        assert_eq!(idx.len(), POSITIONS);
        assert!(idx[0] >= 6 && *idx.last().unwrap() < 150, "{idx:?}");
        assert!(idx.windows(2).all(|w| w[0] < w[1]));
        let short = position_indices(0.3); // 4–5 frames: indices repeat, stay in range
        assert!(short.iter().all(|i| *i < 5));
    }

    #[test]
    fn parse_rates() {
        assert!((parse_rate("30000/1001") - 29.97).abs() < 0.01);
        assert_eq!(parse_rate("25"), 25.0);
        assert_eq!(parse_rate("0/0"), 0.0);
    }

    #[test]
    fn identical_descriptors_score_one() {
        let a = descriptor(1, 0, 900);
        let (l2, off) = level2(&a, &a);
        assert!((level1(&a, &a) - 1.0).abs() < 1e-4);
        assert!((l2 - 1.0).abs() < 1e-3, "{l2}");
        assert_eq!(off, 0.0);
    }

    #[test]
    fn time_shifted_copies_keep_a_high_level2() {
        // the same signal, copies starting 45, 150 and 600 frames later
        let a = descriptor(2, 0, 1200);
        for shift in [45u32, 150, 600] {
            let b = descriptor(2, shift, 1200);
            let (l2, _) = level2(&a, &b);
            assert!(l2 > 0.9, "shift {shift}: {l2}");
            assert!(level1(&a, &b) > 0.9);
        }
        // the per-coefficient normalisation the reference applies makes the
        // recovered offset approximate; it is not asserted and not displayed
    }

    #[test]
    fn different_signals_score_low() {
        let a = descriptor(3, 0, 900);
        let b = descriptor(4, 0, 900);
        let (l2, _) = level2(&a, &b);
        assert!(l2 < 0.3, "{l2}");
        assert!(level1(&a, &b) < 0.3);
    }

    #[test]
    fn confirmation_rules() {
        let base = VideoRelation {
            pos: None,
            variant: Variant::Identity,
            level1: 0.9,
            level2: 0.95,
            offset: 0.0,
        };
        assert!(base.similar() && base.confirmed());
        let grouped_only = VideoRelation {
            level2: 0.85,
            ..base
        };
        assert!(grouped_only.similar() && !grouped_only.confirmed());
        assert_eq!(grouped_only.unconfirmed_reason(), "TMK 0.85 under 0.9");
        let gated = VideoRelation {
            level1: 0.3,
            level2: 0.0,
            ..base
        };
        assert!(!gated.similar());
        let frames_ok = VideoRelation {
            pos: Some((2, 3, 10)),
            level1: -0.5,
            level2: 0.0,
            ..base
        };
        assert!(frames_ok.similar() && frames_ok.confirmed());
        let frames_q_high = VideoRelation {
            pos: Some((2, 3, 40)),
            ..frames_ok
        };
        assert!(frames_q_high.similar() && !frames_q_high.confirmed());
        assert_eq!(
            frames_q_high.unconfirmed_reason(),
            "frames q 40 over 31; TMK 0.00 under 0.9"
        );
        let frames_far = VideoRelation {
            pos: Some((20, 30, 120)),
            ..frames_ok
        };
        assert!(!frames_far.similar());
    }

    #[test]
    fn normalizer_matches_reference_table() {
        // c_0 + 2·Σ c_j over Meta's table; K(0) of identical descriptors equals it
        let n: f32 = COEFFS[0] + 2.0 * COEFFS[1..].iter().sum::<f32>();
        assert!((n - 1.9292).abs() < 1e-3, "{n}");
    }
}

// ---------------------------------------------------------------------------
// grouping
// ---------------------------------------------------------------------------

/// One video entering the pass (same shape as `near::Rep`).
#[derive(Debug, Clone)]
pub struct VRep {
    pub cand: crate::walk::Candidate,
    pub exact_group: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct VHashed {
    pub rep: VRep,
    pub sig: VideoSig,
}

#[derive(Debug, Clone)]
pub struct VideoMember {
    pub hashed: VHashed,
    /// Relation to the keeper; `None` for the keeper.
    pub relation: Option<VideoRelation>,
}

#[derive(Debug, Clone)]
pub struct VideoGroup {
    pub members: Vec<VideoMember>,
}

impl VideoGroup {
    /// Make member `idx` the keeper and relate the others to it. [CLAUDE]
    pub fn set_keeper(&mut self, idx: usize) {
        if idx == 0 || idx >= self.members.len() {
            return;
        }
        let chosen = self.members.remove(idx);
        self.members.insert(0, chosen);
        self.members[0].relation = None;
        let keeper = self.members[0].hashed.sig.clone();
        for m in self.members.iter_mut().skip(1) {
            m.relation = Some(relate(&keeper, &m.hashed.sig));
        }
    }

    /// Bytes `milk`/`purge` act on: confirmed members only, as for near
    /// groups. (Through Phase 8 this summed every member, so a trim the
    /// rule leaves in place was counted as reclaimable.)
    pub fn reclaimable(&self) -> u64 {
        self.members
            .iter()
            .skip(1)
            .filter(|m| m.relation.is_some_and(|r| r.confirmed()))
            .map(|m| m.hashed.rep.cand.size)
            .sum()
    }
}

pub const VIDEO_POLICY_LABEL: &str = "keep: longest, then most pixels";

/// Pixels of the picture itself: inside the bars when there are any, so a
/// padded export never outranks the picture it was padded from.
fn picture_pixels(v: &VHashed) -> u64 {
    match v.sig.bars {
        Some(b) => b.width as u64 * b.height as u64,
        None => v.sig.meta.width as u64 * v.sig.meta.height as u64,
    }
}

/// Keeper order: longest duration (a trim is the derivative), then most
/// picture pixels, then the file without bars (the padding is the
/// derivative), then highest bitrate, then the one not named or filed as
/// a copy (`keeper::derived`), then oldest, then path.
pub fn keeper_order(a: &VHashed, b: &VHashed) -> std::cmp::Ordering {
    let da = (a.sig.meta.duration * 10.0).round() as i64;
    let db = (b.sig.meta.duration * 10.0).round() as i64;
    let pa = picture_pixels(a);
    let pb = picture_pixels(b);
    let bra = if a.sig.meta.duration > 0.0 {
        a.rep.cand.size as f64 / a.sig.meta.duration
    } else {
        0.0
    };
    let brb = if b.sig.meta.duration > 0.0 {
        b.rep.cand.size as f64 / b.sig.meta.duration
    } else {
        0.0
    };
    db.cmp(&da)
        .then_with(|| pb.cmp(&pa))
        .then_with(|| a.sig.bars.is_some().cmp(&b.sig.bars.is_some()))
        .then_with(|| brb.partial_cmp(&bra).unwrap_or(std::cmp::Ordering::Equal))
        .then_with(|| {
            crate::keeper::derived(&a.rep.cand.path).cmp(&crate::keeper::derived(&b.rep.cand.path))
        })
        .then_with(|| a.rep.cand.mtime.cmp(&b.rep.cand.mtime))
        .then_with(|| a.rep.cand.path.cmp(&b.rep.cand.path))
}

/// All pairs (level 1 gates level 2), union–find, keeper order.
pub fn group(
    hashed: Vec<VHashed>,
    stats: &mut VideoStats,
    progress: &dyn Progress,
) -> Vec<VideoGroup> {
    let n = hashed.len();
    progress.tick(Stage::Match, 0, n as u64);
    let done = AtomicU64::new(0);
    let pairs: Vec<(usize, usize)> = (0..n)
        .into_par_iter()
        .flat_map_iter(|i| {
            let mut v = Vec::new();
            for j in i + 1..n {
                if relate(&hashed[i].sig, &hashed[j].sig).similar() {
                    v.push((i, j));
                }
            }
            let d = done.fetch_add(1, Ordering::Relaxed) + 1;
            progress.tick(Stage::Match, d, n as u64);
            v.into_iter()
        })
        .collect();
    stats.pairs = pairs.len() as u64;

    let mut uf = crate::near::UnionFind::new(n);
    for (a, b) in &pairs {
        uf.union(*a, *b);
    }
    let mut buckets: std::collections::HashMap<usize, Vec<usize>> =
        std::collections::HashMap::new();
    for i in 0..n {
        let r = uf.find(i);
        buckets.entry(r).or_default().push(i);
    }
    let mut slots: Vec<Option<VHashed>> = hashed.into_iter().map(Some).collect();
    let mut groups: Vec<VideoGroup> = Vec::new();
    for (_, mut idxs) in buckets {
        if idxs.len() < 2 {
            continue;
        }
        let mut members: Vec<VHashed> = idxs.drain(..).map(|i| slots[i].take().unwrap()).collect();
        members.sort_by(keeper_order);
        let keeper = members[0].sig.clone();
        let members: Vec<VideoMember> = members
            .into_iter()
            .enumerate()
            .map(|(i, h)| {
                let relation = if i == 0 {
                    None
                } else {
                    Some(relate(&keeper, &h.sig))
                };
                VideoMember {
                    hashed: h,
                    relation,
                }
            })
            .collect();
        stats.grouped += members.len() as u64;
        groups.push(VideoGroup { members });
    }
    groups.sort_by(|a, b| {
        b.reclaimable().cmp(&a.reclaimable()).then_with(|| {
            a.members[0]
                .hashed
                .rep
                .cand
                .path
                .cmp(&b.members[0].hashed.rep.cand.path)
        })
    });
    groups
}
