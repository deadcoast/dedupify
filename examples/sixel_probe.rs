//! The sixel encoder on one picture: the picture as fitted, and the sixel
//! it becomes, for decoding and comparing (Phase 19 check) [CLAUDE].
//! usage: sixel_probe <image> <max_w> <max_h> <out_prefix>
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let img = image::open(&a[1]).expect("image");
    let (w, h): (u32, u32) = (a[2].parse().unwrap(), a[3].parse().unwrap());
    let fitted = dedupe::preview::fit_pixels_sharp(&img, w, h);
    fitted.save(format!("{}_fitted.png", a[4])).unwrap();
    let t = std::time::Instant::now();
    let s = dedupe::preview::sixel(&fitted);
    eprintln!(
        "{}x{} → {} bytes in {:?}",
        fitted.width(),
        fitted.height(),
        s.len(),
        t.elapsed()
    );
    std::fs::write(format!("{}.six", a[4]), s).unwrap();
}
