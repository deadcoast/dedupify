//! Writes a few heavy edits of one image next to it (for the deep tier run).
use image::imageops::FilterType;
fn main() {
    let src = std::env::args().nth(1).unwrap();
    let img = image::open(&src).unwrap().to_rgb8();
    let (w, h) = (img.width(), img.height());
    let dir = std::path::Path::new(&src).parent().unwrap();
    let crop60 = image::imageops::crop_imm(&img, w / 5, h / 5, w * 3 / 5, h * 3 / 5).to_image();
    crop60.save(dir.join("beach_crop60.jpg")).unwrap();
    let rot = image::imageops::rotate90(&image::imageops::resize(
        &img,
        w / 2,
        h / 2,
        FilterType::Triangle,
    ));
    rot.save(dir.join("beach_rot90_small.png")).unwrap();
    let mut lb = image::RgbImage::from_pixel(w, h * 5 / 4, image::Rgb([0, 0, 0]));
    image::imageops::overlay(&mut lb, &img, 0, (h / 8) as i64);
    lb.save(dir.join("beach_letterbox.png")).unwrap();
}
