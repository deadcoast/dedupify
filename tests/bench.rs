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

mod common;
use common::*;

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use dedupe::exact::{self, PipelineStats, Progress, Stage};
use dedupe::kind::Kind;
use dedupe::near::{self, Rep};
use dedupe::video::{self, VHashed, VRep};
use dedupe::walk::{self, Filter};

/// Records when each stage of the exact tier ends.
struct Clock {
    t0: Instant,
    ends: Mutex<Vec<(Stage, Duration)>>,
}

impl Progress for Clock {
    fn tick(&self, _: Stage, _: u64, _: u64) {}
    fn done(&self, stage: Stage, _: &PipelineStats) {
        self.ends.lock().unwrap().push((stage, self.t0.elapsed()));
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
    // a warm page cache for every stage: read everything once
    for e in walkdir::WalkDir::new(&root).into_iter().flatten() {
        if e.file_type().is_file() {
            let _ = std::fs::read(e.path());
        }
    }
    let threads = rayon::current_num_threads();
    let mut rows: Vec<(String, String, f64)> = Vec::new();
    let filter = Filter { skip: &[], min: 0 };

    // SCAN and the exact tier
    let t = Instant::now();
    let (cands, scan) = walk::scan(&root, true, Kind::All, filter, &mut |_| {});
    rows.push((
        "SCAN".into(),
        format!("{} files, {:.0} MB", scan.files, megabytes(&cands)),
        t.elapsed().as_secs_f64(),
    ));
    let clock = Clock {
        t0: Instant::now(),
        ends: Mutex::new(Vec::new()),
    };
    let (groups, pipe) = exact::find_groups(cands.clone(), false, &clock);
    let mut last = Duration::ZERO;
    for (stage, end) in clock.ends.lock().unwrap().iter() {
        let text = match stage {
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
        rows.push((stage.label().into(), text, (*end - last).as_secs_f64()));
        last = *end;
    }

    // images: DECODE, MATCH
    let reps: Vec<Rep> = cands
        .iter()
        .filter(|c| dedupe::decode::decodable(&c.path))
        .map(|c| Rep {
            cand: c.clone(),
            exact_group: None,
        })
        .collect();
    let n_img = reps.len();
    let t = Instant::now();
    let (hashed, mut nstats) = near::hash_all(reps, &exact::NoProgress, None);
    let decode = t.elapsed().as_secs_f64();
    rows.push((
        "DECODE".into(),
        format!("{n_img} images, {:.1} a second", n_img as f64 / decode),
        decode,
    ));
    let t = Instant::now();
    let ngroups = near::group(hashed, &mut nstats, &exact::NoProgress);
    rows.push((
        "MATCH".into(),
        format!("{} near groups", ngroups.len()),
        t.elapsed().as_secs_f64(),
    ));

    // videos: FRAMES, MATCH
    if videos {
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
        let t = Instant::now();
        let (vh, mut vstats) = video::hash_all(items, &exact::NoProgress);
        let frames = t.elapsed().as_secs_f64();
        rows.push((
            "FRAMES".into(),
            format!("{n_vid} videos, {} frames", vstats.frames),
            frames,
        ));
        let vh: Vec<VHashed> = vh
            .into_iter()
            .map(|(rep, sig)| VHashed { rep, sig })
            .collect();
        let t = Instant::now();
        let vg = video::group(vh, &mut vstats, &exact::NoProgress);
        rows.push((
            "MATCH v".into(),
            format!("{} video groups", vg.len()),
            t.elapsed().as_secs_f64(),
        ));
    } else {
        rows.push(("FRAMES".into(), "ffmpeg not found: no videos".into(), 0.0));
    }

    eprintln!();
    eprintln!("{} threads   corpus at {}", threads, root.display());
    for (stage, what, secs) in &rows {
        eprintln!(" {stage:<8} {secs:>7.3} s   {what}");
    }
}
