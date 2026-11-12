//! In-tree benchmark (Phase 12, TECH-05): the stages of a scan timed on a
//! fixed corpus, so each phase's record can say what it cost.
//!
//! ```text
//! cargo test --release --test bench -- --ignored --nocapture
//! DEDUPE_BENCH_DIR=<folder> …   keeps the corpus there between runs
//! ```
//!
//! The corpus is generated, the same bytes every time (seeded):
//!
//! - **files**: 1,200 files of 8–407 KiB (400 sizes), 200 of them byte
//!   copies of another, 100 the size of another with other bytes;
//! - **images**: 120 procedural photographs at 1024 × 768 (JPEG 90), each
//!   with a half-size copy, and 40 of them re-encoded at quality 60;
//! - **videos** (when ffmpeg is found): six 10-second test patterns at
//!   640 × 360, each with a re-encode at a lower bitrate.
//!
//! Each stage runs through the library as `find` runs it, and the clock
//! stops at each stage's end: SCAN, SIZE, SAMPLE, HASH (the exact tier),
//! DECODE and MATCH (images), FRAMES and the video MATCH. Nothing here is
//! a unit test; it prints a record for VERIFICATION.md.
//!
//! Real files join the corpus from `DEDUPE_PHOTO_DIRS` and
//! `DEDUPE_VIDEO_DIRS` (lists of folders, separated as PATH is; only read).
//! With the model installed, DECODE runs again with it (`DECODE+m`). The
//! videos' frame pass runs once more one file at a time in software and
//! with `-hwaccel auto` (`FRAMES sw`, `FRAMES hw`), and the signatures are
//! compared. Each
//! row carries the stage's peak memory: on Linux the peak is reset at the
//! stage's start, elsewhere it is the peak since the run began (the
//! header says which). (Phase 14, TECH-10)
//!
//! ```text
//! cargo test --release --test bench scale_run -- --ignored --nocapture
//! DEDUPE_SCALE_FILES=<n> (100000)   DEDUPE_SCALE_DIR=<folder> keeps it
//! ```
//!
//! The scale run: `n` files in folders of 100 — 8 % of them small
//! photographs (192 × 144, one in eight a half-size copy of another), the
//! rest 0.5–8 KiB of bytes, one in ten a byte copy — through SCAN, the
//! exact tier, DECODE and MATCH.

mod common;
use common::*;

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use dedupe::exact::{self, PipelineStats, Progress, Stage};
use dedupe::kind::Kind;
use dedupe::near::{self, Rep};
use dedupe::video::{self, VHashed, VRep, VideoSig};
use dedupe::walk::{self, Filter};

/// Records when each stage of the exact tier ends, and its peak memory.
struct Clock {
    t0: Instant,
    ends: Mutex<Vec<(Stage, Duration, Option<u64>)>>,
}

impl Progress for Clock {
    fn tick(&self, _: Stage, _: u64, _: u64) {}
    fn done(&self, stage: Stage, _: &PipelineStats) {
        self.ends
            .lock()
            .unwrap()
            .push((stage, self.t0.elapsed(), peak()));
        peak_reset();
    }
}

/// The process's peak resident memory in bytes: since the last
/// `peak_reset` on Linux, since the start elsewhere.
#[cfg(target_os = "linux")]
fn peak() -> Option<u64> {
    let s = std::fs::read_to_string("/proc/self/status").ok()?;
    let kb: u64 = s
        .lines()
        .find(|l| l.starts_with("VmHWM:"))?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()?;
    Some(kb * 1024)
}

#[cfg(all(unix, not(target_os = "linux")))]
fn peak() -> Option<u64> {
    let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
    // SAFETY: getrusage fills the struct it is given
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut ru) } != 0 {
        return None;
    }
    // bytes on macOS
    Some(ru.ru_maxrss as u64)
}

#[cfg(windows)]
fn peak() -> Option<u64> {
    use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows::Win32::System::Threading::GetCurrentProcess;
    let mut c = PROCESS_MEMORY_COUNTERS {
        cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        ..Default::default()
    };
    // SAFETY: the counters are sized as the call expects; the pseudo handle
    // needs no closing
    unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb) }.ok()?;
    Some(c.PeakWorkingSetSize as u64)
}

/// Starts a new peak (Linux: `clear_refs` 5 resets VmHWM to the current
/// size). `false` where the peak cannot be reset.
fn peak_reset() -> bool {
    #[cfg(target_os = "linux")]
    {
        std::fs::write("/proc/self/clear_refs", b"5").is_ok()
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

/// One line of the record.
struct Row {
    stage: String,
    what: String,
    secs: f64,
    peak: Option<u64>,
}

/// Times a stage: resets the peak, runs it, reads the peak.
fn stage<T>(rows: &mut Vec<Row>, name: &str, f: impl FnOnce() -> (T, String)) -> T {
    peak_reset();
    let t = Instant::now();
    let (out, what) = f();
    rows.push(Row {
        stage: name.into(),
        what,
        secs: t.elapsed().as_secs_f64(),
        peak: peak(),
    });
    out
}

fn print(rows: &[Row], header: &str) {
    let per = if peak_reset() {
        "peak memory per stage"
    } else {
        "peak memory since the start"
    };
    eprintln!();
    eprintln!("{header}   {per}");
    for r in rows {
        let mem = r
            .peak
            .map(|b| format!("{:>6.0} MB", b as f64 / 1e6))
            .unwrap_or_else(|| "     ? MB".into());
        eprintln!(" {:<8} {:>8.3} s  {mem}   {}", r.stage, r.secs, r.what);
    }
}

/// The folders listed in an environment variable, as PATH lists them.
fn dirs_in(var: &str) -> Vec<PathBuf> {
    std::env::var_os(var)
        .map(|v| std::env::split_paths(&v).filter(|p| p.is_dir()).collect())
        .unwrap_or_default()
}

/// SCAN of the corpus and any real folders, as one list.
fn scan_all(roots: &[PathBuf]) -> (Vec<walk::Candidate>, u64) {
    let filter = Filter { skip: &[], min: 0 };
    let mut all = Vec::new();
    let mut files = 0;
    for r in roots {
        let (c, s) = walk::scan(r, true, Kind::All, filter, &mut |_| {});
        files += s.files;
        all.extend(c);
    }
    (all, files)
}

/// The exact tier, a row per stage.
fn exact_rows(rows: &mut Vec<Row>, cands: Vec<walk::Candidate>) {
    peak_reset();
    let clock = Clock {
        t0: Instant::now(),
        ends: Mutex::new(Vec::new()),
    };
    let (groups, pipe) = exact::find_groups(cands, false, &clock);
    let mut last = Duration::ZERO;
    for (stage, end, mem) in clock.ends.lock().unwrap().iter() {
        let what = match stage {
            Stage::Size => format!("{} share a size", pipe.size_sharing),
            Stage::Sample => format!(
                "{} share a sample, {:.1} MB read",
                pipe.sample_sharing,
                pipe.bytes_sampled as f64 / 1e6
            ),
            Stage::Hash => format!(
                "{} confirmed, {:.0} MB read, {} groups",
                pipe.confirmed,
                pipe.bytes_hashed as f64 / 1e6,
                groups.len()
            ),
            _ => String::new(),
        };
        rows.push(Row {
            stage: stage.label().into(),
            what,
            secs: (*end - last).as_secs_f64(),
            peak: *mem,
        });
        last = *end;
    }
}

/// DECODE (and DECODE+m with the model) and MATCH over the images.
fn image_rows(rows: &mut Vec<Row>, cands: &[walk::Candidate], model: bool) {
    let reps = || -> Vec<Rep> {
        cands
            .iter()
            .filter(|c| dedupe::decode::decodable(&c.path))
            .map(|c| Rep {
                cand: c.clone(),
                exact_group: None,
            })
            .collect()
    };
    let n_img = reps().len();
    let (hashed, mut nstats) = stage(rows, "DECODE", || {
        let t = Instant::now();
        let out = near::hash_all(reps(), &exact::NoProgress, None);
        let rate = n_img as f64 / t.elapsed().as_secs_f64();
        (out, format!("{n_img} images, {rate:.1} a second"))
    });
    if model {
        if let Some(m) = dedupe::deep::installed().and_then(|p| dedupe::deep::Deep::load(&p).ok()) {
            stage(rows, "DECODE+m", || {
                let t = Instant::now();
                let (h, _) = near::hash_all(reps(), &exact::NoProgress, Some(&m));
                let rate = n_img as f64 / t.elapsed().as_secs_f64();
                let embedded = h.iter().filter(|x| x.deep.is_some()).count();
                (
                    (),
                    format!("{n_img} images, {embedded} embedded, {rate:.1} a second"),
                )
            });
        } else {
            rows.push(Row {
                stage: "DECODE+m".into(),
                what: "model not installed".into(),
                secs: 0.0,
                peak: None,
            });
        }
    }
    let groups = stage(rows, "MATCH", || {
        let g = near::group(hashed, &mut nstats, &exact::NoProgress);
        let what = format!(
            "{} near groups, {} fits, {} split from another",
            g.len(),
            nstats.fits,
            nstats.split
        );
        (g, what)
    });
    // the scale corpus plants one copy per eight images (`i…7` a half-size
    // copy of `i…6`): any other group is a finding
    if cands
        .iter()
        .any(|c| c.path.to_string_lossy().contains("i000006"))
    {
        let num = |m: &near::NearMember| -> u64 {
            let s = m
                .hashed
                .rep
                .cand
                .path
                .file_stem()
                .unwrap()
                .to_string_lossy()
                .to_string();
            s.trim_start_matches('i').parse().unwrap_or(u64::MAX)
        };
        // keeper first; a member acted on carries `*`, its relation's
        // numbers after it
        let mut other = Vec::new();
        for g in &groups {
            let mut n: Vec<u64> = g.members.iter().map(num).collect();
            n.sort();
            let planted = n.len() == 2 && n[0] % 8 == 6 && n[1] == n[0] + 1;
            if !planted {
                let line: Vec<String> = g
                    .members
                    .iter()
                    .map(|m| match m.relation {
                        None => format!("{}", num(m)),
                        Some(r) => format!(
                            "{}{} ({})",
                            num(m),
                            if r.confirmed() { "*" } else { "" },
                            r.numbers()
                        ),
                    })
                    .collect();
                other.push(line.join(" "));
            }
        }
        rows.push(Row {
            stage: "  check".into(),
            what: format!(
                "{} planted pairs grouped, {} other groups{}",
                groups.len() - other.len(),
                other.len(),
                if other.is_empty() {
                    String::new()
                } else {
                    format!(":\n            {}", other.join("\n            "))
                }
            ),
            secs: 0.0,
            peak: None,
        });
    }
}

/// `len` bytes from `seed`, the same every run.
fn bytes(seed: u64, len: usize) -> Vec<u8> {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect()
}

fn write(p: &Path, b: &[u8]) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, b).unwrap();
}

fn jpeg(img: &image::RgbImage, q: u8) -> Vec<u8> {
    let mut v = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut v, q)
        .encode_image(img)
        .unwrap();
    v
}

fn build(root: &Path) -> bool {
    let done = root.join(".complete");
    if done.exists() {
        return dedupe::video::tools_present() == (true, true) && root.join("videos").exists();
    }
    // files: 400 sizes from 8 to 407 KiB by a fixed rule, so most share a
    // size and the larger ones need more than their sample; 200 are byte
    // copies of an earlier file, 100 more are the size of an earlier file
    // with other bytes
    let len = |i: u64| 8 * 1024 + (i * 7919 % 400) as usize * 1024;
    for i in 0..1200u64 {
        let data = if i >= 1000 {
            bytes(i - 1000, len(i - 1000))
        } else if i >= 900 {
            bytes(i, len(i - 900))
        } else {
            bytes(i, len(i))
        };
        write(
            &root.join(format!("files/{:02}/f{i:05}.bin", i % 40)),
            &data,
        );
    }
    // images
    for s in 0..120u64 {
        let p = photo(s);
        write(&root.join(format!("images/p{s:03}.jpg")), &jpeg(&p, 90));
        write(
            &root.join(format!("images/half/p{s:03}.jpg")),
            &jpeg(&scale(&p, 0.5), 90),
        );
        if s % 3 == 0 {
            write(&root.join(format!("images/q60/p{s:03}.jpg")), &jpeg(&p, 60));
        }
    }
    // videos
    let videos = dedupe::video::tools_present() == (true, true);
    if videos {
        let v = root.join("videos");
        std::fs::create_dir_all(&v).unwrap();
        for (i, src) in [
            "testsrc2",
            "smptehdbars",
            "mandelbrot",
            "rgbtestsrc",
            "testsrc",
            "cellauto",
        ]
        .iter()
        .enumerate()
        {
            let a = v.join(format!("v{i}.mp4"));
            let b = v.join(format!("v{i}_low.mp4"));
            let ok = dedupe::tools::ffmpeg()
                .args([
                    "-hide_banner",
                    "-loglevel",
                    "error",
                    "-y",
                    "-nostdin",
                    "-f",
                    "lavfi",
                ])
                .args(["-i", &format!("{src}=size=640x360:rate=25"), "-t", "10"])
                .args([
                    "-c:v", "libx264", "-preset", "veryfast", "-crf", "20", "-pix_fmt", "yuv420p",
                ])
                .arg(&a)
                .status()
                .is_ok_and(|s| s.success())
                && dedupe::tools::ffmpeg()
                    .args(["-hide_banner", "-loglevel", "error", "-y", "-nostdin", "-i"])
                    .arg(&a)
                    .args(["-c:v", "libx264", "-preset", "veryfast", "-crf", "32"])
                    .arg(&b)
                    .status()
                    .is_ok_and(|s| s.success());
            assert!(ok, "ffmpeg could not make the test videos");
        }
    }
    std::fs::write(&done, b"").unwrap();
    videos
}

fn megabytes(cands: &[walk::Candidate]) -> f64 {
    cands.iter().map(|c| c.size).sum::<u64>() as f64 / 1e6
}

#[test]
#[ignore]
fn stages_on_a_fixed_corpus() {
    let keep = std::env::var_os("DEDUPE_BENCH_DIR").map(PathBuf::from);
    let tmp = tempfile::tempdir().unwrap();
    let root = keep.unwrap_or_else(|| tmp.path().join("bench"));
    let videos = build(&root);
    let photos = dirs_in("DEDUPE_PHOTO_DIRS");
    let real_videos = dirs_in("DEDUPE_VIDEO_DIRS");
    let mut roots = vec![root.clone()];
    roots.extend(photos.iter().cloned());
    roots.extend(real_videos.iter().cloned());
    // a warm page cache for every stage: read everything once
    for r in &roots {
        for e in walkdir::WalkDir::new(r).into_iter().flatten() {
            if e.file_type().is_file() {
                let _ = std::fs::read(e.path());
            }
        }
    }
    let threads = rayon::current_num_threads();
    let mut rows: Vec<Row> = Vec::new();

    // SCAN and the exact tier
    let cands = stage(&mut rows, "SCAN", || {
        let (c, files) = scan_all(&roots);
        let what = format!("{files} files, {:.0} MB", megabytes(&c));
        (c, what)
    });
    exact_rows(&mut rows, cands.clone());

    // images: DECODE, DECODE+m, MATCH
    image_rows(&mut rows, &cands, true);

    // videos: FRAMES, MATCH
    if videos || (!real_videos.is_empty() && dedupe::video::tools_present() == (true, true)) {
        let items: Vec<(VRep, PathBuf)> = cands
            .iter()
            .filter(|c| Kind::Video.matches(&c.path))
            .map(|c| {
                (
                    VRep {
                        cand: c.clone(),
                        exact_group: None,
                    },
                    c.path.clone(),
                )
            })
            .collect();
        let n_vid = items.len();
        let (vh, mut vstats) = stage(&mut rows, "FRAMES", || {
            let (vh, vstats) = video::hash_all(items, &exact::NoProgress);
            let what = format!("{n_vid} videos, {} frames", vstats.frames);
            ((vh, vstats), what)
        });
        let vh: Vec<VHashed> = vh
            .into_iter()
            .map(|(rep, sig)| VHashed { rep, sig })
            .collect();
        stage(&mut rows, "MATCH v", || {
            let vg = video::group(vh, &mut vstats, &exact::NoProgress);
            ((), format!("{} video groups", vg.len()))
        });
        // the frame pass by the graphics hardware against software, on
        // every video, whatever its length (CORE-16): the same frames, and
        // what it costs or saves
        let paths: Vec<PathBuf> = cands
            .iter()
            .filter(|c| Kind::Video.matches(&c.path))
            .map(|c| c.path.clone())
            .collect();
        let metas: Vec<(PathBuf, video::Meta)> = paths
            .iter()
            .filter_map(|p| video::probe(p).ok().map(|m| (p.clone(), m)))
            .collect();
        let pass = |hw: bool| -> (Vec<Option<VideoSig>>, f64) {
            let t = Instant::now();
            let sigs = metas
                .iter()
                .map(|(p, m)| video::frame_pass(p, m, None, hw).ok())
                .collect();
            (sigs, t.elapsed().as_secs_f64())
        };
        let (soft, soft_s) = pass(false);
        let (hard, hard_s) = pass(true);
        let same = soft
            .iter()
            .zip(&hard)
            .filter(|(a, b)| match (a, b) {
                (Some(a), Some(b)) => a.tmk == b.tmk && a.positional == b.positional,
                _ => false,
            })
            .count();
        let hours: f64 = metas.iter().map(|(_, m)| m.duration).sum::<f64>() / 3600.0;
        rows.push(Row {
            stage: "FRAMES sw".into(),
            what: format!("{} videos, {hours:.2} h, one at a time", metas.len()),
            secs: soft_s,
            peak: None,
        });
        rows.push(Row {
            stage: "FRAMES hw".into(),
            what: format!(
                "-hwaccel auto: {same} of {} signatures identical to software",
                metas.len()
            ),
            secs: hard_s,
            peak: None,
        });
    } else {
        rows.push(Row {
            stage: "FRAMES".into(),
            what: "ffmpeg not found: no videos".into(),
            secs: 0.0,
            peak: None,
        });
    }

    let mut header = format!("{threads} threads   corpus at {}", root.display());
    for d in photos.iter().chain(&real_videos) {
        header.push_str(&format!(" + {}", d.display()));
    }
    print(&rows, &header);
}

/// The scale corpus (see the module's head); `true` once written.
fn build_scale(root: &Path, n: u64) {
    let done = root.join(format!(".complete-{n}"));
    if done.exists() {
        return;
    }
    use rayon::prelude::*;
    let images = n * 8 / 100;
    (0..n).into_par_iter().for_each(|i| {
        let dir = root.join(format!("{:04}", i / 100));
        if i < images {
            // one in eight a half-size copy of the image before it
            let seed = if i % 8 == 7 { i - 1 } else { i };
            let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
            let (w, h) = (192u32, 144u32);
            let base = value_noise(&mut rng, w, h);
            let tint = [rng.f(), rng.f(), rng.f()];
            let img = image::RgbImage::from_fn(w, h, |x, y| {
                let v = base[(y * w + x) as usize];
                image::Rgb([
                    (255.0 * v * (0.5 + 0.5 * tint[0])) as u8,
                    (255.0 * v * (0.5 + 0.5 * tint[1])) as u8,
                    (255.0 * v * (0.5 + 0.5 * tint[2])) as u8,
                ])
            });
            let img = if i % 8 == 7 { scale(&img, 0.5) } else { img };
            write(&dir.join(format!("i{i:06}.jpg")), &jpeg(&img, 85));
        } else {
            let len = |k: u64| 512 + (k * 7919 % 7680) as usize;
            // one in ten a byte copy of the file before it
            let k = if i % 10 == 9 { i - 1 } else { i };
            write(&dir.join(format!("f{i:06}.bin")), &bytes(k, len(k)));
        }
    });
    std::fs::write(&done, b"").unwrap();
}

#[test]
#[ignore]
fn scale_run() {
    let n: u64 = std::env::var("DEDUPE_SCALE_FILES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(100_000);
    let keep = std::env::var_os("DEDUPE_SCALE_DIR").map(PathBuf::from);
    let tmp = tempfile::tempdir().unwrap();
    let root = keep.unwrap_or_else(|| tmp.path().join("scale"));
    let t = Instant::now();
    build_scale(&root, n);
    let built = t.elapsed().as_secs_f64();
    let threads = rayon::current_num_threads();
    let mut rows: Vec<Row> = Vec::new();
    let cands = stage(&mut rows, "SCAN", || {
        let (c, files) = scan_all(std::slice::from_ref(&root));
        let what = format!("{files} files, {:.0} MB", megabytes(&c));
        (c, what)
    });
    exact_rows(&mut rows, cands.clone());
    image_rows(&mut rows, &cands, false);
    print(
        &rows,
        &format!(
            "{threads} threads   {n} files at {} (written in {built:.1} s)",
            root.display()
        ),
    );
}
