//! Where decode time goes for one image: decode, prefilter, grid, pdq.
use std::time::Instant;
fn main() {
    let path = std::env::args().nth(1).unwrap();
    let n = 10;
    let t = Instant::now();
    let mut img = None;
    for _ in 0..n {
        img = Some(image::open(&path).unwrap());
    }
    let img = img.unwrap();
    println!(
        "decode        {:>7.2} ms",
        t.elapsed().as_secs_f64() * 1000.0 / n as f64
    );
    let t = Instant::now();
    let mut pre = None;
    for _ in 0..n {
        pre = Some(dedupe::perceptual::prefiltered_luma(&img));
    }
    let (pre, pw, ph) = pre.unwrap();
    println!(
        "prefilter     {:>7.2} ms   ({pw}x{ph})",
        t.elapsed().as_secs_f64() * 1000.0 / n as f64
    );
    let t = Instant::now();
    for _ in 0..n {
        let _ = dedupe::perceptual::grid_from_luma(&pre, pw, ph);
    }
    println!(
        "grid lanczos  {:>7.2} ms",
        t.elapsed().as_secs_f64() * 1000.0 / n as f64
    );
    let t = Instant::now();
    for _ in 0..n {
        let _ = dedupe::pdq::downsample(&dedupe::pdq::Luma {
            data: &pre,
            width: pw,
            height: ph,
        });
    }
    println!(
        "pdq jarosz    {:>7.2} ms",
        t.elapsed().as_secs_f64() * 1000.0 / n as f64
    );
    let t = Instant::now();
    for _ in 0..n {
        let _ = dedupe::pdq::compute(&dedupe::pdq::Luma {
            data: &pre,
            width: pw,
            height: ph,
        });
    }
    println!(
        "pdq total     {:>7.2} ms",
        t.elapsed().as_secs_f64() * 1000.0 / n as f64
    );
    let grid = dedupe::perceptual::grid_from_luma(&pre, pw, ph);
    let t = Instant::now();
    for _ in 0..n {
        let _ = dedupe::perceptual::signature_from_grid(&grid, 1, 1);
    }
    println!(
        "dhash+phash×8 {:>7.2} ms",
        t.elapsed().as_secs_f64() * 1000.0 / n as f64
    );
}
