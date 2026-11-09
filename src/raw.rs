//! Camera RAW stills (CR2, CR3, NEF, NRW, ARW, SR2, DNG, RAF, ORF, RW2,
//! PEF, SRW and the rest): the picture is the camera's own embedded JPEG
//! preview, the orientation and capture moment come from the RAW's
//! metadata. Nothing here demosaics sensor data. [CLAUDE, Phase 10]
//!
//! Every RAW format carries at least one JPEG rendering of the shot, but
//! where differs per maker (TIFF IFDs, maker notes, a CR3 `PRVW` box, a
//! RAF header field). Rather than a parser per maker, the file is scanned
//! for JPEG start markers and each candidate is accepted only when its
//! whole marker chain parses: the SOI, then well-formed segments (each
//! length inside the file) up to a baseline or progressive frame header
//! with sane dimensions, quantisation tables before the scan, and the
//! entropy-coded data walked to its EOI. The scan of the real camera set
//! (RESEARCH §2.10) found byte runs in compressed sensor data that start
//! like a JPEG and claim a 64328×14895 frame; the chain check and the
//! dimension cap reject them, and a candidate is used only if it decodes
//! to the size its header states.
//!
//! The preview chosen is the smallest one whose short side is at least
//! 1000 pixels and whose shape matches the largest (the hashes work at
//! 288 and 512 pixels; a 1620×1080 preview is a twenty-first of a
//! 7360×4912 one's pixels); without such a one, the largest.

use std::ops::Range;

use exif::{Exif, Reader};

/// One embedded JPEG: its bytes in the file and the size its frame
/// header states.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preview {
    pub range: Range<usize>,
    pub width: u32,
    pub height: u32,
}

impl Preview {
    pub fn pixels(&self) -> u64 {
        self.width as u64 * self.height as u64
    }
}

/// Largest side accepted for an embedded JPEG.
const MAX_SIDE: u32 = 16_384;

/// Every well-formed embedded JPEG, largest first.
pub fn previews(b: &[u8]) -> Vec<Preview> {
    let mut out: Vec<Preview> = Vec::new();
    let mut i = 0;
    while let Some(p) = find(b, i, &[0xFF, 0xD8, 0xFF]) {
        match jpeg_at(b, p) {
            Some(pv) => {
                // the next candidate starts after this one (a thumbnail
                // inside its APP1 is smaller and not wanted)
                i = pv.range.end;
                out.push(pv);
            }
            None => i = p + 3,
        }
        if out.len() > 64 {
            break;
        }
    }
    out.sort_by_key(|p| std::cmp::Reverse(p.pixels()));
    out
}

/// The preview to decode: the smallest with a short side of at least
/// 1000 pixels and the largest's shape (within 1%), else the largest.
pub fn choose(list: &[Preview]) -> Option<&Preview> {
    let big = list.first()?;
    let aspect = big.width as f64 / big.height.max(1) as f64;
    list.iter()
        .filter(|p| p.width.min(p.height) >= 1000)
        .filter(|p| {
            let a = p.width as f64 / p.height.max(1) as f64;
            (a - aspect).abs() / aspect <= 0.01
        })
        .min_by_key(|p| p.pixels())
        .or(Some(big))
}

fn find(b: &[u8], from: usize, pat: &[u8]) -> Option<usize> {
    if from >= b.len() {
        return None;
    }
    b[from..]
        .windows(pat.len())
        .position(|w| w == pat)
        .map(|x| x + from)
}

fn be16(b: &[u8], i: usize) -> Option<usize> {
    b.get(i..i + 2)
        .map(|s| u16::from_be_bytes([s[0], s[1]]) as usize)
}

/// Parse the JPEG starting at `s` (an SOI) to its EOI.
fn jpeg_at(b: &[u8], s: usize) -> Option<Preview> {
    let n = b.len();
    let mut j = s + 2;
    let mut dqt = false;
    let mut dht = false;
    let mut frame: Option<(u32, u32, bool)> = None; // w, h, progressive
                                                    // markers up to the first scan
    loop {
        if j + 4 > n || b[j] != 0xFF {
            return None;
        }
        let m = b[j + 1];
        match m {
            0xFF => {
                j += 1; // fill byte
                continue;
            }
            0xD8 | 0x01 | 0xD0..=0xD7 | 0xD9 => return None,
            _ => {}
        }
        let len = be16(b, j + 2)?;
        if len < 2 || j + 2 + len > n {
            return None;
        }
        let seg = j + 4;
        match m {
            0xC0..=0xC2 => {
                // precision, height, width, components
                let p = *b.get(seg)?;
                let h = be16(b, seg + 1)? as u32;
                let w = be16(b, seg + 3)? as u32;
                let nc = *b.get(seg + 5)?;
                if p != 8 || !(16..=MAX_SIDE).contains(&w) || !(16..=MAX_SIDE).contains(&h) {
                    return None;
                }
                if nc != 1 && nc != 3 {
                    return None;
                }
                if len != 8 + 3 * nc as usize {
                    return None;
                }
                frame = Some((w, h, m == 0xC2));
            }
            0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF => return None, // lossless, hierarchical, arithmetic
            0xC4 => dht = true,
            0xDB => dqt = true,
            0xDA => break,
            0xDD | 0xE0..=0xEF | 0xFE => {}
            _ => return None,
        }
        j += 2 + len;
    }
    let (w, h, progressive) = frame?;
    if !dqt || !(dht || progressive) {
        return None;
    }
    // the scans: entropy-coded bytes (FF is followed by 00 or a restart),
    // then either another marker segment (progressive) or the EOI
    loop {
        // j is at an SOS marker
        let len = be16(b, j + 2)?;
        if j + 2 + len > n {
            return None;
        }
        j += 2 + len;
        loop {
            let k = j + b.get(j..)?.iter().position(|&x| x == 0xFF)?;
            let next = *b.get(k + 1)?;
            match next {
                0x00 | 0xD0..=0xD7 => j = k + 2,
                0xFF => j = k + 1,
                0xD9 => {
                    return Some(Preview {
                        range: s..k + 2,
                        width: w,
                        height: h,
                    })
                }
                0xDA => {
                    j = k;
                    break;
                }
                0xC4 | 0xDB | 0xDD | 0xE0..=0xEF | 0xFE => {
                    // a table between scans
                    let l = be16(b, k + 2)?;
                    if l < 2 || k + 2 + l > n {
                        return None;
                    }
                    let after = k + 2 + l;
                    if b.get(after) != Some(&0xFF) {
                        return None;
                    }
                    j = after;
                    // skip straight to the next marker segment
                    let mm = *b.get(j + 1)?;
                    if mm == 0xDA {
                        break;
                    }
                    if mm == 0xD9 {
                        return Some(Preview {
                            range: s..j + 2,
                            width: w,
                            height: h,
                        });
                    }
                }
                _ => return None,
            }
        }
    }
}

// ---------------------------------------------------------------------------
// metadata
// ---------------------------------------------------------------------------

/// What the RAW says about its orientation and capture moment, read from
/// the container: a TIFF structure for most makers (some with their own
/// magic number), CR3's `CMT1`/`CMT2` boxes, and for the rest (RAF) the
/// preview's own EXIF.
pub struct RawMeta {
    /// EXIF orientation 1–8.
    pub orientation: Option<u32>,
    pub capture: Option<crate::capture::Capture>,
}

/// Read EXIF from TIFF bytes; `fix_magic` rewrites a maker's own magic
/// number (Olympus `RO`/`RS`, Panasonic `U`) to TIFF's 42 first.
fn tiff_exif(b: &[u8], fix_magic: bool) -> Option<Exif> {
    if b.len() < 8 {
        return None;
    }
    let mut v = b.to_vec();
    if fix_magic {
        if &v[..2] == b"II" {
            v[2] = 42;
            v[3] = 0;
        } else if &v[..2] == b"MM" {
            v[2] = 0;
            v[3] = 42;
        }
    }
    Reader::new().read_raw(v).ok()
}

/// The children of an ISO-BMFF box body.
fn bmff_children(b: &[u8], r: Range<usize>) -> Vec<([u8; 4], Range<usize>)> {
    let mut out = Vec::new();
    let mut i = r.start;
    while i + 8 <= r.end {
        let size = u32::from_be_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]) as usize;
        let typ = [b[i + 4], b[i + 5], b[i + 6], b[i + 7]];
        let (size, hdr) = match size {
            0 => (r.end - i, 8),
            1 => {
                let Some(s) = b.get(i + 8..i + 16) else { break };
                (
                    u64::from_be_bytes(s.try_into().unwrap_or([0; 8])) as usize,
                    16,
                )
            }
            s => (s, 8),
        };
        if size < hdr || i + size > r.end {
            break;
        }
        out.push((typ, i + hdr..i + size));
        i += size;
    }
    out
}

/// CR3: moov → uuid 85c0b687-820f-11e0-8111-f4ce462b6a48 → CMT1 (IFD0),
/// CMT2 (the Exif IFD), each a complete TIFF structure.
fn cr3_tiffs(b: &[u8]) -> Vec<Range<usize>> {
    const CANON: [u8; 16] = [
        0x85, 0xc0, 0xb6, 0x87, 0x82, 0x0f, 0x11, 0xe0, 0x81, 0x11, 0xf4, 0xce, 0x46, 0x2b, 0x6a,
        0x48,
    ];
    let mut out = Vec::new();
    for (t, r) in bmff_children(b, 0..b.len()) {
        if &t != b"moov" {
            continue;
        }
        for (t2, r2) in bmff_children(b, r) {
            if &t2 == b"uuid" && b.get(r2.start..r2.start + 16) == Some(&CANON[..]) {
                for (t3, r3) in bmff_children(b, r2.start + 16..r2.end) {
                    if &t3 == b"CMT1" || &t3 == b"CMT2" {
                        out.push(r3);
                    }
                }
            }
        }
    }
    out
}

fn orientation_of(e: &Exif) -> Option<u32> {
    e.fields()
        .find(|f| f.tag.number() == 0x0112 && f.ifd_num == exif::In::PRIMARY)
        .and_then(|f| f.value.get_uint(0))
        .filter(|o| (1..=8).contains(o))
}

/// Orientation and capture moment of the RAW in `b`; `preview` is the
/// chosen preview's bytes, the fallback source.
pub fn meta(b: &[u8], preview: &[u8]) -> RawMeta {
    let mut sources: Vec<Exif> = Vec::new();
    if b.len() >= 8 {
        let head = &b[..4];
        if head == b"II*\0" || head == b"MM\0*" {
            sources.extend(tiff_exif(b, false));
        } else if head == b"IIRO" || head == b"IIRS" || head == b"MMOR" || head == b"IIU\0" {
            sources.extend(tiff_exif(b, true));
        } else if b.get(4..12) == Some(b"ftypcrx ") {
            for r in cr3_tiffs(b) {
                sources.extend(tiff_exif(&b[r], false));
            }
        }
    }
    // the preview's own EXIF (RAF, RW2 and any container read above
    // without a capture time)
    if let Ok(e) = Reader::new().read_from_container(&mut std::io::Cursor::new(preview)) {
        sources.push(e);
    }
    let orientation = sources.iter().find_map(orientation_of);
    let capture = sources.iter().find_map(crate::capture::from_exif);
    RawMeta {
        orientation,
        capture,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real baseline JPEG, encoded here.
    fn jpeg(w: u32, h: u32) -> Vec<u8> {
        let img =
            image::RgbImage::from_fn(w, h, |x, y| image::Rgb([(x * 7) as u8, (y * 5) as u8, 90]));
        let mut out = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 80)
            .encode_image(&img)
            .unwrap();
        out
    }

    #[test]
    fn finds_embedded_jpegs_exactly_and_largest_first() {
        let small = jpeg(160, 120);
        let big = jpeg(1620, 1080);
        let mut file = b"II*\0\x08\0\0\0".to_vec();
        file.extend(std::iter::repeat_n(0x55u8, 500));
        let a = file.len();
        file.extend_from_slice(&small);
        file.extend(std::iter::repeat_n(0xAAu8, 300));
        let c = file.len();
        file.extend_from_slice(&big);
        file.extend(std::iter::repeat_n(0x11u8, 100));
        let p = previews(&file);
        assert_eq!(p.len(), 2);
        assert_eq!((p[0].width, p[0].height), (1620, 1080));
        assert_eq!(p[0].range, c..c + big.len());
        assert_eq!(p[1].range, a..a + small.len());
        // and the bytes decode to the stated size
        let img = image::load_from_memory(&file[p[0].range.clone()]).unwrap();
        assert_eq!((img.width(), img.height()), (1620, 1080));
    }

    #[test]
    fn a_jpeg_lookalike_in_sensor_data_is_rejected() {
        // SOI, then an SOF0 claiming 64328×14895 (seen in a real RW2)
        let mut fake = vec![0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x11, 0x08];
        fake.extend_from_slice(&14895u16.to_be_bytes());
        fake.extend_from_slice(&64328u16.to_be_bytes());
        fake.extend_from_slice(&[3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]);
        fake.extend(std::iter::repeat_n(0x37u8, 200));
        assert!(previews(&fake).is_empty());
        // a plausible size but no tables and no end: rejected too
        let mut fake2 = vec![
            0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x11, 0x08, 0x04, 0x38, 0x06, 0x54,
        ];
        fake2.extend_from_slice(&[3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]);
        fake2.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x0C]);
        fake2.extend(std::iter::repeat_n(0x00u8, 50));
        assert!(previews(&fake2).is_empty());
    }

    #[test]
    fn choice_prefers_a_mid_size_preview_of_the_same_shape() {
        let p = |w, h| Preview {
            range: 0..1,
            width: w,
            height: h,
        };
        let list = vec![p(7360, 4912), p(1620, 1080), p(640, 424), p(160, 120)];
        assert_eq!(choose(&list).map(|x| x.width), Some(1620));
        // a mid-size preview of another shape (letterboxed) is passed over
        let list = vec![p(6000, 4000), p(1620, 1215), p(160, 120)];
        assert_eq!(choose(&list).map(|x| x.width), Some(6000));
        // nothing big enough: the largest
        let list = vec![p(960, 720), p(160, 120)];
        assert_eq!(choose(&list).map(|x| x.width), Some(960));
        assert!(choose(&[]).is_none());
    }
}
