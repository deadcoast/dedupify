//! Flat bars around a still picture, and the picture's own pixels for the
//! keeper rule. (Split from `near.rs`, Phase 14, SC-09.)

use super::*;

/// Bars around a still picture: rows at the top and bottom, or columns at
/// the left and right, all of one colour, on one grey copy `FLAT_WIDTH`
/// wide. The left, top, right and bottom of the picture as shares of the
/// width and height. [CLAUDE, found on the owner's Windows run: a
/// letterboxed export kept its group over the original, having more
/// pixels]
///
/// An export pads with one colour, exactly, so a line is a bar when
/// `STILL_BAR_SHARE` of its pixels sit within `STILL_BAR_TOL` grey levels
/// of the outermost line's colour, and both ends carry such bands of one
/// colour, of about one depth (2 % of the side), leaving the picture a
/// quarter of the side or more (`video::BAR_MIN_SHARE`, `BAR_KEEP_MIN`).
/// The video tier's rule (detail along the band, and a picture that
/// starts at once) was tried first and missed half of the letterboxed
/// photographs whose sky meets the bar (VERIFICATION § Phase 12).
pub fn picture_bars(img: &image::DynamicImage) -> Option<[f32; 4]> {
    let w = crate::video::FLAT_WIDTH;
    if img.width() < 16 || img.height() < 16 {
        return None;
    }
    let g = if img.width() > w {
        img.thumbnail(w, u32::MAX).to_luma8()
    } else {
        img.to_luma8()
    };
    let (gw, gh) = (g.width() as usize, g.height() as usize);
    let px = g.as_raw();
    let rows: Vec<Vec<u8>> = (0..gh).map(|y| px[y * gw..(y + 1) * gw].to_vec()).collect();
    let cols: Vec<Vec<u8>> = (0..gw)
        .map(|x| (0..gh).map(|y| px[y * gw + x]).collect())
        .collect();
    let x = uniform_band(&cols);
    let y = uniform_band(&rows);
    if x.is_none() && y.is_none() {
        return None;
    }
    let (l, r) = x.map_or((0.0, 1.0), |(a, b)| {
        (a as f32 / gw as f32, 1.0 - b as f32 / gw as f32)
    });
    let (t, b) = y.map_or((0.0, 1.0), |(a, b)| {
        (a as f32 / gh as f32, 1.0 - b as f32 / gh as f32)
    });
    Some([l, t, r, b])
}

/// Grey levels a bar's pixel may stray from the bar's colour (a JPEG's
/// noise, the thumbnail's averaging at the picture's edge).
pub const STILL_BAR_TOL: i32 = 8;

/// The share of a line's pixels that must be the bar's colour.
pub const STILL_BAR_SHARE: f64 = 0.97;

/// The depth of the bands of one colour at both ends of `lines`, when
/// there are such bands and they make bars.
fn uniform_band(lines: &[Vec<u8>]) -> Option<(usize, usize)> {
    let n = lines.len();
    if n < 8 {
        return None;
    }
    let colour = |l: &Vec<u8>| {
        let mut v = l.clone();
        v.sort_unstable();
        v[v.len() / 2] as i32
    };
    let is_bar = |l: &Vec<u8>, c: i32| {
        let near = l
            .iter()
            .filter(|&&v| (v as i32 - c).abs() <= STILL_BAR_TOL)
            .count();
        near as f64 >= STILL_BAR_SHARE * l.len() as f64
    };
    let (c0, c1) = (colour(&lines[0]), colour(&lines[n - 1]));
    if (c0 - c1).abs() > STILL_BAR_TOL {
        return None;
    }
    let lead = lines.iter().take_while(|l| is_bar(l, c0)).count();
    let trail = lines.iter().rev().take_while(|l| is_bar(l, c1)).count();
    let min = ((crate::video::BAR_MIN_SHARE * n as f64).ceil() as usize).max(2);
    if lead < min || trail < min || lead + trail >= n {
        return None;
    }
    if lead.abs_diff(trail) > min {
        return None;
    }
    if ((n - lead - trail) as f64) < crate::video::BAR_KEEP_MIN * n as f64 {
        return None;
    }
    Some((lead, trail))
}

/// The picture's own pixels, for "most pixels": inside its flat bars when
/// it has any, and then a fiftieth less, so a padded copy outranks the
/// picture it was padded from only with clearly more picture (its bars
/// are found at 256 wide, a line either way). [CLAUDE]
pub fn picture_pixels(h: &Hashed) -> f64 {
    let px = h.sig.pixels() as f64;
    match h.bars {
        Some([l, t, r, b]) => px * ((r - l) * (b - t)) as f64 * 0.98,
        None => px,
    }
}

#[cfg(test)]
mod bars_tests {
    use super::*;
    use image::{DynamicImage, Rgb, RgbImage};

    /// A textured picture: rings and stripes, detail everywhere.
    fn picture(w: u32, h: u32) -> RgbImage {
        RgbImage::from_fn(w, h, |x, y| {
            let (fx, fy) = (x as f32 / w as f32, y as f32 / h as f32);
            let v = 128.0
                + 60.0 * (fx * 41.0).sin() * (fy * 29.0).cos()
                + 50.0 * ((fx - 0.4).hypot(fy - 0.6) * 90.0).sin();
            let c = v.clamp(0.0, 255.0) as u8;
            Rgb([c, c / 2 + 40, 255 - c])
        })
    }

    /// `pic` centred on a canvas of `w × h` in `colour`.
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

    fn hashed(name: &str, img: &RgbImage) -> Hashed {
        let d = DynamicImage::ImageRgb8(img.clone());
        Hashed {
            rep: Rep {
                cand: crate::walk::Candidate {
                    path: std::path::PathBuf::from(name),
                    size: 1000,
                    mtime: std::time::SystemTime::UNIX_EPOCH,
                },
                exact_group: None,
            },
            sig: perceptual::signature_from_image(&d),
            deep: None,
            capture: None,
            raw: false,
            bars: picture_bars(&d),
        }
    }

    #[test]
    fn a_letterboxed_copy_never_outranks_its_picture() {
        let pic = picture(512, 512);
        assert_eq!(picture_bars(&DynamicImage::ImageRgb8(pic.clone())), None);
        // black, white and grey bars, above and below or left and right
        for (w, h, colour) in [
            (512, 640, [0, 0, 0]),
            (512, 640, [255, 255, 255]),
            (680, 512, [128, 128, 128]),
        ] {
            let b = boxed(&pic, w, h, colour);
            let [l, t, r, bt] =
                picture_bars(&DynamicImage::ImageRgb8(b.clone())).expect("bars found");
            let near = |a: f32, e: f32| (a - e).abs() < 0.02;
            assert!(
                near(l, (w - 512) as f32 / 2.0 / w as f32)
                    && near(t, (h - 512) as f32 / 2.0 / h as f32)
                    && near(r, 1.0 - (w - 512) as f32 / 2.0 / w as f32)
                    && near(bt, 1.0 - (h - 512) as f32 / 2.0 / h as f32),
                "{w}x{h}: {l} {t} {r} {bt}"
            );
            // the original keeps, whichever is older or named which way
            let (o, c) = (hashed("photos/pic.jpg", &pic), hashed("a/pic_box.png", &b));
            assert_eq!(keeper_order(&o, &c), std::cmp::Ordering::Less, "{w}x{h}");
            assert_eq!(keeper_order(&c, &o), std::cmp::Ordering::Greater, "{w}x{h}");
            // and the member is said to be what it is
            let want = if h > w { Boxed::Letter } else { Boxed::Pillar };
            assert_eq!(relation_of(&o, &c).boxed, want, "{w}x{h}");
            assert_eq!(relation_of(&c, &o).boxed, Boxed::Removed);
        }
        // more picture inside the bars than the bare file has: it keeps
        let big = image::imageops::resize(&pic, 1024, 1024, image::imageops::FilterType::Triangle);
        let up = hashed("up.png", &boxed(&big, 1024, 1280, [0, 0, 0]));
        let o = hashed("pic.jpg", &pic);
        assert_eq!(keeper_order(&up, &o), std::cmp::Ordering::Less);
        // a picture with a flat sky on one side only has no bars
        let mut sky = pic.clone();
        for y in 0..120 {
            for x in 0..512 {
                sky.put_pixel(x, y, Rgb([170, 200, 230]));
            }
        }
        assert_eq!(picture_bars(&DynamicImage::ImageRgb8(sky.clone())), None);
        // and boxed, the bar ends where the sky starts
        let [_, t, _, b] = picture_bars(&DynamicImage::ImageRgb8(boxed(&sky, 512, 640, [0, 0, 0])))
            .expect("bars above a sky");
        assert!((t - 0.1).abs() < 0.02 && (b - 0.9).abs() < 0.02, "{t} {b}");
    }

    #[test]
    fn the_bars_say_what_was_added_or_removed() {
        let k = (512, 512);
        let mut r = relation_deep(
            &perceptual::signature_from_image(&DynamicImage::ImageRgb8(picture(64, 64))),
            &perceptual::signature_from_image(&DynamicImage::ImageRgb8(picture(64, 64))),
            None,
            false,
        );
        for (b, w, m) in [
            (Boxed::Letter, "letterboxed", (512, 640)),
            (Boxed::Pillar, "pillarboxed", (680, 512)),
            (Boxed::Border, "with a border", (600, 600)),
            (Boxed::Removed, "bars removed", (512, 410)),
        ] {
            r.boxed = b;
            assert_eq!(r.words_for(k, m, None, None), w);
        }
    }
}
