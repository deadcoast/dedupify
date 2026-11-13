//! CORE-03 (Phase 11): bars of any colour around a video's picture.
//!
//! ```text
//! DEDUPE_BAR_CLIPS=<folder of real clips> DEDUPE_BAR_OUT=<work folder> \
//!   cargo test --release --test bars_measure -- --ignored --nocapture
//! ```
//!
//! From every real clip (≥ 1 s): a portrait original (the middle 9:16 of
//! a landscape clip, 720 high), then that portrait in a 1280×720 frame
//! the ways platforms post it — over a blurred, enlarged copy of itself
//! (`ambient`), between white, grey or dark-blue bars — and the
//! landscape clip in a white letterbox. Measured: whether the black-bar
//! pass and the flat-bar pass find the picture (each edge within 2 % of
//! its side), whether either finds bars in the originals or their
//! re-encodes (which have none), and the TMK relation of each framed
//! copy to its original with and without the bars cropped away.
//! Nothing here is a unit test; it prints a record for VERIFICATION.md.

use std::path::{Path, PathBuf};
use std::time::Instant;

use dedupe::video::{
    detect_bars, detect_flat_bars, hash_video_inside, probe, relate, tools_present, Bars,
};

fn ffmpeg(args: &[&str]) -> bool {
    dedupe::tools::ffmpeg()
        .args(["-hide_banner", "-loglevel", "error", "-y", "-nostdin"])
        .args(args)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn make(src: &Path, out: &Path, filter: &str, complex: bool) -> bool {
    if out.exists() {
        return true;
    }
    let s = src.to_string_lossy();
    let o = out.to_string_lossy();
    let f = if complex { "-filter_complex" } else { "-vf" };
    let ok = ffmpeg(&[
        "-i", &s, "-an", f, filter, "-c:v", "libx264", "-crf", "20", "-preset", "veryfast",
        "-pix_fmt", "yuv420p", &o,
    ]);
    if !ok {
        // ffmpeg can leave a file it wrote nothing into; a rerun must not
        // take it for a made one
        let _ = std::fs::remove_file(out);
    }
    ok
}

/// Each edge of `got` within 2 % of the side from `want`.
fn close(got: Option<Bars>, want: (u32, u32, u32, u32), w: u32, h: u32) -> bool {
    let Some(b) = got else { return false };
    let (x0, y0, x1, y1) = want;
    let dx = |a: u32, b: u32| (a as f64 - b as f64).abs() / w as f64 <= 0.02;
    let dy = |a: u32, b: u32| (a as f64 - b as f64).abs() / h as f64 <= 0.02;
    dx(b.x, x0) && dy(b.y, y0) && dx(b.x + b.width, x1) && dy(b.y + b.height, y1)
}

#[test]
#[ignore]
fn measure_bars() {
    let (Some(clips), Some(out)) = (
        std::env::var_os("DEDUPE_BAR_CLIPS").map(PathBuf::from),
        std::env::var_os("DEDUPE_BAR_OUT").map(PathBuf::from),
    ) else {
        eprintln!("DEDUPE_BAR_CLIPS / DEDUPE_BAR_OUT not set; skipped");
        return;
    };
    if tools_present() != (true, true) {
        eprintln!("no ffmpeg; skipped");
        return;
    }
    std::fs::create_dir_all(&out).unwrap();
    let t0 = Instant::now();
    let mut sources: Vec<PathBuf> = walkdir::WalkDir::new(&clips)
        .follow_links(true)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file())
        .map(|e| e.into_path())
        .filter(|p| {
            p.extension().is_some_and(|e| {
                dedupe::kind::VIDEO_EXT.contains(&e.to_string_lossy().to_lowercase().as_str())
            })
        })
        .collect();
    sources.sort();
    // (variant, found by black pass, found by flat pass, related before, related after)
    let mut rows: Vec<(String, String, bool, bool, String, String)> = Vec::new();
    let mut false_found: Vec<(String, &'static str, Bars)> = Vec::new();
    let mut negatives = 0;
    // the black pass's band detail where it found bars, and how often the
    // shipped decision (`detect_any_bars`) found the picture
    let mut details: Vec<(String, f64)> = Vec::new();
    let mut shipped_right = 0;
    let mut shipped_false: Vec<String> = Vec::new();
    for src in &sources {
        let Ok(meta) = probe(src) else { continue };
        if meta.duration < 1.0 || meta.width.min(meta.height) < 240 {
            continue;
        }
        let name = src.file_stem().unwrap().to_string_lossy().replace(' ', "_");
        // negatives: the clip itself and a smaller re-encode
        let re = out.join(format!("{name}~re.mp4"));
        if !make(src, &re, "scale=iw/2:-2", false) {
            continue;
        }
        for (label, p) in [("original", src.clone()), ("re-encode", re.clone())] {
            let Ok(m) = probe(&p) else { continue };
            negatives += 1;
            if let Some(b) = detect_bars(&p, &m) {
                false_found.push((format!("{name} {label}"), "black", b));
            }
            if let Some(b) = detect_flat_bars(&p, &m) {
                false_found.push((format!("{name} {label}"), "flat", b));
            }
            if let Some((_, r)) = dedupe::video::black_band_detail(&p, &m) {
                details.push((format!("{name} {label} (no bars)"), r));
            }
            if dedupe::video::detect_any_bars(&p, &m).is_some() {
                shipped_false.push(format!("{name} {label}"));
            }
        }
        // the portrait original
        let portrait = out.join(format!("{name}~portrait.mp4"));
        let pf = if meta.width > meta.height {
            "crop=ih*9/16:ih,scale=-2:720"
        } else {
            "scale=-2:720"
        };
        if !make(src, &portrait, pf, false) {
            continue;
        }
        let Ok(pm) = probe(&portrait) else { continue };
        let Ok(psig) = hash_video_inside(&portrait, &pm, None) else {
            continue;
        };
        // the portrait as the pad sees it: 720 high, its own width (a
        // portrait original keeps its aspect, 3:4 or 9:16)
        let pw = ((pm.width as f64 * 720.0 / pm.height as f64 / 2.0).round() as u32) * 2;
        if pw >= 1280 {
            continue;
        }
        let x0 = (1280 - pw) / 2;
        let framed = [
            (
                "ambient",
                "[0:v]scale=-2:720[fg];[0:v]scale=1280:720:force_original_aspect_ratio=increase,crop=1280:720,gblur=sigma=30,eq=brightness=-0.08[bg];[bg][fg]overlay=(W-w)/2:(H-h)/2".to_string(),
                true,
            ),
            ("black", "scale=-2:720,pad=1280:720:(ow-iw)/2:0:black".into(), false),
            ("white", "scale=-2:720,pad=1280:720:(ow-iw)/2:0:white".into(), false),
            ("grey", "scale=-2:720,pad=1280:720:(ow-iw)/2:0:0x808080".into(), false),
            ("dark blue", "scale=-2:720,pad=1280:720:(ow-iw)/2:0:0x203050".into(), false),
        ];
        for (label, filter, complex) in framed {
            let v = out.join(format!("{name}~{}.mp4", label.replace(' ', "_")));
            if !make(&portrait, &v, &filter, complex) {
                continue;
            }
            let Ok(m) = probe(&v) else { continue };
            let black = detect_bars(&v, &m);
            let flat = detect_flat_bars(&v, &m);
            let shipped = dedupe::video::detect_any_bars(&v, &m);
            if let Some((_, r)) = dedupe::video::black_band_detail(&v, &m) {
                details.push((format!("{name} {label}"), r));
            }
            let want = (x0, 0, x0 + pw, 720);
            if close(shipped, want, 1280, 720) {
                shipped_right += 1;
            }
            let before = hash_video_inside(&v, &m, None).map(|s| relate(&psig, &s));
            let after = hash_video_inside(&v, &m, shipped).map(|s| relate(&psig, &s));
            let fmt = |r: Result<dedupe::video::VideoRelation, _>| match r {
                Ok(r) => format!(
                    "{:.2} {}",
                    r.level2,
                    if r.confirmed() {
                        "confirmed"
                    } else if r.similar() {
                        "grouped"
                    } else {
                        "apart"
                    }
                ),
                Err(_) => "error".into(),
            };
            rows.push((
                format!("{name} {label}"),
                match black {
                    Some(b) if close(Some(b), want, 1280, 720) => "found".into(),
                    Some(b) => format!("wrong {}x{}+{}+{}", b.width, b.height, b.x, b.y),
                    None => "-".into(),
                },
                close(flat, want, 1280, 720),
                flat.is_some(),
                fmt(before),
                fmt(after),
            ));
        }
        // the landscape clip in a white letterbox, where it leaves bars
        // worth the name (a 4:3 clip fills 960×720)
        let ih_of = |w: u32, h: u32| (960.0 * h as f64 / w as f64 / 2.0).round() as u32 * 2;
        if meta.width > meta.height
            && (720 - ih_of(meta.width, meta.height).min(720)) as f64 / 720.0
                >= 2.0 * dedupe::video::BAR_MIN_SHARE
        {
            let v = out.join(format!("{name}~letterbox_white.mp4"));
            if make(src, &v, "scale=960:-2,pad=960:720:0:(oh-ih)/2:white", false) {
                if let (Ok(m), Ok(om)) = (probe(&v), probe(src)) {
                    let ih = (960.0 * om.height as f64 / om.width as f64 / 2.0).round() as u32 * 2;
                    let want = (0, (720 - ih) / 2, 960, (720 - ih) / 2 + ih);
                    let black = detect_bars(&v, &m);
                    let flat = detect_flat_bars(&v, &m);
                    let shipped = dedupe::video::detect_any_bars(&v, &m);
                    if close(shipped, want, 960, 720) {
                        shipped_right += 1;
                    }
                    let osig = hash_video_inside(src, &om, None);
                    let before = hash_video_inside(&v, &m, None);
                    let after = hash_video_inside(&v, &m, shipped);
                    let fmt = |a: &Result<dedupe::video::VideoSig, _>| match (&osig, a) {
                        (Ok(o), Ok(s)) => {
                            let r = relate(o, s);
                            format!(
                                "{:.2} {}",
                                r.level2,
                                if r.confirmed() {
                                    "confirmed"
                                } else if r.similar() {
                                    "grouped"
                                } else {
                                    "apart"
                                }
                            )
                        }
                        _ => "error".into(),
                    };
                    rows.push((
                        format!("{name} letterbox white"),
                        if black.is_some() {
                            "found".into()
                        } else {
                            "-".into()
                        },
                        close(flat, want, 960, 720),
                        flat.is_some(),
                        fmt(&before),
                        fmt(&after),
                    ));
                }
            }
        }
    }
    eprintln!();
    eprintln!(
        "{:<44} {:<16} {:<8} {:<18} {:<18}",
        "framed copy", "black pass", "flat", "uncropped", "cropped"
    );
    for (n, b, ok, any, before, after) in &rows {
        let flat = if *ok {
            "found"
        } else if *any {
            "WRONG"
        } else {
            "-"
        };
        eprintln!("{n:<44} {b:<16} {flat:<8} {before:<18} {after:<18}");
    }
    let found = rows.iter().filter(|r| r.1 == "found" || r.2).count();
    eprintln!();
    eprintln!(
        "{} framed copies: bars found in {found}; {} originals and re-encodes, bars wrongly found in {}: {:?}",
        rows.len(),
        negatives,
        false_found.len(),
        false_found
    );
    eprintln!(
        "the shipped decision: the picture found in {shipped_right} of {} framed copies; bars in {} of {negatives} without: {shipped_false:?}",
        rows.len(),
        shipped_false.len()
    );
    details.sort_by(|a, b| a.1.total_cmp(&b.1));
    eprintln!(
        "black pass's band detail against the picture's (BLACK_DETAIL_MAX {}):",
        dedupe::video::BLACK_DETAIL_MAX
    );
    for (n, r) in &details {
        eprintln!("    {r:>6.3}  {n}");
    }
    eprintln!("total {:.0}s", t0.elapsed().as_secs_f64());
}
