//! Every still decoder in one place (TECH-01, Phase 10). Everything that
//! needs pixels — the hashes, the deep tier, the geometric fit, the
//! workspace's view — calls `load`, which picks the route by the file's
//! own bytes and returns the picture upright:
//!
//! - **image** — JPEG, PNG, GIF, WebP, BMP, TIFF, ICO through the `image`
//!   crate, EXIF orientation applied;
//! - **HEIF** — HEIC, HEIF and AVIF: `heif` reads the container, ffmpeg
//!   decodes the tiles as an elementary stream (colour as the stream
//!   signals it), the tiles are composed and the container's crop,
//!   rotation and mirror applied here;
//! - **RAW** — the camera's embedded JPEG preview (`raw`), oriented by the
//!   RAW's metadata;
//! - **ffmpeg** — anything the crate cannot read that ffmpeg can (JPEG XL,
//!   PSD, a DNG without a preview), one frame through `image2pipe`.
//!
//! Video frames for the workspace's view are here too (`video_frame`); the
//! video pass streams its own frames (`video`). [CLAUDE]

use std::io::{self, Read};
use std::path::Path;

use image::metadata::Orientation;
use image::{DynamicImage, ImageDecoder, ImageReader, RgbImage};

use crate::capture::{self, Capture};
use crate::heif::{self, Codec, Heif, Transform};
use crate::raw;

/// How a picture was decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    Image,
    Heif(Codec),
    Raw,
    Ffmpeg,
}

impl Route {
    /// The name the DECODE line counts it under; `None` for ordinary
    /// images.
    pub fn label(self) -> Option<&'static str> {
        match self {
            Route::Image => None,
            Route::Heif(c) => Some(c.name()),
            Route::Raw => Some("RAW"),
            Route::Ffmpeg => Some("ffmpeg"),
        }
    }
}

pub struct Decoded {
    /// Upright: EXIF orientation, or the HEIF transforms, applied.
    pub img: DynamicImage,
    pub route: Route,
    /// The capture moment, when the route read the metadata itself
    /// (HEIF, RAW). `None` means the caller reads it from the file.
    pub capture: Option<Option<Capture>>,
}

#[derive(Debug)]
pub enum DecodeError {
    Io(io::Error),
    Image(image::ImageError),
    /// The HEIF container could not be read.
    Heif(String),
    /// The route needs ffmpeg and it was not found (PATH, nor where an
    /// installer puts it: `tools`).
    NoFfmpeg(&'static str),
    /// ffmpeg ran and could not decode (no decoder for the coding, a
    /// broken stream); the message is its last line.
    Ffmpeg(&'static str, String),
    /// A RAW with no usable embedded preview.
    NoPreview,
    /// ffmpeg sent nothing for `tools::stall()` and was stopped.
    Stalled(&'static str),
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecodeError::Io(e) => write!(f, "{e}"),
            DecodeError::Image(e) => write!(f, "{e}"),
            DecodeError::Heif(e) => write!(f, "HEIF: {e}"),
            DecodeError::NoFfmpeg(what) => write!(f, "{what} needs ffmpeg, not found"),
            DecodeError::Ffmpeg(what, e) => write!(f, "{what}: ffmpeg: {e}"),
            DecodeError::NoPreview => write!(f, "RAW without an embedded preview"),
            DecodeError::Stalled(what) => write!(
                f,
                "{what}: ffmpeg sent nothing for {} s and was stopped",
                crate::tools::stall().as_secs()
            ),
        }
    }
}

impl DecodeError {
    /// The short reason the DECODE line groups failures by.
    pub fn reason(&self) -> String {
        match self {
            DecodeError::NoFfmpeg(what) => format!("{what} need ffmpeg"),
            DecodeError::Ffmpeg(what, _) => format!("{what} ffmpeg could not decode"),
            DecodeError::NoPreview => "RAW without a preview".to_string(),
            DecodeError::Stalled(_) => "timed out".to_string(),
            DecodeError::Heif(_) => "HEIF unreadable".to_string(),
            DecodeError::Io(_) | DecodeError::Image(_) => "unreadable".to_string(),
        }
    }
}

/// Decode `path`, upright.
pub fn load(path: &Path) -> Result<Decoded, DecodeError> {
    let mut head = [0u8; 64];
    let n = {
        let mut f = std::fs::File::open(path).map_err(DecodeError::Io)?;
        read_up_to(&mut f, &mut head).map_err(DecodeError::Io)?
    };
    let head = &head[..n];
    if heif::is_heif(head) {
        let b = std::fs::read(path).map_err(DecodeError::Io)?;
        return load_heif(&b);
    }
    if crate::kind::is_raw(path) {
        let b = std::fs::read(path).map_err(DecodeError::Io)?;
        return load_raw(path, &b);
    }
    match load_with_crate(path) {
        Ok(img) => Ok(Decoded {
            img,
            route: Route::Image,
            capture: None,
        }),
        Err(DecodeError::Image(image::ImageError::Unsupported(_))) => {
            let img = ffmpeg_still(path, "this format")?;
            Ok(Decoded {
                img,
                route: Route::Ffmpeg,
                capture: None,
            })
        }
        Err(e) => Err(e),
    }
}

/// Has a route here: every image kind the scan accepts (`kind::IMAGE_EXT`)
/// — the `image` crate's formats, the HEIF family, camera RAWs, and the
/// rest through ffmpeg. Which route a file takes is decided by its bytes.
pub fn decodable(path: &Path) -> bool {
    crate::kind::has_ext(path, crate::kind::IMAGE_EXT)
}

/// A HEIF-family file held in memory (the doctor's probes).
pub fn load_heif_bytes(b: &[u8]) -> Result<Decoded, DecodeError> {
    load_heif(b)
}

/// `load`, the picture only.
pub fn load_image(path: &Path) -> Result<DynamicImage, DecodeError> {
    load(path).map(|d| d.img)
}

fn read_up_to(f: &mut std::fs::File, buf: &mut [u8]) -> io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match f.read(&mut buf[n..])? {
            0 => break,
            k => n += k,
        }
    }
    Ok(n)
}

/// The `image` crate, EXIF orientation applied.
fn load_with_crate(path: &Path) -> Result<DynamicImage, DecodeError> {
    let reader = ImageReader::open(path)
        .map_err(DecodeError::Io)?
        .with_guessed_format()
        .map_err(DecodeError::Io)?;
    let mut decoder = reader.into_decoder().map_err(DecodeError::Image)?;
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let mut img = DynamicImage::from_decoder(decoder).map_err(DecodeError::Image)?;
    img.apply_orientation(orientation);
    Ok(img)
}

// ---------------------------------------------------------------------------
// HEIF / AVIF
// ---------------------------------------------------------------------------

fn load_heif(b: &[u8]) -> Result<Decoded, DecodeError> {
    let h = heif::read(b).map_err(|e| DecodeError::Heif(e.0))?;
    let what = h.codec.name();
    let frames: Vec<RgbImage> = match h.codec {
        Codec::Jpeg => h
            .tiles
            .iter()
            .map(|t| {
                image::load_from_memory_with_format(t, image::ImageFormat::Jpeg)
                    .map(|i| i.to_rgb8())
                    .map_err(DecodeError::Image)
            })
            .collect::<Result<_, _>>()?,
        Codec::Hevc | Codec::Av1 => {
            let fmt = if h.codec == Codec::Hevc {
                "hevc"
            } else {
                "obu"
            };
            let frames = ffmpeg_frames(what, fmt, h.stream())?;
            if frames.len() != h.tiles.len() {
                return Err(DecodeError::Ffmpeg(
                    what,
                    format!("{} pictures for {} tiles", frames.len(), h.tiles.len()),
                ));
            }
            frames
        }
    };
    let img = compose(&h, frames).map_err(|e| DecodeError::Heif(e.to_string()))?;
    let img = transform(img, &h.transforms);
    let cap = h
        .exif
        .clone()
        .and_then(|r| exif::Reader::new().read_raw(b[r].to_vec()).ok())
        .and_then(|e| capture::from_exif(&e));
    Ok(Decoded {
        img: DynamicImage::ImageRgb8(img),
        route: Route::Heif(h.codec),
        capture: Some(cap),
    })
}

/// Place the tiles of a grid row by row and cut to the output size; a
/// single item is its one picture.
fn compose(h: &Heif, mut frames: Vec<RgbImage>) -> Result<RgbImage, &'static str> {
    let Some((rows, cols, ow, oh)) = h.grid else {
        let f = frames.pop().ok_or("no picture")?;
        // a coded picture may carry padding beyond the item's size
        if f.width() > h.width || f.height() > h.height {
            return Ok(image::imageops::crop_imm(
                &f,
                0,
                0,
                h.width.min(f.width()),
                h.height.min(f.height()),
            )
            .to_image());
        }
        return Ok(f);
    };
    let (tw, th) = frames
        .first()
        .map(|f| (f.width(), f.height()))
        .ok_or("no tiles")?;
    if frames.iter().any(|f| f.width() != tw || f.height() != th) {
        return Err("tiles of different sizes");
    }
    if cols * tw < ow || rows * th < oh {
        return Err("tiles do not cover the image");
    }
    let mut canvas = RgbImage::new(ow, oh);
    for (i, f) in frames.iter().enumerate() {
        let (r, c) = (i as u32 / cols, i as u32 % cols);
        image::imageops::replace(&mut canvas, f, (c * tw) as i64, (r * th) as i64);
    }
    Ok(canvas)
}

/// Apply the container's transforms in order.
pub fn transform(mut img: RgbImage, ts: &[Transform]) -> RgbImage {
    for t in ts {
        img = match *t {
            Transform::Crop {
                left,
                top,
                width,
                height,
            } => {
                if left + width <= img.width() && top + height <= img.height() {
                    image::imageops::crop_imm(&img, left, top, width, height).to_image()
                } else {
                    img
                }
            }
            // n quarter turns anticlockwise
            Transform::Rotate(1) => image::imageops::rotate270(&img),
            Transform::Rotate(2) => image::imageops::rotate180(&img),
            Transform::Rotate(3) => image::imageops::rotate90(&img),
            Transform::Rotate(_) => img,
            // mode 0 flips top to bottom, mode 1 left to right
            Transform::Mirror(0) => image::imageops::flip_vertical(&img),
            Transform::Mirror(_) => image::imageops::flip_horizontal(&img),
        };
    }
    img
}

// ---------------------------------------------------------------------------
// RAW
// ---------------------------------------------------------------------------

fn load_raw(path: &Path, b: &[u8]) -> Result<Decoded, DecodeError> {
    let list = raw::previews(b);
    // the chosen preview first, then the rest largest first, until one
    // decodes to the size its header states
    let chosen = raw::choose(&list);
    let order = chosen
        .into_iter()
        .chain(list.iter().filter(|p| Some(*p) != chosen));
    for p in order {
        let bytes = &b[p.range.clone()];
        let Ok(img) = image::load_from_memory_with_format(bytes, image::ImageFormat::Jpeg) else {
            continue;
        };
        if (img.width(), img.height()) != (p.width, p.height) {
            continue;
        }
        let meta = raw::meta(b, bytes);
        let mut img = img;
        if let Some(o) = meta
            .orientation
            .and_then(|o| Orientation::from_exif(o as u8))
        {
            img.apply_orientation(o);
        }
        return Ok(Decoded {
            img,
            route: Route::Raw,
            capture: Some(meta.capture),
        });
    }
    // no preview: ffmpeg may still read it (its TIFF decoder handles some DNGs)
    match ffmpeg_still(path, "RAW") {
        Ok(img) => {
            let meta = raw::meta(b, &[]);
            let mut img = img;
            if let Some(o) = meta
                .orientation
                .and_then(|o| Orientation::from_exif(o as u8))
            {
                img.apply_orientation(o);
            }
            Ok(Decoded {
                img,
                route: Route::Raw,
                capture: Some(meta.capture),
            })
        }
        Err(_) => Err(DecodeError::NoPreview),
    }
}

// ---------------------------------------------------------------------------
// ffmpeg
// ---------------------------------------------------------------------------

/// Run ffmpeg with `args`, `input` on stdin (written from a thread so a
/// full stdout pipe cannot stall it), and return stdout. A child silent
/// for `tools::stall()` is stopped (Phase 13, CORE-11).
fn ffmpeg(
    what: &'static str,
    args: &[String],
    input: Option<Vec<u8>>,
) -> Result<Vec<u8>, DecodeError> {
    let ran = crate::tools::run(crate::tools::ffmpeg().args(args), input).map_err(|e| {
        match e.kind() {
            io::ErrorKind::NotFound => DecodeError::NoFfmpeg(what),
            io::ErrorKind::TimedOut => DecodeError::Stalled(what),
            _ => DecodeError::Ffmpeg(what, e.to_string()),
        }
    })?;
    if !ran.status.success() || ran.stdout.is_empty() {
        let msg = String::from_utf8_lossy(&ran.stderr);
        let last = msg
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("no output")
            .trim()
            .to_string();
        return Err(DecodeError::Ffmpeg(what, last));
    }
    Ok(ran.stdout)
}

fn base_args() -> Vec<String> {
    ["-nostats", "-hide_banner", "-loglevel", "error"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// Decode an elementary stream into its pictures, in order.
///
/// Colour: the coded stream's own signalling (range, matrix) decides. A
/// container's `colr nclx` can disagree with it — the Sony A7M4 HEIF says
/// limited range while its HEVC stream says full — and on that file the
/// stream's reading matches the camera's own JPEG thumbnail (4.3 levels
/// mean difference against 9.2; RESEARCH §2.9).
fn ffmpeg_frames(
    what: &'static str,
    fmt: &str,
    stream: Vec<u8>,
) -> Result<Vec<RgbImage>, DecodeError> {
    let mut args = base_args();
    args.extend(["-f", fmt, "-i", "pipe:0"].map(String::from));
    args.extend(
        [
            "-fps_mode",
            "passthrough",
            "-pix_fmt",
            "rgb24",
            "-f",
            "image2pipe",
            "-c:v",
            "ppm",
            "pipe:1",
        ]
        .map(String::from),
    );
    let out = ffmpeg(what, &args, Some(stream))?;
    ppm_frames(&out).ok_or_else(|| DecodeError::Ffmpeg(what, "unreadable output".into()))
}

/// The first frame of anything ffmpeg can open.
fn ffmpeg_still(path: &Path, what: &'static str) -> Result<DynamicImage, DecodeError> {
    let mut args = base_args();
    args.push("-i".into());
    args.push(path.to_string_lossy().into_owned());
    args.extend(
        [
            "-frames:v",
            "1",
            "-pix_fmt",
            "rgb24",
            "-f",
            "image2pipe",
            "-c:v",
            "ppm",
            "pipe:1",
        ]
        .map(String::from),
    );
    let out = ffmpeg(what, &args, None)?;
    ppm_frames(&out)
        .and_then(|mut v| (!v.is_empty()).then(|| v.remove(0)))
        .map(DynamicImage::ImageRgb8)
        .ok_or_else(|| DecodeError::Ffmpeg(what, "unreadable output".into()))
}

/// The frame a third of the way into a video (the workspace's view):
/// `-ss` before `-i`, so the seek is to the keyframe and cheap.
pub fn video_frame(path: &Path, duration: f64) -> Option<DynamicImage> {
    let t = format!("{:.3}", (duration / 3.0).max(0.0));
    let mut args = base_args();
    args.extend(["-ss".to_string(), t, "-i".to_string()]);
    args.push(path.to_string_lossy().into_owned());
    args.extend(
        [
            "-frames:v",
            "1",
            "-pix_fmt",
            "rgb24",
            "-f",
            "image2pipe",
            "-c:v",
            "ppm",
            "pipe:1",
        ]
        .map(String::from),
    );
    let out = ffmpeg("video", &args, None).ok()?;
    ppm_frames(&out)
        .and_then(|mut v| (!v.is_empty()).then(|| v.remove(0)))
        .map(DynamicImage::ImageRgb8)
}

/// Binary PPM frames laid end to end (`P6 w h 255`, then w·h·3 bytes).
pub(crate) fn ppm_frames(b: &[u8]) -> Option<Vec<RgbImage>> {
    let mut out = Vec::new();
    let mut i = 0;
    let field = |i: &mut usize| -> Option<u32> {
        // whitespace and comments, then digits
        loop {
            match b.get(*i)? {
                b' ' | b'\n' | b'\r' | b'\t' => *i += 1,
                b'#' => {
                    while *b.get(*i)? != b'\n' {
                        *i += 1;
                    }
                }
                _ => break,
            }
        }
        let start = *i;
        while b.get(*i).is_some_and(u8::is_ascii_digit) {
            *i += 1;
        }
        std::str::from_utf8(&b[start..*i]).ok()?.parse().ok()
    };
    while i < b.len() {
        if b.get(i..i + 2)? != b"P6" {
            return None;
        }
        i += 2;
        let w = field(&mut i)?;
        let h = field(&mut i)?;
        let max = field(&mut i)?;
        if max != 255 || w == 0 || h == 0 {
            return None;
        }
        i += 1; // the one whitespace byte before the data
        let len = (w as usize).checked_mul(h as usize)?.checked_mul(3)?;
        let data = b.get(i..i + len)?.to_vec();
        i += len;
        out.push(RgbImage::from_raw(w, h, data)?);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ppm_frames_split_and_check() {
        let mut b = b"P6\n2 1\n255\n".to_vec();
        b.extend_from_slice(&[1, 2, 3, 4, 5, 6]);
        b.extend_from_slice(b"P6 1 2 255\n");
        b.extend_from_slice(&[7, 8, 9, 10, 11, 12]);
        let f = ppm_frames(&b).unwrap();
        assert_eq!(f.len(), 2);
        assert_eq!(f[0].dimensions(), (2, 1));
        assert_eq!(f[1].get_pixel(0, 1).0, [10, 11, 12]);
        assert!(ppm_frames(&b[..b.len() - 1]).is_none());
        assert!(ppm_frames(b"P5\n1 1\n255\n\0").is_none());
    }

    /// The transforms against the AVIF test set's own statement of what
    /// each file shows (RESEARCH §2.9): undoing an encoded rotation and
    /// mirror restores the original, in association order.
    #[test]
    fn transforms_undo_what_the_test_files_encoded() {
        let orig = RgbImage::from_fn(4, 3, |x, y| image::Rgb([x as u8, y as u8, 0]));
        // encoded rotated 90° anticlockwise, displayed with irot 3
        let enc = image::imageops::rotate270(&orig);
        assert_eq!(transform(enc, &[Transform::Rotate(3)]), orig);
        // mirrored left-right, displayed with imir 1
        let enc = image::imageops::flip_horizontal(&orig);
        assert_eq!(transform(enc, &[Transform::Mirror(1)]), orig);
        // flipped top-bottom, displayed with imir 0
        let enc = image::imageops::flip_vertical(&orig);
        assert_eq!(transform(enc, &[Transform::Mirror(0)]), orig);
        // mirrored top-bottom then turned 90° clockwise; displayed with
        // irot 1 then imir 0 (kimono.mirror-vertical.rotate270)
        let enc = image::imageops::rotate90(&image::imageops::flip_vertical(&orig));
        assert_eq!(
            transform(enc, &[Transform::Rotate(1), Transform::Mirror(0)]),
            orig
        );
        // a crop
        let c = transform(
            orig.clone(),
            &[Transform::Crop {
                left: 1,
                top: 1,
                width: 2,
                height: 2,
            }],
        );
        assert_eq!(c.dimensions(), (2, 2));
        assert_eq!(c.get_pixel(0, 0).0, [1, 1, 0]);
    }

    #[test]
    fn a_grid_composes_row_by_row_and_crops() {
        let tile = |v: u8| RgbImage::from_pixel(4, 2, image::Rgb([v, v, v]));
        let h = Heif {
            codec: Codec::Hevc,
            tiles: vec![vec![]; 4],
            configs: vec![vec![]; 4],
            grid: Some((2, 2, 7, 3)),
            width: 7,
            height: 3,
            transforms: vec![],
            exif: None,
        };
        let img = compose(&h, vec![tile(10), tile(20), tile(30), tile(40)]).unwrap();
        assert_eq!(img.dimensions(), (7, 3));
        assert_eq!(img.get_pixel(0, 0).0[0], 10);
        assert_eq!(img.get_pixel(6, 0).0[0], 20);
        assert_eq!(img.get_pixel(0, 2).0[0], 30);
        assert_eq!(img.get_pixel(6, 2).0[0], 40);
        // tiles that cannot cover the output are an error
        let mut h2 = h.clone();
        h2.grid = Some((2, 2, 9, 3));
        assert!(compose(&h2, vec![tile(1), tile(2), tile(3), tile(4)]).is_err());
    }
}
