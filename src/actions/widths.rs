//! SC-10 (Phase 16): every line of `find`'s result — the header, the
//! listing, the totals, the hints — rendered at every width from 40 to 200
//! cells on one scan of fixed groups (exact copies, near copies with their
//! evidence, a RAW with its camera JPEG and an export, videos with bars, a
//! long root and long names), and the stage lines at the same widths:
//! none may run past the edge, and the keepers' dates share one column.

use std::fs;
use std::path::Path;

use crate::render::Renderer;
use crate::theme::Theme;
use crate::util::visible_width;

fn put(root: &Path, rel: &str, bytes: &[u8]) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(&p, bytes).unwrap();
}

fn blob(seed: u8, len: usize) -> Vec<u8> {
    (0..len)
        .map(|i| ((i as u64 * 2_654_435_761u64) >> 7) as u8 ^ seed)
        .collect()
}

fn photo(seed: u32, w: u32, h: u32) -> image::RgbImage {
    let mut state = (seed as u64 * 0x9E37_79B9_7F4A_7C15) | 1;
    let mut rnd = move || {
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        (state.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40) as f32 / (1u64 << 24) as f32
    };
    // two octaves of smooth value noise plus a few shapes
    let cells = [4u32, 12];
    let grids: Vec<(u32, Vec<f32>)> = cells
        .iter()
        .map(|&c| (c, (0..(c + 1) * (c + 1)).map(|_| rnd()).collect()))
        .collect();
    let shapes: Vec<(f32, f32, f32, f32, [f32; 3])> = (0..4)
        .map(|_| {
            (
                rnd(),
                rnd(),
                0.1 + 0.2 * rnd(),
                0.1 + 0.2 * rnd(),
                [rnd(), rnd(), rnd()],
            )
        })
        .collect();
    let tint = [0.6 + 0.4 * rnd(), 0.6 + 0.4 * rnd(), 0.6 + 0.4 * rnd()];
    image::ImageBuffer::from_fn(w, h, |x, y| {
        let fx = x as f32 / w as f32;
        let fy = y as f32 / h as f32;
        let mut v = 0.0;
        let mut amp = 1.0;
        let mut tot = 0.0;
        for (c, g) in &grids {
            let gx = fx * *c as f32;
            let gy = fy * *c as f32;
            let (x0, y0) = (gx.floor() as u32, gy.floor() as u32);
            let (tx, ty) = (gx - x0 as f32, gy - y0 as f32);
            let at = |xx: u32, yy: u32| g[(yy.min(*c) * (c + 1) + xx.min(*c)) as usize];
            v += amp
                * (at(x0, y0) * (1.0 - tx) * (1.0 - ty)
                    + at(x0 + 1, y0) * tx * (1.0 - ty)
                    + at(x0, y0 + 1) * (1.0 - tx) * ty
                    + at(x0 + 1, y0 + 1) * tx * ty);
            tot += amp;
            amp *= 0.5;
        }
        v /= tot;
        let mut px = [v * tint[0], v * tint[1], v * tint[2]];
        for (cx, cy, rx, ry, col) in &shapes {
            let dx = (fx - cx) / rx;
            let dy = (fy - cy) / ry;
            let d = dx * dx + dy * dy;
            if d < 1.0 {
                let a = 0.7 * (1.0 - d).sqrt();
                for k in 0..3 {
                    px[k] = px[k] * (1.0 - a) + col[k] * a;
                }
            }
        }
        image::Rgb([
            (px[0] * 255.0) as u8,
            (px[1] * 255.0) as u8,
            (px[2] * 255.0) as u8,
        ])
    })
}

fn png_bytes(img: &image::RgbImage) -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(img.clone())
        .write_to(&mut buf, image::ImageFormat::Png)
        .unwrap();
    buf.into_inner()
}

fn jpeg_bytes(img: &image::RgbImage, q: u8) -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, q)
        .encode_image(img)
        .unwrap();
    buf.into_inner()
}

/// The same JPEG with an EXIF APP1 segment carrying `DateTimeOriginal`
/// (the camera's exposure time) inserted after the SOI marker.
fn with_capture_time(jpeg: &[u8], datetime: &str) -> Vec<u8> {
    assert_eq!(&jpeg[..2], &[0xFF, 0xD8]);
    let mut tiff: Vec<u8> = Vec::new();
    tiff.extend_from_slice(b"MM\x00\x2A\x00\x00\x00\x08");
    let exif_ifd_off: u32 = 8 + 2 + 12 + 4;
    tiff.extend_from_slice(&1u16.to_be_bytes());
    tiff.extend_from_slice(&0x8769u16.to_be_bytes());
    tiff.extend_from_slice(&4u16.to_be_bytes());
    tiff.extend_from_slice(&1u32.to_be_bytes());
    tiff.extend_from_slice(&exif_ifd_off.to_be_bytes());
    tiff.extend_from_slice(&0u32.to_be_bytes());
    let dt = format!("{datetime}\0");
    let data_off = exif_ifd_off + 2 + 12 + 4;
    tiff.extend_from_slice(&1u16.to_be_bytes());
    tiff.extend_from_slice(&0x9003u16.to_be_bytes());
    tiff.extend_from_slice(&2u16.to_be_bytes());
    tiff.extend_from_slice(&(dt.len() as u32).to_be_bytes());
    tiff.extend_from_slice(&data_off.to_be_bytes());
    tiff.extend_from_slice(&0u32.to_be_bytes());
    tiff.extend_from_slice(dt.as_bytes());
    let mut app1 = b"Exif\0\0".to_vec();
    app1.extend_from_slice(&tiff);
    let mut out = vec![0xFF, 0xD8, 0xFF, 0xE1];
    out.extend_from_slice(&((app1.len() + 2) as u16).to_be_bytes());
    out.extend_from_slice(&app1);
    out.extend_from_slice(&jpeg[2..]);
    out
}

/// A little-endian TIFF the way a camera writes a RAW: IFD0 with the
/// orientation, the embedded preview's offset and length, and an Exif IFD
/// with the capture moment; then bytes standing for sensor data; then the
/// preview JPEG.
fn raw_file(preview: &[u8], datetime: &str, orientation: u16) -> Vec<u8> {
    let le16 = |v: u16| v.to_le_bytes();
    let le32 = |v: u32| v.to_le_bytes();
    let entry = |t: &mut Vec<u8>, tag: u16, typ: u16, count: u32, value: [u8; 4]| {
        t.extend_from_slice(&le16(tag));
        t.extend_from_slice(&le16(typ));
        t.extend_from_slice(&le32(count));
        t.extend_from_slice(&value);
    };
    let ifd0 = 8u32;
    let ifd0_len = 2 + 4 * 12 + 4;
    let exif = ifd0 + ifd0_len;
    let exif_len = 2 + 12 + 4;
    let dt = format!("{datetime}\0");
    let dt_off = exif + exif_len;
    let sensor_off = dt_off + dt.len() as u32;
    let sensor = blob(77, 40_000);
    let preview_off = sensor_off + sensor.len() as u32;
    let mut t = b"II*\0".to_vec();
    t.extend_from_slice(&le32(ifd0));
    t.extend_from_slice(&le16(4));
    let mut short = [0u8; 4];
    short[..2].copy_from_slice(&le16(orientation));
    entry(&mut t, 0x0112, 3, 1, short);
    entry(&mut t, 0x0201, 4, 1, le32(preview_off));
    entry(&mut t, 0x0202, 4, 1, le32(preview.len() as u32));
    entry(&mut t, 0x8769, 4, 1, le32(exif));
    t.extend_from_slice(&le32(0));
    t.extend_from_slice(&le16(1));
    entry(&mut t, 0x9003, 2, dt.len() as u32, le32(dt_off));
    t.extend_from_slice(&le32(0));
    t.extend_from_slice(dt.as_bytes());
    t.extend_from_slice(&sensor);
    t.extend_from_slice(preview);
    t
}

/// Two short clips, one with black bars, through ffmpeg; `false` when
/// ffmpeg is not there.
fn videos(root: &Path) -> bool {
    if crate::tools::ffmpeg_at().is_none() {
        return false;
    }
    let dir = root.join("video clips from the summer trip");
    fs::create_dir_all(&dir).unwrap();
    let make = |name: &str, vf: &str| {
        crate::tools::ffmpeg()
            .args([
                "-nostdin",
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
            ])
            .arg("testsrc2=size=320x240:rate=15:duration=3")
            .args([
                "-vf",
                vf,
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-pix_fmt",
                "yuv420p",
                "-y",
            ])
            .arg(dir.join(name))
            .status()
            .is_ok_and(|s| s.success())
    };
    make("beach day.mp4", "null")
        && make(
            "beach day (letterboxed for the phone, a long name).mp4",
            "scale=320:180,pad=320:240:0:30:black",
        )
}

fn fixture(root: &Path) -> bool {
    let shot = photo(7, 900, 600);
    put(root, "photos/beach.png", &png_bytes(&shot));
    put(root, "photos/export/beach.jpg", &jpeg_bytes(&shot, 90));
    put(
        root,
        "photos/export/beach (copy).jpg",
        &jpeg_bytes(&shot, 90),
    );
    let half = image::imageops::resize(&shot, 450, 300, image::imageops::FilterType::Triangle);
    put(
        root,
        "photos/small/beach_small_for_the_web_and_email.jpg",
        &jpeg_bytes(&half, 80),
    );
    put(
        root,
        "photos/export/beach_mirror.png",
        &png_bytes(&image::imageops::flip_horizontal(&shot)),
    );
    let mut bar = shot.clone();
    for y in 480..600 {
        for x in 0..900 {
            bar.put_pixel(x, y, image::Rgb([240, 240, 240]));
        }
    }
    put(root, "photos/beach_bar.png", &png_bytes(&bar));
    let other = photo(41, 1200, 800);
    put(
        root,
        "dcim/IMG_0041.CR2",
        &raw_file(&jpeg_bytes(&other, 92), "2022:06:10 18:17:40", 1),
    );
    put(
        root,
        "dcim/IMG_0041.JPG",
        &with_capture_time(&jpeg_bytes(&other, 95), "2022:06:10 18:17:40"),
    );
    let web = image::imageops::resize(&other, 600, 400, image::imageops::FilterType::Triangle);
    put(
        root,
        "web/holiday.jpg",
        &with_capture_time(&jpeg_bytes(&web, 80), "2022:06:10 18:17:40"),
    );
    put(root, "docs/report.pdf", &blob(3, 90_000));
    put(root, "docs/old/report (1).pdf", &blob(3, 90_000));
    videos(root)
}

/// The column a keeper row's date starts at, if the row ends in one.
fn date_col(line: &str) -> Option<usize> {
    let t = line.trim_end();
    let tail = t.get(t.len().checked_sub(10)?..)?;
    let b = tail.as_bytes();
    let is_date = b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit());
    is_date.then(|| visible_width(t) - 10)
}

#[test]
fn every_line_fits_from_40_to_200_cells() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp
        .path()
        .join("a library folder with a long name that a header must shorten")
        .join("Pictures and Videos 2019-2024 (backup from the old laptop)");
    fs::create_dir_all(&root).unwrap();
    let with_video = fixture(&root);
    let args: Vec<String> = ["find", "."].iter().map(|s| s.to_string()).collect();
    let inv = crate::cli::parse(&args, &root).unwrap();
    let r = Renderer::new(Theme::custom(false, true, 200, false));
    let mut found = None;
    let _ = r.capture(200, |r| found = Some(super::scan::scan_and_group(&inv, r)));
    let found = found.unwrap();
    assert!(
        !found.groups.is_empty() && !found.near.is_empty(),
        "the fixture groups"
    );
    assert!(!with_video || !found.video.is_empty(), "the videos group");
    let mut over: Vec<String> = Vec::new();
    let mut cols: Vec<String> = Vec::new();
    for w in 40..=200usize {
        let lines = r.capture(w, |r| {
            r.header(&inv);
            super::scan::report(&found, &inv, r);
            r.next_after_find(&inv);
        });
        for l in &lines {
            let n = visible_width(l);
            if n > w {
                over.push(format!("{w:>3}: {n:>3}  {l}"));
            }
        }
        let dates: Vec<usize> = lines
            .iter()
            .filter(|l| l.trim_start().starts_with("keep"))
            .filter_map(|l| date_col(l))
            .collect();
        if dates.windows(2).any(|p| p[0] != p[1]) {
            cols.push(format!("{w}: {dates:?}"));
        }
        for (stage, text) in crate::render::sample_stage_texts() {
            let l = crate::render::stage_line(&r.theme, w, stage, &text);
            let n = visible_width(&l);
            if n > w {
                over.push(format!("{w:>3}: {n:>3}  {l}"));
            }
        }
    }
    assert!(
        over.is_empty(),
        "{} lines past the edge:\n{}",
        over.len(),
        over.join("\n")
    );
    assert!(
        cols.is_empty(),
        "keepers' dates in different columns:\n{}",
        cols.join("\n")
    );
}
