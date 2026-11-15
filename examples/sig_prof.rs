//! The signature's parts, a file at a time: prefilter, grid, PDQ (the
//! fixed working plane), the 64-bit hashes.
use std::time::{Duration, Instant};
fn main() {
    let files: Vec<String> = std::env::args().skip(1).collect();
    let mut t = [Duration::ZERO; 4];
    for f in &files {
        let d = dedupe::decode::load(std::path::Path::new(f)).unwrap();
        let s = Instant::now();
        let (pre, pw, ph) = dedupe::perceptual::prefiltered_luma(&d.img);
        t[0] += s.elapsed();
        let s = Instant::now();
        let grid = dedupe::perceptual::grid_from_luma(&pre, pw, ph);
        t[1] += s.elapsed();
        let s = Instant::now();
        let _p = dedupe::pdq::compute_fixed(
            &dedupe::pdq::Luma {
                data: &pre,
                width: pw,
                height: ph,
            },
            image::imageops::FilterType::Triangle,
            dedupe::pdq::WORK_SIDE,
        );
        t[2] += s.elapsed();
        let s = Instant::now();
        let _h = dedupe::perceptual::signature_from_grid(&grid, 1, 1);
        t[3] += s.elapsed();
    }
    let n = files.len() as f64;
    for (name, d) in ["prefilter", "grid", "pdq", "dhash+phash"].iter().zip(t) {
        println!("{name:<12} {:>7.2} ms a file", d.as_secs_f64() * 1000.0 / n);
    }
}
