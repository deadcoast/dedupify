//! The fit's scores on files, written out so two builds can be compared
//! (Phase 14, CORE-15: the coarse search on summed-area tables, the small
//! JPEG decode, one keeper thumbnail per group — every score within 0.005,
//! no decision changed).
//!
//! ```text
//! DEDUPE_FIT_OUT=<file> cargo test --release --test fit_compare -- --ignored --nocapture
//! ```
//!
//! The pairs: 60 procedural photographs (640–1200 px, JPEG 90) against a
//! half-size copy, a centre crop to 70 %, a 4° rotation, a letterbox, a
//! squeeze, a watermark and a quality-50 re-encode, each written as a
//! JPEG; and each photograph against the next as a stranger.

mod common;
use common::*;

use std::io::Write;
use std::path::Path;

fn save(p: &Path, img: &image::RgbImage) {
    let mut v = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut v, 90)
        .encode_image(img)
        .unwrap();
    std::fs::write(p, v).unwrap();
}

#[test]
#[ignore]
fn scores_to_a_file() {
    let Some(out) = std::env::var_os("DEDUPE_FIT_OUT") else {
        eprintln!("DEDUPE_FIT_OUT not set");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    let variants: [(&str, fn(&image::RgbImage) -> image::RgbImage); 7] = [
        ("half", |i| scale(i, 0.5)),
        ("crop70", |i| crop_center(i, 0.7)),
        ("rot4", |i| rotate_small(i, 4.0)),
        ("letterbox", letterbox),
        ("squeeze", squeeze),
        ("watermark", watermark),
        ("q50", |i| jpeg(i, 50)),
    ];
    let mut pairs = Vec::new();
    for s in 0..60u64 {
        let p = photo(1000 + s);
        let a = d.join(format!("p{s:02}.jpg"));
        save(&a, &p);
        for (name, f) in &variants {
            let b = d.join(format!("p{s:02}_{name}.jpg"));
            save(&b, &f(&p));
            pairs.push((a.clone(), b));
        }
    }
    for s in 0..59u64 {
        pairs.push((d.join(format!("p{s:02}.jpg")), d.join(format!("p{:02}.jpg", s + 1))));
    }
    let mut f = std::fs::File::create(out).unwrap();
    for (a, b) in &pairs {
        let fit = dedupe::geometry::fit_files(a, b).unwrap();
        writeln!(
            f,
            "{}\t{}\t{:.6}\t{:.6}\t{}",
            a.file_name().unwrap().to_string_lossy(),
            b.file_name().unwrap().to_string_lossy(),
            fit.ncc,
            fit.robust,
            fit.confirmed()
        )
        .unwrap();
    }
    eprintln!("{} pairs", pairs.len());
}
