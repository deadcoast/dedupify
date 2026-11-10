//! TECH-02 (Phase 11): decoding a large JPEG at 1/2, 1/4 or 1/8 inside its
//! inverse DCT for the hashes, against decoding it in full.
//!
//! ```text
//! DEDUPE_JPEG_DIRS=<folder>[:<folder>…] \
//!   cargo test --release --test decode_reduced -- --ignored --nocapture
//! ```
//!
//! `reduced_jpeg_route_against_full`, for every JPEG the reduced route
//! takes: the distance between the two routes' hashes of the same file
//! (dHash, pHash, PDQ on the upright picture), whether the full size and
//! the capture moment agree, the decode-and-hash time of each (file in
//! the page cache, each route run twice in turn, the faster kept), and
//! the pixels each holds. `reduced_route_on_copies`: the robustness table
//! through each route (below).
//!
//! The route lives here and not in the program: measured on real phone
//! and camera JPEGs it was 1.3× faster at 12 megapixels and no faster
//! under 9, its PDQ of a file moved up to 22 bits from the full decode's,
//! and against a library's copies it gained no margin — a half-size copy
//! came closer, a full-size PNG export went from 0 bits to 22
//! (VERIFICATION § Phase 11, TECH-02). Nothing here is a unit test; it
//! prints a record for VERIFICATION.md.

use std::path::{Path, PathBuf};
use std::time::Instant;

use image::metadata::Orientation;
use image::{DynamicImage, RgbImage};

use dedupe::decode::{load, DecodeError, Decoded, Route};
use dedupe::pdq::{distance as pdq_distance, PDQ_MAX};
use dedupe::perceptual::{hamming, signature_from_image, signature_full, Signature, GRID};

/// The route measured here (TECH-02, Phase 11; not shipped — see the
/// module doc). Decode `path` upright for the hashes. A JPEG whose
/// short side is at least twice `short_min` is decoded at 1/2, 1/4 or 1/8
/// inside its inverse DCT — the largest of those that keeps the short side
/// at or above `short_min` — so the full picture is never built; every
/// other file goes through `load`. Returns the picture and the full
/// picture's upright size (what the result shows as its dimensions).
fn load_reduced(path: &Path, short_min: u32) -> Result<(Decoded, (u32, u32)), DecodeError> {
    if let Some(r) = jpeg_reduced(path, short_min) {
        return Ok(r);
    }
    let d = load(path)?;
    let full = (d.img.width(), d.img.height());
    Ok((d, full))
}

/// The reduced JPEG route; `None` sends the file to `load` (not a JPEG,
/// too small to gain, CMYK or 16-bit samples, a coding this decoder does
/// not read, any error).
fn jpeg_reduced(path: &Path, short_min: u32) -> Option<(Decoded, (u32, u32))> {
    use jpeg_decoder::PixelFormat;
    let f = std::fs::File::open(path).ok()?;
    let mut dec = jpeg_decoder::Decoder::new(std::io::BufReader::new(f));
    dec.read_info().ok()?;
    let info = dec.info()?;
    if !matches!(info.pixel_format, PixelFormat::L8 | PixelFormat::RGB24) {
        return None;
    }
    let (w, h) = (info.width as u32, info.height as u32);
    let k = [8u32, 4, 2]
        .into_iter()
        .find(|&k| w.min(h) / k >= short_min)?;
    let (sw, sh) = dec.scale(w.div_ceil(k) as u16, h.div_ceil(k) as u16).ok()?;
    let px = dec.decode().ok()?;
    let (sw, sh) = (sw as u32, sh as u32);
    let mut img = match info.pixel_format {
        PixelFormat::L8 => DynamicImage::ImageLuma8(image::GrayImage::from_raw(sw, sh, px)?),
        _ => DynamicImage::ImageRgb8(RgbImage::from_raw(sw, sh, px)?),
    };
    // the EXIF block: orientation and the capture moment, read once
    let exif = dec
        .exif_data()
        .and_then(|d| exif::Reader::new().read_raw(d.to_vec()).ok());
    let orientation = exif
        .as_ref()
        .and_then(|e| e.get_field(exif::Tag::Orientation, exif::In::PRIMARY))
        .and_then(|f| f.value.get_uint(0))
        .and_then(|o| Orientation::from_exif(o as u8))
        .unwrap_or(Orientation::NoTransforms);
    img.apply_orientation(orientation);
    let turned = matches!(
        orientation,
        Orientation::Rotate90
            | Orientation::Rotate270
            | Orientation::Rotate90FlipH
            | Orientation::Rotate270FlipH
    );
    let full = if turned { (h, w) } else { (w, h) };
    let capture = match (&exif, dec.exif_data()) {
        (Some(e), _) => Some(dedupe::capture::from_exif(e)),
        // no EXIF block at all: nothing for the caller to find either
        (None, None) => Some(None),
        // a block that did not parse: let the caller try its own reader
        (None, Some(_)) => None,
    };
    Some((
        Decoded {
            img,
            route: Route::Image,
            capture,
        },
        full,
    ))
}

fn pct(v: &mut [u32], p: f64) -> u32 {
    v.sort_unstable();
    v[((v.len() as f64 * p) as usize).min(v.len() - 1)]
}

fn jpegs() -> Option<Vec<PathBuf>> {
    let dirs = std::env::var_os("DEDUPE_JPEG_DIRS")?;
    let mut files: Vec<PathBuf> = std::env::split_paths(&dirs)
        .flat_map(|d| {
            walkdir::WalkDir::new(d)
                .into_iter()
                .flatten()
                .filter(|e| e.file_type().is_file())
                .map(|e| e.into_path())
        })
        .filter(|p| {
            p.extension().is_some_and(|e| {
                matches!(e.to_string_lossy().to_lowercase().as_str(), "jpg" | "jpeg")
            })
        })
        .collect();
    files.sort();
    Some(files)
}

/// The signature the program takes of a file (a large JPEG reduced inside
/// its DCT), and the one a full decode gives.
fn routes(p: &Path) -> (Signature, Signature) {
    let (rd, full) = load_reduced(p, 2 * GRID as u32).unwrap();
    let prod = signature_full(&rd.img, full);
    let all = signature_from_image(&load(p).unwrap().img);
    (prod, all)
}

/// The robustness table through each route: a large JPEG against copies
/// a library holds of it — a re-save at its size, at 70, 50, 35 and 25 %
/// (JPEG, quality 85), and as PNG at full and half size — by PDQ, dHash and
/// pHash on the upright picture: every file decoded in full, against
/// every file decoded as the program decodes it (a copy small enough is
/// decoded in full either way, so the pair mixes the routes).
#[test]
#[ignore]
fn reduced_route_on_copies() {
    let Some(files) = jpegs() else {
        eprintln!("DEDUPE_JPEG_DIRS not set; skipped");
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let kinds = [
        "re-save q85",
        "70 % q85",
        "half q85",
        "35 % q85",
        "quarter q85",
        "full png",
        "half png",
    ];
    // per kind: (full-route pdq, program-route pdq, full d+p, program d+p)
    let mut rows: Vec<[Vec<u32>; 4]> = (0..kinds.len()).map(|_| Default::default()).collect();
    let mut n = 0;
    for f in &files {
        let Ok((rd, full)) = load_reduced(f, 2 * GRID as u32) else {
            continue;
        };
        if (rd.img.width(), rd.img.height()) == full {
            continue;
        }
        n += 1;
        let up = load(f).unwrap().img;
        let (o_prod, o_all) = routes(f);
        let resized = |k: f32| {
            DynamicImage::ImageRgb8(image::imageops::resize(
                &up.to_rgb8(),
                ((up.width() as f32 * k).round() as u32).max(8),
                ((up.height() as f32 * k).round() as u32).max(8),
                image::imageops::FilterType::Triangle,
            ))
        };
        let jpeg = |img: &DynamicImage, name: &str| {
            let p = tmp.path().join(name);
            let mut w = std::io::BufWriter::new(std::fs::File::create(&p).unwrap());
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut w, 85)
                .encode_image(&img.to_rgb8())
                .unwrap();
            p
        };
        let png = |img: &DynamicImage, name: &str| {
            let p = tmp.path().join(name);
            img.to_rgb8().save(&p).unwrap();
            p
        };
        let copies = [
            jpeg(&up, "c0.jpg"),
            jpeg(&resized(0.7), "c1.jpg"),
            jpeg(&resized(0.5), "c2.jpg"),
            jpeg(&resized(0.35), "c3.jpg"),
            jpeg(&resized(0.25), "c4.jpg"),
            png(&up, "c5.png"),
            png(&resized(0.5), "c6.png"),
        ];
        for (i, c) in copies.iter().enumerate() {
            let (c_prod, c_all) = routes(c);
            rows[i][0].push(pdq_distance(&o_all.pdq[0], &c_all.pdq[0]));
            rows[i][1].push(pdq_distance(&o_prod.pdq[0], &c_prod.pdq[0]));
            let dp = |a: &Signature, b: &Signature| {
                hamming(a.dhash[0], b.dhash[0]).max(hamming(a.phash[0], b.phash[0]))
            };
            rows[i][2].push(dp(&o_all, &c_all));
            rows[i][3].push(dp(&o_prod, &c_prod));
        }
    }
    eprintln!();
    eprintln!(
        "{n} large JPEGs and {} copies of each; PDQ max / p95 / share ≤ {PDQ_MAX}, and the larger of dHash and pHash, max",
        kinds.len()
    );
    eprintln!(
        "{:<14} {:>24} {:>24}",
        "copy", "every file in full", "as the program decodes"
    );
    for (k, r) in kinds.iter().zip(rows.iter_mut()) {
        if r[0].is_empty() {
            continue;
        }
        let share =
            |v: &[u32]| 100.0 * v.iter().filter(|&&d| d <= PDQ_MAX).count() as f64 / v.len() as f64;
        let (s0, s1) = (share(&r[0]), share(&r[1]));
        let (m2, m3) = (pct(&mut r[2], 1.0), pct(&mut r[3], 1.0));
        eprintln!(
            "{k:<14} {:>5} {:>4} {:>6.1}%  d/p {:>2}   {:>5} {:>4} {:>6.1}%  d/p {:>2}",
            pct(&mut r[0], 1.0),
            pct(&mut r[0], 0.95),
            s0,
            m2,
            pct(&mut r[1], 1.0),
            pct(&mut r[1], 0.95),
            s1,
            m3
        );
    }
}

#[test]
#[ignore]
fn reduced_jpeg_route_against_full() {
    let Some(files) = jpegs() else {
        eprintln!("DEDUPE_JPEG_DIRS not set; skipped");
        return;
    };
    let (mut d, mut p, mut q) = (Vec::new(), Vec::new(), Vec::new());
    let (mut t_full, mut t_red) = (0.0f64, 0.0f64);
    let (mut px_full, mut px_red) = (0u64, 0u64);
    let (mut taken, mut size_diff, mut moment_diff, mut failed) = (0, 0, 0, 0);
    let mut worst: Vec<(u32, String)> = Vec::new();
    for f in &files {
        // the file in the page cache first, so neither route pays the disk;
        // each route timed twice, the faster taken
        let _ = std::fs::read(f);
        let Ok((rd, full)) = load_reduced(f, 2 * GRID as u32) else {
            failed += 1;
            continue;
        };
        if (rd.img.width(), rd.img.height()) == full {
            continue; // not reduced: the same decode either way
        }
        let route_reduced = || {
            let t = Instant::now();
            let (rd, full) = load_reduced(f, 2 * GRID as u32).unwrap();
            let s = signature_full(&rd.img, full);
            (t.elapsed().as_secs_f64(), s)
        };
        let route_full = || {
            let t = Instant::now();
            let fd = load(f).unwrap();
            let s = signature_from_image(&fd.img);
            (t.elapsed().as_secs_f64(), s, fd)
        };
        let (tr1, rs) = route_reduced();
        let (tf1, fs, fd) = route_full();
        let (tr2, _) = route_reduced();
        let (tf2, _, _) = route_full();
        let (tr, tf) = (tr1.min(tr2), tf1.min(tf2));
        taken += 1;
        t_full += tf;
        t_red += tr;
        px_full += fd.img.width() as u64 * fd.img.height() as u64;
        px_red += rd.img.width() as u64 * rd.img.height() as u64;
        if (fs.width, fs.height) != (rs.width, rs.height) {
            size_diff += 1;
            eprintln!(
                "  size differs: {} full {}x{} reduced says {}x{}",
                f.display(),
                fs.width,
                fs.height,
                rs.width,
                rs.height
            );
        }
        let full_moment = dedupe::capture::read(f);
        if let Some(m) = &rd.capture {
            if *m != full_moment {
                moment_diff += 1;
                eprintln!(
                    "  moment differs: {} {:?} {:?}",
                    f.display(),
                    full_moment,
                    m
                );
            }
        }
        let (dd, pp, qq) = (
            hamming(fs.dhash[0], rs.dhash[0]),
            hamming(fs.phash[0], rs.phash[0]),
            pdq_distance(&fs.pdq[0], &rs.pdq[0]),
        );
        d.push(dd);
        p.push(pp);
        q.push(qq);
        worst.push((qq, f.file_name().unwrap().to_string_lossy().into_owned()));
    }
    eprintln!();
    eprintln!(
        "{} JPEGs, {taken} through the reduced route ({failed} failed to decode either way)",
        files.len()
    );
    if taken == 0 {
        return;
    }
    let n = taken as f64;
    eprintln!(
        "distance between the routes' hashes of one file — dHash max {} p99 {} p95 {} | pHash max {} p99 {} p95 {} | PDQ max {} p99 {} p95 {} median {}",
        pct(&mut d, 1.0),
        pct(&mut d, 0.99),
        pct(&mut d, 0.95),
        pct(&mut p, 1.0),
        pct(&mut p, 0.99),
        pct(&mut p, 0.95),
        pct(&mut q, 1.0),
        pct(&mut q, 0.99),
        pct(&mut q, 0.95),
        pct(&mut q, 0.5)
    );
    eprintln!("full size differs in {size_diff}; capture moment differs in {moment_diff}");
    eprintln!(
        "decode and hash: full {:.0} ms, reduced {:.0} ms a file; pixels held: full {:.1} MP, reduced {:.1} MP a file",
        1000.0 * t_full / n,
        1000.0 * t_red / n,
        px_full as f64 / n / 1e6,
        px_red as f64 / n / 1e6
    );
    worst.sort_by_key(|w| std::cmp::Reverse(w.0));
    eprintln!("largest PDQ moves: {:?}", &worst[..worst.len().min(6)]);
}
