//! Real camera and phone files through `decode::load` (Phase 10): every
//! file decodes upright to the size and capture moment recorded here.
//! The sizes and moments were checked against metadata-extractor's own
//! dumps of the same files (orientation, DateTimeOriginal) and by looking
//! at each decoded picture (docs/VERIFICATION.md § Phase 10).
//!
//! The files are not in the repository (third-party test images); point
//! `DEDUPE_FORMATS` at a folder holding the sparse checkouts named in
//! VERIFICATION (`runs/win/win_verify.ps1` fetches a subset) and run with
//! `--ignored`. The files present are checked. HEIC and AVIF need ffmpeg
//! with HEVC and AV1 (libdav1d or libaom) decoders.

use std::path::PathBuf;

use dedupe::decode::{self, Route};

fn root() -> Option<PathBuf> {
    std::env::var_os("DEDUPE_FORMATS").map(PathBuf::from)
}

/// (path under the root, route, upright size, capture moment)
type Case = (&'static str, &'static str, (u32, u32), Option<&'static str>);
const CASES: &[Case] = &[
    // HEIC: iPhone 48-tile grids, one turned by irot 3; a Sony 6-tile grid
    (
        "mei/heic/IMG_1034.heic",
        "HEIC",
        (4032, 3024),
        Some("2018-02-05 15:11:44"),
    ),
    (
        "mei/heic/IMG_2927.HEIC",
        "HEIC",
        (3024, 4032),
        Some("2020-05-04 18:39:11"),
    ),
    (
        "mei/heic/IMG_2928.HEIC",
        "HEIC",
        (3024, 4032),
        Some("2020-05-04 18:43:31"),
    ),
    (
        "mei/heic/IMG_2929.HEIC",
        "HEIC",
        (3024, 4032),
        Some("2020-05-04 18:44:06"),
    ),
    (
        "mei/heic/Issue 263 dotnet.heic",
        "HEIC",
        (3024, 4032),
        Some("2019-08-21 11:21:36"),
    ),
    (
        "mei/heic/Issue 437 (dotnet).heic",
        "HEIC",
        (4000, 1848),
        Some("2025-05-09 11:38:44"),
    ),
    (
        "mei/heic/Issue 487.heic",
        "HEIC",
        (3024, 4032),
        Some("2020-06-30 11:27:59"),
    ),
    (
        "mei/heic/exif-at-eof.heic",
        "HEIC",
        (3024, 4032),
        Some("2020-05-04 18:39:11"),
    ),
    ("mei/heic/cheers_1440x960.heic", "HEIC", (1440, 960), None),
    (
        "mei/heif/Sony ILCE-7M4 (A7M4).heif",
        "HEIC",
        (7008, 4672),
        Some("2022-06-10 18:17:40"),
    ),
    (
        "strukturag_libheif/examples/example.heic",
        "HEIC",
        (1280, 854),
        None,
    ),
    // AVIF
    (
        "strukturag_libheif/examples/example.avif",
        "AVIF",
        (800, 533),
        None,
    ),
    (
        "mei/avif/Issue 649.avif",
        "AVIF",
        (5472, 3648),
        Some("2015-10-11 14:41:44"),
    ),
    (
        "AOMediaCodec_av1-avif/testFiles/Link-U/kimono.rotate90.avif",
        "AVIF",
        (722, 1024),
        None,
    ),
    // the Microsoft set's stated display sizes: a 5×4 grid of 1280×720
    // pictures shown as one 6400×2880 image, a quarter turn, two crops,
    // a monochrome picture
    (
        "AOMediaCodec_av1-avif/testFiles/Microsoft/Summer_in_Tomsk_720p_5x4_grid.avif",
        "AVIF",
        (6400, 2880),
        None,
    ),
    (
        "AOMediaCodec_av1-avif/testFiles/Microsoft/Ronda_rotate90.avif",
        "AVIF",
        (1080, 1920),
        Some("2016-12-29 11:24:41"),
    ),
    (
        "AOMediaCodec_av1-avif/testFiles/Microsoft/Chimera_8bit_cropped_480x256.avif",
        "AVIF",
        (480, 256),
        Some("2018-08-30 23:40:38"),
    ),
    (
        "AOMediaCodec_av1-avif/testFiles/Microsoft/Chimera_10bit_cropped_to_1920x1008.avif",
        "AVIF",
        (1920, 1008),
        Some("2018-08-30 00:38:53"),
    ),
    (
        "AOMediaCodec_av1-avif/testFiles/Microsoft/Monochrome.avif",
        "AVIF",
        (1280, 720),
        None,
    ),
    // cropped 330×385 as stored, then turned: displayed 385×330, as the
    // plain crop file is
    (
        "AOMediaCodec_av1-avif/testFiles/Link-U/kimono.mirror-vertical.rotate270.crop.avif",
        "AVIF",
        (385, 330),
        None,
    ),
    // RAW: the embedded preview, oriented by the RAW (two portraits)
    (
        "mei/cr2/Canon EOS 70D.cr2",
        "RAW",
        (5472, 3648),
        Some("2014-04-29 17:07:52"),
    ),
    (
        "mei/cr2/Canon EOS 350D.CR2",
        "RAW",
        (1536, 1024),
        Some("2010-05-20 20:37:26"),
    ),
    (
        "mei/cr3/Canon EOS R3.cr3",
        "RAW",
        (1620, 1080),
        Some("2023-02-09 10:10:27"),
    ),
    (
        "mei/nef/Nikon D810.nef",
        "RAW",
        (1080, 1620),
        Some("2015-05-13 19:49:55"),
    ),
    (
        "mei/nef/Nikon D70.nef",
        "RAW",
        (2000, 3008),
        Some("2007-01-13 10:30:55"),
    ),
    (
        "mei/arw/Sony ILCE-7M3 (A7M3).arw",
        "RAW",
        (1080, 1616),
        Some("2019-04-24 22:14:20"),
    ),
    (
        "mei/arw/Sony NEX-5N.arw",
        "RAW",
        (1616, 1080),
        Some("2011-02-26 15:51:13"),
    ),
    (
        "mei/dng/Leica T (Typ 701).dng",
        "RAW",
        (720, 480),
        Some("2014-05-07 10:41:44"),
    ),
    (
        "mei/dng/LG Nexus 5X.dng",
        "RAW",
        (4080, 3028),
        Some("2017-08-28 09:43:14"),
    ),
    (
        "mei/raf/Fujifilm X-E1.raf",
        "RAW",
        (1920, 1280),
        Some("2012-11-08 14:57:15"),
    ),
    (
        "mei/orf/Olympus E-M5.orf",
        "RAW",
        (3200, 2400),
        Some("2012-05-08 16:14:20"),
    ),
    // the clock was never set: DateTimeOriginal is all zeros
    ("mei/orf/OM Sytem OM-1.orf", "RAW", (3200, 2400), None),
    (
        "mei/rw2/Panasonic DMC-GF7.rw2",
        "RAW",
        (1920, 1440),
        Some("2015-01-18 15:41:07"),
    ),
    (
        "mei/pef/Pentax K-1 Mark II.pef",
        "RAW",
        (7360, 4912),
        Some("2020-07-25 22:48:40"),
    ),
];

#[test]
#[ignore]
fn real_files_decode_upright_with_their_moment() {
    let Some(root) = root() else {
        eprintln!("DEDUPE_FORMATS not set; skipped");
        return;
    };
    // the files present are checked (the Windows run fetches a subset)
    let mut failures = Vec::new();
    let mut checked = 0;
    for (rel, route, size, moment) in CASES {
        let p = root.join(rel);
        if !p.is_file() {
            continue;
        }
        checked += 1;
        match decode::load(&p) {
            Ok(d) => {
                let got_route = d.route.label().unwrap_or("image");
                let got_size = (d.img.width(), d.img.height());
                let got_moment = match &d.capture {
                    Some(c) => c.as_ref().map(|c| c.datetime.clone()),
                    None => dedupe::capture::read(&p).map(|c| c.datetime),
                };
                if got_route != *route || got_size != *size || got_moment.as_deref() != *moment {
                    failures.push(format!(
                        "{rel}: {got_route} {got_size:?} {got_moment:?}, expected {route} {size:?} {moment:?}"
                    ));
                }
            }
            Err(e) => failures.push(format!("{rel}: {e}")),
        }
    }
    eprintln!("checked {checked} of {} files", CASES.len());
    assert!(checked > 0, "no files under {}", root.display());
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The malformed HEIF files of the same set are errors, never panics.
#[test]
#[ignore]
fn malformed_heif_files_are_errors() {
    let Some(root) = root() else {
        return;
    };
    for f in [
        "mei/heic/IllegalArgumentException.HeifReader.processBoxes.heif",
        "mei/heic/NegativeArraySizeException.HeifReader.processBoxes.heif",
        "mei/heic/NegativeArraySizeException.ItemInfoBox.init.heif",
        "mei/heic/NullPointerException.HeifPictureHandler.processBox.heif",
    ] {
        let p = root.join(f);
        if p.is_file() {
            assert!(decode::load(&p).is_err(), "{f}");
        }
    }
    // and every route label used above exists
    assert_eq!(Route::Raw.label(), Some("RAW"));
}

/// Undoing each Link-U file's transform gives the untransformed picture:
/// the files state what they must display as (RESEARCH §2.9).
#[test]
#[ignore]
fn avif_transforms_display_as_the_original() {
    let Some(root) = root() else {
        return;
    };
    let dir = root.join("AOMediaCodec_av1-avif/testFiles/Link-U");
    if !dir.join("kimono.avif").is_file() {
        eprintln!("no Link-U files; skipped");
        return;
    }
    let orig = decode::load_image(&dir.join("kimono.avif"))
        .unwrap()
        .to_luma8();
    for f in [
        "kimono.rotate90.avif",
        "kimono.rotate270.avif",
        "kimono.mirror-horizontal.avif",
        "kimono.mirror-vertical.avif",
        "kimono.mirror-vertical.rotate270.avif",
    ] {
        let got = decode::load_image(&dir.join(f)).unwrap().to_luma8();
        assert_eq!(got.dimensions(), orig.dimensions(), "{f}");
        let mad: f64 = got
            .pixels()
            .zip(orig.pixels())
            .map(|(a, b)| (a.0[0] as f64 - b.0[0] as f64).abs())
            .sum::<f64>()
            / (got.width() * got.height()) as f64;
        // re-encoded pictures: small differences, never a turned or
        // flipped picture (which differs by tens of levels)
        assert!(mad < 4.0, "{f}: mean difference {mad:.2}");
    }
    // crop, turn, mirror in that order lands on the same crop as the
    // plain crop file
    let a = decode::load_image(&dir.join("kimono.crop.avif"))
        .unwrap()
        .to_luma8();
    let b = decode::load_image(&dir.join("kimono.mirror-vertical.rotate270.crop.avif"))
        .unwrap()
        .to_luma8();
    assert_eq!(a.dimensions(), (385, 330));
    assert_eq!(b.dimensions(), (385, 330));
    let mad: f64 = a
        .pixels()
        .zip(b.pixels())
        .map(|(x, y)| (x.0[0] as f64 - y.0[0] as f64).abs())
        .sum::<f64>()
        / (385.0 * 330.0);
    assert!(mad < 4.0, "crop files differ: {mad:.2}");
}
