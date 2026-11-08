//! Diagnostic: d/p/q of derivatives against a source image.
use dedupe::near::relation;
use dedupe::perceptual::signature_from_image;
use image::DynamicImage;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let src = image::open(&args[1]).unwrap();
    let k = signature_from_image(&src);
    println!(
        "source {}x{}  pdq quality {}",
        src.width(),
        src.height(),
        k.pdq_quality
    );
    let rgb = src.to_rgb8();
    let (w, h) = (rgb.width(), rgb.height());
    let mut cases: Vec<(String, DynamicImage)> = Vec::new();
    for keep in [0.98f32, 0.95, 0.92, 0.90] {
        let cw = (w as f32 * keep) as u32;
        let ch = (h as f32 * keep) as u32;
        let c = image::imageops::crop_imm(&rgb, (w - cw) / 2, (h - ch) / 2, cw, ch).to_image();
        cases.push((
            format!("crop {:.0}%", keep * 100.0),
            DynamicImage::ImageRgb8(c),
        ));
    }
    for (name, img) in cases {
        let m = signature_from_image(&img);
        let r = relation(&k, &m);
        println!(
            "{name:<10} d {:>2}  p {:>2}  q {:>3}  confirmed {}",
            r.d,
            r.p,
            r.q,
            r.confirmed()
        );
    }
}
