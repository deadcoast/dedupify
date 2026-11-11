//! Threshold measurement corpus for the video pass.
//!
//! `cargo test --release --test video_corpus -- --ignored --nocapture`
//!
//! Needs `ffmpeg` and `ffprobe` (as `dedupe::tools` finds them); skips otherwise. Generates 16
//! distinct 20-second clips from ffmpeg's synthetic sources (fractal zooms,
//! cellular automata, test patterns, gradients, a static card), applies the
//! edits a library shows (re-encode, downscale, crop, head/tail/both trims,
//! title card, speed change, mirror, letterbox, frame-rate change,
//! brightness), and measures, per edit, the positional medians and the TMK
//! level-1/level-2 scores against the source, plus every negative pair.
//!
//! `DEDUPE_VIDEO_CORPUS_DIR` keeps the generated clips for a second run.

use std::path::{Path, PathBuf};
use std::time::Instant;

use dedupe::pdq::PDQ_MAX;
use dedupe::perceptual::{DHASH_MAX, PHASH_MAX};
use dedupe::video::{
    hash_video, level1, level2, probe, relate, tools_present, VideoSig, LEVEL1_MIN, LEVEL2_CONFIRM,
    LEVEL2_MIN,
};

const SOURCES: &[(&str, &str)] = &[
    ("mandel_a", "mandelbrot=size=640x360:rate=24:start_scale=3:end_scale=0.3:maxiter=200"),
    ("mandel_b", "mandelbrot=size=640x360:rate=24:start_x=-0.75:start_y=0.1:start_scale=2:end_scale=0.05:maxiter=300"),
    ("mandel_c", "mandelbrot=size=640x360:rate=24:start_x=0.3:start_y=-0.6:start_scale=1.5:end_scale=0.01:maxiter=400:inner=convergence"),
    ("mandel_d", "mandelbrot=size=640x360:rate=24:start_x=-1.4:start_y=0.0:start_scale=0.5:end_scale=0.002:maxiter=500:outer=normalized_iteration_count"),
    ("life_a", "life=size=640x360:rate=24:ratio=0.4:seed=7:mold=10"),
    ("life_b", "life=size=640x360:rate=24:ratio=0.3:seed=99:mold=30:life_color=#ffcc00:death_color=#202040"),
    ("life_c", "life=size=640x360:rate=24:ratio=0.5:seed=1234:rule=B36/S23:mold=0"),
    ("cell_a", "cellauto=size=640x360:rate=24:rule=30:scroll=1"),
    ("cell_b", "cellauto=size=640x360:rate=24:rule=110:scroll=1:random_fill_ratio=0.3:random_seed=5"),
    ("cell_c", "cellauto=size=640x360:rate=24:rule=184:scroll=0:random_fill_ratio=0.5:random_seed=9"),
    ("testsrc", "testsrc=size=640x360:rate=24"),
    ("testsrc2", "testsrc2=size=640x360:rate=30"),
    ("grad_a", "gradients=size=640x360:rate=24:speed=0.05:seed=3"),
    ("grad_b", "gradients=size=640x360:rate=24:speed=0.2:seed=44:nb_colors=5"),
    ("bars", "smptehdbars=size=640x360:rate=24"),
    ("rgbtest", "rgbtestsrc=size=640x360:rate=24"),
];

/// (label, input-side args, filter, output-side args)
const EDITS: &[(&str, &[&str], &str, &[&str])] = &[
    ("re-encode crf30", &[], "", &["-crf", "30"]),
    ("scale 50%", &[], "scale=320:180", &["-crf", "22"]),
    ("crop 90%", &[], "crop=iw*0.9:ih*0.9", &["-crf", "22"]),
    ("trim head 3s", &["-ss", "3"], "", &["-crf", "22"]),
    ("trim tail 3s", &[], "", &["-t", "17", "-crf", "22"]),
    (
        "trim both 2+2s",
        &["-ss", "2"],
        "",
        &["-t", "16", "-crf", "22"],
    ),
    (
        "title card 3s",
        &[],
        "tpad=start_duration=3:color=black",
        &["-crf", "22"],
    ),
    ("speed 1.1x", &[], "setpts=PTS/1.1", &["-crf", "22"]),
    ("mirror", &[], "hflip", &["-crf", "22"]),
    (
        "letterbox",
        &[],
        "pad=iw:ih*1.25:0:(oh-ih)/2",
        &["-crf", "22"],
    ),
    ("fps 15", &[], "fps=15", &["-crf", "22"]),
    ("bright +0.1", &[], "eq=brightness=0.1", &["-crf", "22"]),
];

fn ffmpeg(args: &[&str]) -> bool {
    dedupe::tools::ffmpeg()
        .args(["-hide_banner", "-loglevel", "error", "-y", "-nostdin"])
        .args(args)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn make_source(dir: &Path, name: &str, lavfi: &str) -> PathBuf {
    let out = dir.join(format!("{name}.mp4"));
    if !out.exists() {
        assert!(
            ffmpeg(&[
                "-f",
                "lavfi",
                "-i",
                lavfi,
                "-t",
                "20",
                "-c:v",
                "libx264",
                "-preset",
                "veryfast",
                "-crf",
                "20",
                "-pix_fmt",
                "yuv420p",
                out.to_str().unwrap()
            ]),
            "ffmpeg failed for {name}"
        );
    }
    out
}

fn make_edit(dir: &Path, src: &Path, name: &str, k: usize) -> PathBuf {
    let (_, pre, vf, post) = EDITS[k];
    let out = dir.join(format!("{name}__e{k:02}.mp4"));
    if !out.exists() {
        let mut args: Vec<&str> = Vec::new();
        args.extend_from_slice(pre);
        args.extend_from_slice(&["-i", src.to_str().unwrap()]);
        if !vf.is_empty() {
            args.extend_from_slice(&["-vf", vf]);
        }
        args.extend_from_slice(&[
            "-an", "-c:v", "libx264", "-preset", "veryfast", "-pix_fmt", "yuv420p",
        ]);
        args.extend_from_slice(post);
        args.push(out.to_str().unwrap());
        assert!(ffmpeg(&args), "ffmpeg failed for {name} edit {k}");
    }
    out
}

fn sig(p: &Path) -> VideoSig {
    let m = probe(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
    hash_video(p, &m).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

#[test]
#[ignore]
fn measure_video_thresholds() {
    let (ff, fp) = tools_present();
    if !(ff && fp) {
        eprintln!("ffmpeg/ffprobe not found; skipping");
        return;
    }
    let keep = std::env::var("DEDUPE_VIDEO_CORPUS_DIR").ok();
    let tmp;
    let dir: PathBuf = match &keep {
        Some(d) => {
            std::fs::create_dir_all(d).unwrap();
            PathBuf::from(d)
        }
        None => {
            tmp = tempfile::tempdir().unwrap();
            tmp.path().to_path_buf()
        }
    };

    let t0 = Instant::now();
    let sources: Vec<PathBuf> = SOURCES
        .iter()
        .map(|(n, l)| make_source(&dir, n, l))
        .collect();
    let edits: Vec<Vec<PathBuf>> = SOURCES
        .iter()
        .zip(&sources)
        .map(|((n, _), src)| {
            (0..EDITS.len())
                .map(|k| make_edit(&dir, src, n, k))
                .collect()
        })
        .collect();
    eprintln!();
    eprintln!(
        "corpus: {} sources × {} edits generated/found in {:.0}s",
        SOURCES.len(),
        EDITS.len(),
        t0.elapsed().as_secs_f64()
    );

    let t1 = Instant::now();
    let src_sigs: Vec<VideoSig> = sources.iter().map(|p| sig(p)).collect();
    let edit_sigs: Vec<Vec<VideoSig>> = edits
        .iter()
        .map(|v| v.iter().map(|p| sig(p)).collect())
        .collect();
    let total_frames: u32 = src_sigs.iter().map(|s| s.frames).sum::<u32>()
        + edit_sigs.iter().flatten().map(|s| s.frames).sum::<u32>();
    let n_clips = src_sigs.len() + edit_sigs.iter().map(Vec::len).sum::<usize>();
    let hash_s = t1.elapsed().as_secs_f64();
    eprintln!("hashed {n_clips} clips, {total_frames} frames at 15 fps, in {hash_s:.1}s  ({:.2} ms per frame incl. ffmpeg)", hash_s * 1000.0 / total_frames as f64);

    eprintln!();
    eprintln!(
        "{:<16} {:>6} {:>6} {:>6} {:>7} {:>7} {:>6} {:>6} {:>7} {:>8}",
        "edit", "d med", "p med", "q med", "l1 min", "l2 min", "pos", "tmk", "group", "confirm"
    );
    let n = SOURCES.len() as f64;
    for (k, (label, _, _, _)) in EDITS.iter().enumerate() {
        let mut ds = Vec::new();
        let mut ps = Vec::new();
        let mut qs = Vec::new();
        let mut l1s = Vec::new();
        let mut l2s = Vec::new();
        let (mut pos_ok, mut tmk_ok, mut grp, mut conf) = (0, 0, 0, 0);
        for (i, s) in src_sigs.iter().enumerate() {
            let r = relate(s, &edit_sigs[i][k]);
            if let Some((d, p, q)) = r.pos {
                ds.push(d);
                ps.push(p);
                qs.push(q);
            }
            l1s.push(r.level1);
            // level 2 regardless of the level-1 gate, for the record
            let (l2, _) = level2(&s.tmk, &edit_sigs[i][k].tmk);
            l2s.push(l2);
            pos_ok += r.positional_ok() as u32;
            tmk_ok += r.tmk_ok() as u32;
            grp += r.similar() as u32;
            conf += r.confirmed() as u32;
        }
        let med = |v: &mut Vec<u32>| {
            if v.is_empty() {
                "-".to_string()
            } else {
                v.sort_unstable();
                v[v.len() / 2].to_string()
            }
        };
        let fmin = |v: &[f32]| v.iter().cloned().fold(f32::MAX, f32::min);
        eprintln!(
            "{:<16} {:>6} {:>6} {:>6} {:>7.3} {:>7.3} {:>5.0}% {:>5.0}% {:>6.0}% {:>7.0}%",
            label,
            med(&mut ds),
            med(&mut ps),
            med(&mut qs),
            fmin(&l1s),
            fmin(&l2s),
            pos_ok as f64 / n * 100.0,
            tmk_ok as f64 / n * 100.0,
            grp as f64 / n * 100.0,
            conf as f64 / n * 100.0
        );
    }

    // negatives: sources vs sources, and every edit vs every other source
    let mut neg_l1 = Vec::new();
    let mut neg_l2 = Vec::new();
    let mut neg_pos: Vec<(u32, u32, u32)> = Vec::new();
    let mut fp_group = 0u32;
    let mut fp_confirm = 0u32;
    let mut worst: Vec<(f32, String)> = Vec::new();
    let name = |i: usize, k: Option<usize>| match k {
        None => SOURCES[i].0.to_string(),
        Some(k) => format!("{}/{}", SOURCES[i].0, EDITS[k].0),
    };
    for i in 0..src_sigs.len() {
        for j in 0..src_sigs.len() {
            if i == j {
                continue;
            }
            let mut pairs: Vec<(&VideoSig, String)> = vec![(&src_sigs[j], name(j, None))];
            for (k, e) in edit_sigs[j].iter().enumerate() {
                pairs.push((e, name(j, Some(k))));
            }
            for (other, oname) in pairs {
                if i > j && oname == name(j, None) {
                    continue; // source–source counted once
                }
                let r = relate(&src_sigs[i], other);
                let (l2, _) = level2(&src_sigs[i].tmk, &other.tmk);
                neg_l1.push(r.level1);
                neg_l2.push(l2);
                if let Some(p) = r.pos {
                    neg_pos.push(p);
                }
                fp_group += r.similar() as u32;
                fp_confirm += r.confirmed() as u32;
                worst.push((r.level1.max(l2), format!("{} vs {}", name(i, None), oname)));
            }
        }
    }
    let fmax = |v: &[f32]| v.iter().cloned().fold(f32::MIN, f32::max);
    let min_d = neg_pos.iter().map(|p| p.0).min().unwrap_or(0);
    let min_p = neg_pos.iter().map(|p| p.1).min().unwrap_or(0);
    let min_q = neg_pos.iter().map(|p| p.2).min().unwrap_or(0);
    eprintln!();
    eprintln!(
        "negatives: {} pairs   max l1 {:.3}   max l2 {:.3}   min positional d {} p {} q {}   false groups {}   false confirms {}",
        neg_l1.len(),
        fmax(&neg_l1),
        fmax(&neg_l2),
        min_d,
        min_p,
        min_q,
        fp_group,
        fp_confirm
    );
    worst.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
    eprintln!("closest negatives by max(l1, l2):");
    for (v, s) in worst.iter().take(6) {
        eprintln!("   {v:.3}  {s}");
    }
    // per-edit level-2 distribution (min / p10 / median) for the rows TMK is meant to carry
    eprintln!();
    eprintln!("level-2 per edit (min / p10 / median) and level-1 min:");
    for (k, (label, _, _, _)) in EDITS.iter().enumerate() {
        let mut v: Vec<f32> = src_sigs
            .iter()
            .enumerate()
            .map(|(i, s)| level2(&s.tmk, &edit_sigs[i][k].tmk).0)
            .collect();
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let l1min = src_sigs
            .iter()
            .enumerate()
            .map(|(i, s)| level1(&s.tmk, &edit_sigs[i][k].tmk))
            .fold(f32::MAX, f32::min);
        eprintln!(
            "   {:<16} {:.3} / {:.3} / {:.3}    l1 min {:.3}",
            label,
            v[0],
            v[v.len() / 10],
            v[v.len() / 2],
            l1min
        );
    }
    // 2-D sweep: false groups | recall over the TMK-carried edits (all but crop, mirror, letterbox)
    let tmk_edits: Vec<usize> = (0..EDITS.len())
        .filter(|k| ![2usize, 8, 9].contains(k))
        .collect();
    eprintln!();
    eprintln!(
        "TMK sweep — false groups | recall over re-encode/scale/trims/title/speed/fps/bright:"
    );
    eprint!("{:<8}", "l1\\l2");
    for t2 in [0.7f32, 0.75, 0.8, 0.85, 0.9] {
        eprint!("{:>14}", format!("l2≥{t2:.2}"));
    }
    eprintln!();
    for t1 in [0.5f32, 0.6, 0.7, 0.8] {
        eprint!("l1≥{t1:.1}  ");
        for t2 in [0.7f32, 0.75, 0.8, 0.85, 0.9] {
            let fp = neg_l1
                .iter()
                .zip(&neg_l2)
                .filter(|(a, b)| **a >= t1 && **b >= t2)
                .count();
            let mut hit = 0;
            let mut tot = 0;
            for (i, s) in src_sigs.iter().enumerate() {
                for &k in &tmk_edits {
                    tot += 1;
                    let l1 = level1(&s.tmk, &edit_sigs[i][k].tmk);
                    let (l2, _) = level2(&s.tmk, &edit_sigs[i][k].tmk);
                    if l1 >= t1 && l2 >= t2 {
                        hit += 1;
                    }
                }
            }
            eprint!(
                "{:>14}",
                format!("{fp} | {:.1}%", hit as f64 / tot as f64 * 100.0)
            );
        }
        eprintln!();
    }
    // the positional near miss
    let mut by_p: Vec<(u32, u32, u32)> = neg_pos.clone();
    by_p.sort_by_key(|p| p.0 + p.1);
    eprintln!(
        "closest negatives by positional d+p: {:?}",
        &by_p[..by_p.len().min(5)]
    );
    eprintln!("shipped: group at level1 ≥ {LEVEL1_MIN} ∧ level2 ≥ {LEVEL2_MIN}, bin at level2 ≥ {LEVEL2_CONFIRM}; positional d ≤ {DHASH_MAX} p ≤ {PHASH_MAX}, q ≤ {PDQ_MAX} to bin");
    eprintln!();
}
