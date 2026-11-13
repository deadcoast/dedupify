//! TECH-14 (Phase 15): the ONNX export of SSCD against Meta's TorchScript
//! original, and our engine (`tract`) against ONNX Runtime — the Rust side.
//! `.github/workflows/sscd.yml` runs it, then `tests/sscd_agreement.py`.
//!
//! ```text
//! DEDUPE_MODEL=<sscd_disc_mixup.onnx> DEDUPE_SSCD_OUT=<folder> \
//!   cargo test --release --test sscd_agreement -- --ignored --nocapture
//! ```
//!
//! Writes 200 pictures at exactly the model's 320 × 320 — 100 procedural
//! photographs and a perturbed copy of each (crop, rotation, brightness,
//! JPEG, mirror, by turns) — so every engine is fed the same pixels with
//! no resize, and `ours.tsv`: each picture's name and the embedding the
//! deep tier computes for it.
#![cfg(feature = "deep")]

mod common;
use common::*;

use std::io::Write;

#[test]
#[ignore]
fn embeddings_for_the_agreement_check() {
    let (Some(model), Some(out)) = (
        dedupe::deep::installed(),
        std::env::var_os("DEDUPE_SSCD_OUT").map(std::path::PathBuf::from),
    ) else {
        eprintln!("DEDUPE_MODEL / DEDUPE_SSCD_OUT not set; skipped");
        return;
    };
    let deep = dedupe::deep::Deep::load(&model).unwrap();
    std::fs::create_dir_all(&out).unwrap();
    let side = dedupe::deep::SIZE as u32;
    let fit = |img: &image::RgbImage| {
        image::imageops::resize(img, side, side, image::imageops::FilterType::Triangle)
    };
    let mut pics: Vec<(String, image::RgbImage)> = Vec::new();
    for s in 0..100u64 {
        let p = photo(5000 + s);
        let v = match s % 5 {
            0 => crop_center(&p, 0.8),
            1 => rotate_small(&p, 5.0),
            2 => brightness(&p, 1.3),
            3 => jpeg(&p, 50),
            _ => image::imageops::flip_horizontal(&p),
        };
        pics.push((format!("p{s:03}"), fit(&p)));
        pics.push((format!("v{s:03}"), fit(&v)));
    }
    // the deep tier resizes to 320 × 320 first: at that size it must be
    // the identity, or the engines would not see the same tensor
    let identity = pics.iter().all(|(_, img)| {
        image::imageops::resize(img, side, side, image::imageops::FilterType::Triangle) == *img
    });
    assert!(identity, "the resize at 320 changed a picture");
    let mut tsv = std::io::BufWriter::new(std::fs::File::create(out.join("ours.tsv")).unwrap());
    for (name, img) in &pics {
        img.save(out.join(format!("{name}.png"))).unwrap();
        let e = deep
            .embed(&image::DynamicImage::ImageRgb8(img.clone()))
            .unwrap();
        let row: Vec<String> = e.iter().map(|v| format!("{v:.9}")).collect();
        writeln!(tsv, "{name}\t{}", row.join("\t")).unwrap();
    }
    eprintln!(
        "{} pictures and their embeddings in {}",
        pics.len(),
        out.display()
    );
}
