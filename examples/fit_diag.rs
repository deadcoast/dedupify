//! Diagnostic: d/p/q and the geometric fit of edits of a source image —
//! the edits a test fixture needs when it wants a pair the hashes group
//! but nothing confirms.
use dedupe::geometry::{fit, Thumb, MAX_SIDE};
use dedupe::near::relation;
use dedupe::perceptual::signature_from_image;
use image::{DynamicImage, Rgb, RgbImage};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let src = image::open(&args[1]).unwrap();
    let k = signature_from_image(&src);
    let kt = Thumb::from_image(&src, MAX_SIDE);
    let rgb = src.to_rgb8();
    let (w, h) = (rgb.width(), rgb.height());
    let mut cases: Vec<(String, RgbImage)> = Vec::new();
    for frac in [0.10f32, 0.15, 0.20, 0.25, 0.33] {
        // the lower strip replaced by a flat band (a caption bar)
        let mut e = rgb.clone();
        let y0 = (h as f32 * (1.0 - frac)) as u32;
        for y in y0..h {
            for x in 0..w {
                e.put_pixel(x, y, Rgb([40, 40, 48]));
            }
        }
        cases.push((format!("band {:.0}%", frac * 100.0), e));
        // the lower strip replaced by other texture
        let mut e2 = rgb.clone();
        for y in y0..h {
            for x in 0..w {
                let v = (((x * 7 + y * 13) % 97) as f32 * 2.6) as u8;
                e2.put_pixel(x, y, Rgb([v, v / 2 + 60, 255 - v]));
            }
        }
        cases.push((format!("texture {:.0}%", frac * 100.0), e2));
    }
    for (name, img) in cases {
        let d = DynamicImage::ImageRgb8(img);
        let m = signature_from_image(&d);
        let r = relation(&k, &m);
        let f = fit(&kt, &Thumb::from_image(&d, MAX_SIDE));
        println!(
            "{name:<12} d {:>2}  p {:>2}  q {:>3}  fit {:.3}   hash pair {}  pdq {}",
            r.d,
            r.p,
            r.q,
            f.ncc,
            r.hash_pair(),
            r.pdq_ok()
        );
    }
}
