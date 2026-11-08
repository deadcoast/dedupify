//! TMK conformance against Meta's reference feature-vector files.
//!
//! `cargo run --release --example tmk_ref -- <ThreatExchange>/tmk`
//!
//! Reads the `.tmk` files the reference `tmk-hash-video` produced for
//! `sample-videos/*.mp4` (`sample-hashes/recompute.sh`), computes this
//! crate's descriptor from the same videos two ways — the shipped frame
//! path (288-px grey frames, Jarosz-filtered to 64) and the reference
//! tool's frame path (ffmpeg `-s 64:64 -pix_fmt rgb24 -r 15`, Rec. 601
//! luma, DCT directly) — and reports how the three agree: level 1 and
//! level 2 of each of ours against the reference file for the same video,
//! then the pair scores the reference files give against ours for the
//! related and unrelated pairs in the set.
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use dedupe::pdq;
use dedupe::video::{self, Tmk, TmkBuilder, COEFFS, DIM, PERIODS};

/// Parse a reference `.tmk` file into our `Tmk` layout.
fn read_tmk(path: &Path) -> Tmk {
    let bytes = std::fs::read(path).unwrap();
    assert_eq!(&bytes[0..4], b"TMK1");
    assert_eq!(&bytes[4..8], b"FVEC");
    assert_eq!(&bytes[8..12], b"PDQF");
    let mut pos = 12usize;
    let mut take_i32 = |n: usize| -> Vec<i32> {
        let v: Vec<i32> = (0..n)
            .map(|k| i32::from_le_bytes(bytes[pos + k * 4..pos + k * 4 + 4].try_into().unwrap()))
            .collect();
        pos += n * 4;
        v
    };
    let header = take_i32(5); // fps, P, m, d, frame count
    let (fps, p, m, d, frames) = (
        header[0],
        header[1] as usize,
        header[2] as usize,
        header[3] as usize,
        header[4],
    );
    assert_eq!(fps, 15);
    assert_eq!(d, DIM);
    assert_eq!(p, PERIODS.len());
    assert_eq!(m, COEFFS.len());
    let periods = take_i32(p);
    assert_eq!(
        periods,
        PERIODS.iter().map(|x| *x as i32).collect::<Vec<_>>()
    );
    let mut take_f32 = |n: usize| -> Vec<f32> {
        let v: Vec<f32> = (0..n)
            .map(|k| f32::from_le_bytes(bytes[pos + k * 4..pos + k * 4 + 4].try_into().unwrap()))
            .collect();
        pos += n * 4;
        v
    };
    let coeffs = take_f32(m);
    for (a, b) in coeffs.iter().zip(COEFFS.iter()) {
        assert!(
            (a - b).abs() < 1e-9,
            "coefficient table differs: {a} vs {b}"
        );
    }
    let pure = take_f32(d);
    let cos = take_f32(p * m * d);
    let sin = take_f32(p * m * d);
    assert_eq!(pos, bytes.len(), "trailing bytes in {}", path.display());
    Tmk {
        pure,
        cos,
        sin,
        frames: frames as u32,
    }
}

/// Our descriptor from the reference tool's frame path.
fn tmk_tool_path(video: &Path) -> Tmk {
    let mut child = Command::new("ffmpeg")
        .args(["-loglevel", "error", "-nostdin", "-i"])
        .arg(video)
        .args([
            "-s", "64:64", "-an", "-f", "rawvideo", "-c:v", "rawvideo", "-pix_fmt", "rgb24", "-r",
            "15", "pipe:1",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut out = child.stdout.take().unwrap();
    let mut builder = TmkBuilder::new();
    let mut frame = vec![0u8; 64 * 64 * 3];
    let mut t = 0u32;
    loop {
        if out.read_exact(&mut frame).is_err() {
            break;
        }
        let luma: Vec<f32> = (0..64 * 64)
            .map(|i| {
                0.299 * frame[i * 3] as f32
                    + 0.587 * frame[i * 3 + 1] as f32
                    + 0.114 * frame[i * 3 + 2] as f32
            })
            .collect();
        // 64×64 goes straight to the DCT in the reference (no Jarosz pass)
        let band = pdq::dct16(&luma);
        builder.ingest(t, &band);
        t += 1;
    }
    let _ = child.wait();
    builder.finish()
}

/// Our descriptor from the shipped frame path.
fn tmk_shipped(video: &Path) -> Tmk {
    let meta = video::probe(video).unwrap();
    video::hash_video(video, &meta).unwrap().tmk
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let root = Path::new(&args[1]);
    let mut names: Vec<String> = std::fs::read_dir(root.join("sample-videos"))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".mp4"))
        .collect();
    names.sort();
    let mut refs: Vec<(String, Tmk, Tmk, Tmk)> = Vec::new();
    println!(
        "{:<36} {:>6} {:>6} {:>6}   {:>8} {:>8}   {:>8} {:>8} {:>9}",
        "video", "ref fr", "ship", "tool", "l1 ship", "l2 ship", "l1 tool", "l2 tool", "max|Δ|"
    );
    for name in &names {
        let video: PathBuf = root.join("sample-videos").join(name);
        let tmk_path = root
            .join("sample-hashes")
            .join(name.replace(".mp4", ".tmk"));
        if !tmk_path.exists() {
            continue;
        }
        let r = read_tmk(&tmk_path);
        let s = tmk_shipped(&video);
        let t = tmk_tool_path(&video);
        let (l2s, _) = video::level2(&r, &s);
        let (l2t, _) = video::level2(&r, &t);
        // largest element difference between the reference file and our
        // tool-path descriptor, over the pure average and every cos/sin
        // feature (the file stores f32; differences are float-order noise
        // when the two computations agree)
        let max_delta = r
            .pure
            .iter()
            .zip(&t.pure)
            .chain(r.cos.iter().zip(&t.cos))
            .chain(r.sin.iter().zip(&t.sin))
            .map(|(x, y)| (x - y).abs())
            .fold(0.0f32, f32::max);
        println!(
            "{name:<36} {:>6} {:>6} {:>6}   {:>8.4} {:>8.4}   {:>8.4} {:>8.4} {:>9.2e}",
            r.frames,
            s.frames,
            t.frames,
            video::level1(&r, &s),
            l2s,
            video::level1(&r, &t),
            l2t,
            max_delta
        );
        refs.push((name.clone(), r, s, t));
    }
    println!();
    println!("pair scores — reference files against each other, then ours (shipped path):");
    println!(
        "{:<70} {:>6} {:>6}   {:>6} {:>6}",
        "pair", "ref l1", "ref l2", "our l1", "our l2"
    );
    let pairs = [
        ("chair-orig-22-sd-bar.mp4", "chair-22-sd-grey-bar.mp4"),
        ("chair-orig-22-sd-bar.mp4", "chair-22-sd-sepia-bar.mp4"),
        (
            "chair-orig-22-sd-bar.mp4",
            "chair-22-with-large-logo-bar.mp4",
        ),
        (
            "chair-orig-22-sd-bar.mp4",
            "chair-22-with-small-logo-bar.mp4",
        ),
        ("chair-orig-22-sd-bar.mp4", "chair-orig-22-hd-no-bar.mp4"),
        ("chair-orig-22-sd-bar.mp4", "chair-orig-22-fhd-no-bar.mp4"),
        ("chair-19-sd-bar.mp4", "chair-20-sd-bar.mp4"),
        ("chair-19-sd-bar.mp4", "chair-orig-22-sd-bar.mp4"),
        (
            "pattern-hd-no-bar.mp4",
            "pattern-sd-with-small-logo-bar.mp4",
        ),
        (
            "pattern-hd-no-bar.mp4",
            "pattern-sd-with-large-logo-bar.mp4",
        ),
        ("pattern-hd-no-bar.mp4", "pattern-sd-grey-bar.mp4"),
        ("pattern-hd-no-bar.mp4", "pattern-longer-no-bar.mp4"),
        ("doorknob-hd-no-bar.mp4", "chair-orig-22-sd-bar.mp4"),
        ("doorknob-hd-no-bar.mp4", "pattern-hd-no-bar.mp4"),
        ("chair-orig-22-sd-bar.mp4", "pattern-hd-no-bar.mp4"),
    ];
    for (a, b) in pairs {
        let fa = refs.iter().find(|r| r.0 == a);
        let fb = refs.iter().find(|r| r.0 == b);
        let (Some(fa), Some(fb)) = (fa, fb) else {
            continue;
        };
        let (rl2, _) = video::level2(&fa.1, &fb.1);
        let (ol2, _) = video::level2(&fa.2, &fb.2);
        println!(
            "{:<70} {:>6.3} {:>6.3}   {:>6.3} {:>6.3}",
            format!("{a} ~ {b}"),
            video::level1(&fa.1, &fb.1),
            rl2,
            video::level1(&fa.2, &fb.2),
            ol2
        );
    }
}
