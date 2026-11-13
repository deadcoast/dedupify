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
//!
//! Split (Phase 12, TECH-06): `probe` reads a file's stream, `bars` finds
//! the picture inside its bars, `tmk` builds and scores the descriptor,
//! `group` relates two videos and groups them. Here: hashing one file and
//! the pass over all of them.

use std::io::{self, Read};
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::exact::{Progress, Stage};
use crate::pdq::{self};
use crate::perceptual::{self, Signature, Variant, GRID, VARIANT_ORDER};

mod bars;
mod group;
mod probe;
mod tmk;

pub use bars::{
    band_detail, black_band_detail, detect_any_bars, detect_any_bars_clocked, detect_bars,
    detect_bars_clocked, detect_flat_bars, detect_flat_bars_clocked, flat_bars, parse_cropdetect,
    Bars, BAR_KEEP_MIN, BAR_LIMIT, BAR_MIN_SHARE, BLACK_DETAIL_MAX, FLAT_FRAMES, FLAT_PICTURE_MIN,
    FLAT_RATIO, FLAT_START, FLAT_WIDTH,
};
pub use group::{
    group, keeper_order, relate, VHashed, VRep, VideoGroup, VideoMember, VideoRelation,
    VIDEO_POLICY_LABEL,
};
pub use probe::{probe, tools_present, Meta, VideoError};
pub use tmk::{level1, level2, Tmk, TmkBuilder, COEFFS, DIM, PERIODS};

/// Frames per second the stream is resampled to (TMK default).
pub const FPS: u32 = 15;

/// Side of the grey frame requested from ffmpeg; equals the image grid so
/// the same dHash/pHash code runs on frames.
pub const FRAME: usize = GRID;

/// Positions sampled for the positional tier.
pub const POSITIONS: usize = 12;

/// Fewest positions both videos must share for the positional tier to speak.
pub const POSITIONS_MIN: usize = 8;

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

/// ffmpeg threads per process, at least; `ffmpeg_threads` shares the
/// machine's threads among the `concurrency()` processes. Decoding is
/// exact whatever the thread count (frame threading changes when a frame
/// is decoded, never its pixels).
pub const FFMPEG_THREADS: usize = 2;

/// Threads per ffmpeg process: the machine's threads over the processes
/// running at once, at least `FFMPEG_THREADS` (2 on 2 to 5 threads, 4 on
/// 16). (Phase 14, CORE-16)
pub fn ffmpeg_threads() -> usize {
    let n = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2);
    (n / concurrency()).max(FFMPEG_THREADS)
}

/// A file at least this long is decoded by the graphics hardware when
/// ffmpeg finds a decoder there (`-hwaccel auto`), and again in software
/// if that run fails. H.264, HEVC, VP9 and AV1 decoding is exact by their
/// standards, so the frames are the same; shorter files are not worth
/// the hardware's start-up. `DEDUPE_HWACCEL=0` turns it off. (Phase 14,
/// CORE-16)
pub const HWACCEL_MIN_SECS: f64 = 600.0;

/// Whether the frame pass may ask for hardware decoding.
pub fn hwaccel_enabled() -> bool {
    std::env::var("DEDUPE_HWACCEL").map_or(true, |v| v != "0")
}

pub fn concurrency() -> usize {
    (std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2)
        / 2)
    .clamp(1, 4)
}

// ---------------------------------------------------------------------------
// probing

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
    // black bars (cropdetect, Phase 7) unless their band carries detail,
    // bars of any other colour otherwise (Phases 11 and 15)
    // (a pass that stalled ends the file here: the frame pass would too)
    let bars = detect_any_bars_clocked(path, meta)?;
    hash_video_inside(path, meta, bars)
}

/// The frame pass alone, cropping to `bars` when given: by the graphics
/// hardware for a long file when it can (`HWACCEL_MIN_SECS`), in software
/// otherwise or when that fails.
pub fn hash_video_inside(
    path: &Path,
    meta: &Meta,
    bars: Option<Bars>,
) -> Result<VideoSig, VideoError> {
    if meta.duration >= HWACCEL_MIN_SECS && hwaccel_enabled() {
        if let Ok(s) = frame_pass(path, meta, bars, true) {
            return Ok(s);
        }
    }
    frame_pass(path, meta, bars, false)
}

/// The frame pass, `hw` asking ffmpeg for hardware decoding. Public for
/// the bench, which compares the two.
#[doc(hidden)]
pub fn frame_pass(
    path: &Path,
    meta: &Meta,
    bars: Option<Bars>,
    hw: bool,
) -> Result<VideoSig, VideoError> {
    // the picture's own size, for the signature's aspect
    let (pic_w, pic_h) = bars.map_or((meta.width, meta.height), |b| (b.width, b.height));
    let crop = bars.map_or(String::new(), |b| {
        format!("crop={}:{}:{}:{},", b.width, b.height, b.x, b.y)
    });
    let mut child = crate::tools::ffmpeg()
        .args([
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "error",
            "-threads",
            &ffmpeg_threads().to_string(),
        ])
        .args(if hw {
            &["-hwaccel", "auto"][..]
        } else {
            &[][..]
        })
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
    let mut out = child
        .stdout
        .take()
        .expect("invariant: the child was spawned with stdout piped");
    // a clock on the pipe: no frame for `tools::stall()` and ffmpeg is
    // stopped (Phase 13, CORE-11)
    let watch = crate::tools::Watch::new(child);

    let want = position_indices(meta.duration);
    let mut positional: Vec<Signature> = Vec::with_capacity(POSITIONS);
    let mut builder = TmkBuilder::new();
    let mut raw = vec![0u8; FRAME * FRAME];
    let mut grid = vec![0.0f32; FRAME * FRAME];
    let mut t: u32 = 0;
    let mut next_pos = 0usize;
    loop {
        match read_exact_or_eof(&mut out, &mut raw) {
            Ok(true) => watch.touch(),
            Ok(false) => break,
            Err(e) => {
                let stalled = watch.stalled();
                watch.kill();
                let _ = watch.finish();
                return Err(if stalled {
                    VideoError::Stalled
                } else {
                    VideoError::Io(e)
                });
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
    let _ = watch.finish();
    // stopped at a frame's edge reads as the end of the file: it is not
    if watch.stalled() {
        return Err(VideoError::Stalled);
    }
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

#[derive(Debug, Default, Clone)]
pub struct VideoStats {
    pub probed: u64,
    pub unreadable: u64,
    /// Files whose ffmpeg or ffprobe sent nothing for `tools::stall()` and
    /// was stopped (Phase 13, CORE-11); counted in `unreadable` too.
    pub stalled: Vec<std::path::PathBuf>,
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
    let workers = concurrency();
    // probe everything, then hash the longest first: a two-hour file that
    // starts last runs alone at the end while the other workers wait
    // (Phase 14, CORE-16)
    let paths: Vec<&Path> = items.iter().map(|(_, p)| p.as_path()).collect();
    let metas = queue(&paths, workers, |p| probe(p));
    let mut order: Vec<usize> = (0..items.len()).collect();
    let length = |i: &usize| metas[*i].as_ref().map_or(0.0, |m| m.duration);
    order.sort_by(|a, b| length(b).total_cmp(&length(a)));
    let sigs = queue(&order, workers, |&i| {
        let r = match &metas[i] {
            Ok(m) => hash_video(paths[i], m),
            Err(e) => Err(e.again()),
        };
        if let Ok(s) = &r {
            frames.fetch_add(s.frames as u64, Ordering::Relaxed);
        }
        let d = done.fetch_add(1, Ordering::Relaxed) + 1;
        progress.tick(Stage::Frames, d, total);
        r
    });
    let mut by_item: Vec<Option<Result<VideoSig, VideoError>>> =
        (0..items.len()).map(|_| None).collect();
    for (i, r) in order.into_iter().zip(sigs) {
        by_item[i] = Some(r);
    }
    let results: Vec<(T, std::path::PathBuf, Result<VideoSig, VideoError>)> = items
        .into_iter()
        .zip(by_item)
        .map(|((tag, path), r)| (tag, path, r.expect("invariant: every item was hashed once")))
        .collect();
    let mut stats = VideoStats {
        probed: total,
        frames: frames.load(Ordering::Relaxed),
        ..Default::default()
    };
    let mut out = Vec::with_capacity(results.len());
    for (tag, path, r) in results {
        match r {
            Ok(s) => {
                stats.hashed += 1;
                out.push((tag, s));
            }
            Err(VideoError::Stalled) => {
                stats.unreadable += 1;
                stats.stalled.push(path);
            }
            Err(_) => stats.unreadable += 1,
        }
    }
    (out, stats)
}

/// `f` over `items` on `workers` threads, each taking the next item as it
/// finishes one (in order: a list sorted longest first is worked longest
/// first); the results in the items' order.
fn queue<I: Sync, O: Send>(items: &[I], workers: usize, f: impl Fn(&I) -> O + Sync) -> Vec<O> {
    let next = AtomicU64::new(0);
    let done: std::sync::Mutex<Vec<(usize, O)>> =
        std::sync::Mutex::new(Vec::with_capacity(items.len()));
    std::thread::scope(|s| {
        for _ in 0..workers.clamp(1, items.len().max(1)) {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed) as usize;
                let Some(item) = items.get(i) else { break };
                let o = f(item);
                done.lock().unwrap_or_else(|e| e.into_inner()).push((i, o));
            });
        }
    });
    let mut done = done.into_inner().unwrap_or_else(|e| e.into_inner());
    done.sort_by_key(|(i, _)| *i);
    done.into_iter().map(|(_, o)| o).collect()
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn the_queue_keeps_the_order_and_takes_every_item() {
        let items: Vec<u64> = (0..103).collect();
        let out = queue(&items, 4, |x| x * 2);
        assert_eq!(out, items.iter().map(|x| x * 2).collect::<Vec<_>>());
        assert!(queue(&[] as &[u64], 4, |x| *x).is_empty());
    }

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
