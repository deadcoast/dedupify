//! Flat bars around still pictures (found on the owner's Windows run,
//! 2026-11-11: a letterboxed export kept its group over the original by
//! having more pixels). `near::picture_bars` applies the video tier's
//! flat-bar rule to one grey copy of a picture; the keeper rule then
//! counts the picture inside the bars.
//!
//! ```text
//! DEDUPE_PHOTO_DIRS=<folder>[:<folder>…] \
//!   cargo test --release --test image_bars -- --ignored --nocapture
//! ```
//!
//! For every picture in the folders (real photographs, which have no
//! bars): whether bars are found in it, and in copies of it boxed the
//! ways exports box a picture — letterboxed 4:5 in black, pillarboxed
//! 16:9 in white, a grey border — whether they are found within 2 % of
//! each side. Nothing here is a unit test; it prints a record for
//! VERIFICATION.md.

use std::path::PathBuf;

use image::{DynamicImage, Rgb, RgbImage};

use dedupe::near::picture_bars;

fn boxed(pic: &RgbImage, w: u32, h: u32, colour: [u8; 3]) -> RgbImage {
    let mut c = RgbImage::from_pixel(w, h, Rgb(colour));
    image::imageops::overlay(
        &mut c,
        pic,
        ((w - pic.width()) / 2) as i64,
        ((h - pic.height()) / 2) as i64,
    );
    c
}

#[test]
#[ignore]
fn bars_in_real_photographs_and_their_boxed_copies() {
    let Some(dirs) = std::env::var_os("DEDUPE_PHOTO_DIRS") else {
        eprintln!("DEDUPE_PHOTO_DIRS not set; skipped");
        return;
    };
    let mut files: Vec<PathBuf> = std::env::split_paths(&dirs)
        .flat_map(|d| {
            walkdir::WalkDir::new(d)
                .follow_links(true)
                .into_iter()
                .flatten()
                .filter(|e| e.file_type().is_file())
                .map(|e| e.into_path())
        })
        .filter(|p| {
            p.extension().is_some_and(|e| {
                matches!(
                    e.to_string_lossy().to_lowercase().as_str(),
                    "jpg" | "jpeg" | "png"
                )
            })
        })
        .collect();
    files.sort();
    let kinds = ["letterbox 4:5 black", "pillarbox 16:9 white", "border grey"];
    let (mut n, mut false_found) = (0, Vec::new());
    let mut found = [0usize; 3];
    let mut wrong = [0usize; 3];
    let mut examples: Vec<String> = Vec::new();
    let t0 = std::time::Instant::now();
    for f in &files {
        let Ok(d) = dedupe::decode::load(f) else {
            continue;
        };
        // a working size like an export's; the box is built around it
        let pic = d.img.thumbnail(1024, 1024).to_rgb8();
        let (w, h) = (pic.width(), pic.height());
        if w < 64 || h < 64 {
            continue;
        }
        n += 1;
        if let Some(b) = picture_bars(&DynamicImage::ImageRgb8(pic.clone())) {
            false_found.push((f.file_name().unwrap().to_string_lossy().into_owned(), b));
        }
        let lw = w.max(h * 4 / 5);
        let lh = (lw * 5 / 4).max(h + h / 10);
        let ph = h.max(w * 9 / 16);
        let pw = (ph * 16 / 9).max(w + w / 10);
        let copies = [
            (boxed(&pic, w, lh, [0, 0, 0]), (w, lh)),
            (boxed(&pic, pw, h, [255, 255, 255]), (pw, h)),
            (
                boxed(&pic, w + w / 6, h + h / 6, [128, 128, 128]),
                (w + w / 6, h + h / 6),
            ),
        ];
        for (i, (c, (cw, ch))) in copies.iter().enumerate() {
            let want = [
                ((cw - w) / 2) as f32 / *cw as f32,
                ((ch - h) / 2) as f32 / *ch as f32,
                ((cw - w) / 2 + w) as f32 / *cw as f32,
                ((ch - h) / 2 + h) as f32 / *ch as f32,
            ];
            match picture_bars(&DynamicImage::ImageRgb8(c.clone())) {
                Some(b) if b.iter().zip(want).all(|(g, e)| (g - e).abs() <= 0.02) => found[i] += 1,
                Some(b) => {
                    wrong[i] += 1;
                    if examples.iter().filter(|e| e.starts_with(kinds[i])).count() < 3 {
                        examples.push(format!(
                            "{}: {} {w}x{h} in {cw}x{ch}: found {b:.3?}, want {want:.3?}",
                            kinds[i],
                            f.file_name().unwrap().to_string_lossy()
                        ));
                    }
                }
                None => {}
            }
        }
    }
    eprintln!();
    eprintln!("{n} photographs ({:.0} s)", t0.elapsed().as_secs_f64());
    eprintln!(
        "bars found in the photographs themselves: {}  {:?}",
        false_found.len(),
        &false_found[..false_found.len().min(12)]
    );
    for e in &examples {
        eprintln!("  {e}");
    }
    for (i, k) in kinds.iter().enumerate() {
        eprintln!(
            "{k:<22} found {:>4} of {n} ({:.1} %), wrong place {}",
            found[i],
            100.0 * found[i] as f64 / n.max(1) as f64,
            wrong[i]
        );
    }
}
