//! Where the near pass's per-file time goes, on files as the pass gets
//! them: decode (decode::load), the signature, the bars, the capture
//! read, the geometry thumb. Single-threaded, each file once, page cache warm.
use std::time::{Duration, Instant};
fn main() {
    let files: Vec<String> = std::env::args().skip(1).collect();
    let mut t = [Duration::ZERO; 5];
    for f in &files {
        let p = std::path::Path::new(f);
        let _ = std::fs::read(p);
        let s = Instant::now();
        let d = dedupe::decode::load(p).unwrap();
        t[0] += s.elapsed();
        let s = Instant::now();
        let _sig = dedupe::perceptual::signature_from_image(&d.img);
        t[1] += s.elapsed();
        let s = Instant::now();
        let _b = dedupe::near::picture_bars(&d.img);
        t[2] += s.elapsed();
        let s = Instant::now();
        let _c = dedupe::capture::read(p);
        t[3] += s.elapsed();
        let s = Instant::now();
        let _g = dedupe::geometry::thumb_of(p);
        t[4] += s.elapsed();
    }
    let n = files.len() as f64;
    for (name, d) in ["decode", "signature", "bars", "capture", "fit thumb"]
        .iter()
        .zip(t)
    {
        println!("{name:<10} {:>7.2} ms a file", d.as_secs_f64() * 1000.0 / n);
    }
}
