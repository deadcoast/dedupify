//! Capture time (docs/RESEARCH.md §2.8): two photographs taken at
//! different moments are two photographs, however alike they look.
//!
//! EXIF `DateTimeOriginal` is written by the camera at exposure and kept
//! by every editor and exporter that keeps EXIF at all; a copy, a
//! re-encode, a resize or a crop carries its source's value. A burst of
//! frames, a stereo pair, the same board photographed from another angle
//! each carry their own. So when both files state a capture time and the
//! two differ, the pair is `kin` — shown, never moved, never binned.
//! When either file states none (screenshots, renders, stripped exports)
//! the rule says nothing and the hashes decide alone.
//!
//! `SubSecTimeOriginal` separates frames shot within the same second
//! when the camera recorded it.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use exif::{Exif, In, Reader, Tag};

/// What a file says about when it was exposed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capture {
    /// `YYYY-MM-DD HH:MM:SS` (the reader renders EXIF's colons as dashes).
    pub datetime: String,
    /// Fraction of a second, when recorded.
    pub subsec: Option<String>,
}

impl Capture {
    /// Two capture times are the same moment when the seconds agree and
    /// the fractions agree wherever both are present.
    pub fn same_moment(&self, other: &Capture) -> bool {
        if self.datetime != other.datetime {
            return false;
        }
        match (&self.subsec, &other.subsec) {
            (Some(a), Some(b)) => a.trim_end_matches('0') == b.trim_end_matches('0'),
            _ => true,
        }
    }

    /// `2019-06-12 10:11:12` for a result line.
    pub fn display(&self) -> String {
        match &self.subsec {
            Some(f) if !f.trim().is_empty() => format!("{}.{}", self.datetime, f.trim()),
            _ => self.datetime.clone(),
        }
    }
}

/// Read the capture time, if the file carries one. Any failure — no
/// EXIF, a container without EXIF support, a malformed field — is `None`.
pub fn read(path: &Path) -> Option<Capture> {
    let file = File::open(path).ok()?;
    let mut reader = BufReader::new(file);
    let exif = Reader::new().read_from_container(&mut reader).ok()?;
    from_exif(&exif)
}

/// The first ASCII string of the primary image's field `number`, matched
/// by number alone: a CR3 keeps its Exif fields in a TIFF structure of
/// their own (`CMT2`), where the reader files them under IFD0's context.
fn ascii(e: &Exif, number: u16) -> Option<String> {
    let f = e
        .fields()
        .find(|f| f.tag.number() == number && f.ifd_num == In::PRIMARY)?;
    match &f.value {
        exif::Value::Ascii(v) => v.first().map(|s| {
            String::from_utf8_lossy(s)
                .trim_end_matches('\0')
                .trim()
                .to_string()
        }),
        _ => None,
    }
}

/// The capture moment an EXIF block states: `DateTimeOriginal` as
/// `YYYY-MM-DD HH:MM:SS`, and `SubSecTimeOriginal` when present.
pub fn from_exif(e: &Exif) -> Option<Capture> {
    let raw = ascii(e, Tag::DateTimeOriginal.number())?;
    // EXIF writes `YYYY:MM:DD HH:MM:SS`; the date's colons become dashes
    let b = raw.as_bytes();
    if !raw.is_ascii() || b.len() < 19 || b[4] != b':' || b[7] != b':' || raw.starts_with("0000") {
        // a camera that has not been set writes zeros; that is not a time
        return None;
    }
    let datetime = format!("{}-{}-{}", &raw[..4], &raw[5..7], &raw[8..19]);
    let subsec = ascii(e, Tag::SubSecTimeOriginal.number()).filter(|s| !s.is_empty());
    Some(Capture { datetime, subsec })
}

/// `Some(true)` when both files state a capture time and they differ.
/// `Some(false)` when both state one and agree. `None` when either is
/// silent.
pub fn different_moments(a: Option<&Capture>, b: Option<&Capture>) -> Option<bool> {
    match (a, b) {
        (Some(x), Some(y)) => Some(!x.same_moment(y)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cap(dt: &str, sub: Option<&str>) -> Capture {
        Capture {
            datetime: dt.to_string(),
            subsec: sub.map(str::to_string),
        }
    }

    #[test]
    fn same_second_without_fractions_is_the_same_moment() {
        let a = cap("2019:06:12 10:11:12", None);
        let b = cap("2019:06:12 10:11:12", None);
        assert!(a.same_moment(&b));
        assert_eq!(different_moments(Some(&a), Some(&b)), Some(false));
    }

    #[test]
    fn fractions_separate_a_burst() {
        let a = cap("2019:06:12 10:11:12", Some("120"));
        let b = cap("2019:06:12 10:11:12", Some("450"));
        assert!(!a.same_moment(&b));
        // one side without a fraction cannot contradict the other
        let c = cap("2019:06:12 10:11:12", None);
        assert!(a.same_moment(&c));
        // trailing zeros are the same fraction
        let d = cap("2019:06:12 10:11:12", Some("12"));
        assert!(a.same_moment(&d));
    }

    #[test]
    fn different_seconds_are_different_moments() {
        let a = cap("2019:06:12 10:11:12", None);
        let b = cap("2019:06:12 10:11:14", None);
        assert_eq!(different_moments(Some(&a), Some(&b)), Some(true));
        assert_eq!(different_moments(Some(&a), None), None);
        assert_eq!(a.display(), "2019:06:12 10:11:12");
        assert_eq!(
            cap("2019-06-12 10:11:12", Some("45")).display(),
            "2019-06-12 10:11:12.45"
        );
    }

    /// A JPEG with a hand-built APP1 EXIF segment holding DateTimeOriginal
    /// and SubSecTimeOriginal; the reader must return exactly those.
    #[test]
    fn reads_exif_from_a_jpeg() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("shot.jpg");
        std::fs::write(&p, jpeg_with_exif("2021:03:04 05:06:07", Some("33"))).unwrap();
        let c = read(&p).expect("capture time");
        assert_eq!(c.datetime, "2021-03-04 05:06:07");
        assert_eq!(c.subsec.as_deref(), Some("33"));
        // no EXIF at all
        let q = dir.path().join("plain.jpg");
        std::fs::write(&q, jpeg_with_exif("", None)).unwrap();
        assert_eq!(read(&q), None);
    }

    /// Minimal baseline JPEG (one grey 8×8 block) with an optional EXIF
    /// APP1 segment: TIFF header, one IFD0 pointing at an Exif IFD that
    /// holds DateTimeOriginal (0x9003) and SubSecTimeOriginal (0x9291).
    pub(crate) fn jpeg_with_exif(datetime: &str, subsec: Option<&str>) -> Vec<u8> {
        let mut out = vec![0xFF, 0xD8];
        if !datetime.is_empty() {
            let mut tiff: Vec<u8> = Vec::new();
            tiff.extend_from_slice(b"MM\x00\x2A\x00\x00\x00\x08"); // big-endian, IFD0 at 8
                                                                   // IFD0: 1 entry (ExifIFD pointer 0x8769), next IFD 0
            let ifd0_len = 2 + 12 + 4;
            let exif_ifd_off = 8 + ifd0_len;
            tiff.extend_from_slice(&1u16.to_be_bytes());
            tiff.extend_from_slice(&0x8769u16.to_be_bytes());
            tiff.extend_from_slice(&4u16.to_be_bytes()); // LONG
            tiff.extend_from_slice(&1u32.to_be_bytes());
            tiff.extend_from_slice(&(exif_ifd_off as u32).to_be_bytes());
            tiff.extend_from_slice(&0u32.to_be_bytes());
            // Exif IFD
            let n_entries: u16 = if subsec.is_some() { 2 } else { 1 };
            let exif_ifd_len = 2 + 12 * n_entries as usize + 4;
            let data_off = exif_ifd_off + exif_ifd_len;
            let dt_bytes = format!("{datetime}\0");
            tiff.extend_from_slice(&n_entries.to_be_bytes());
            tiff.extend_from_slice(&0x9003u16.to_be_bytes());
            tiff.extend_from_slice(&2u16.to_be_bytes()); // ASCII
            tiff.extend_from_slice(&(dt_bytes.len() as u32).to_be_bytes());
            tiff.extend_from_slice(&(data_off as u32).to_be_bytes());
            if let Some(ss) = subsec {
                let mut v = [0u8; 4];
                let b = ss.as_bytes();
                v[..b.len()].copy_from_slice(b);
                tiff.extend_from_slice(&0x9291u16.to_be_bytes());
                tiff.extend_from_slice(&2u16.to_be_bytes());
                tiff.extend_from_slice(&((b.len() + 1) as u32).to_be_bytes());
                tiff.extend_from_slice(&v); // fits in the value field
            }
            tiff.extend_from_slice(&0u32.to_be_bytes());
            tiff.extend_from_slice(dt_bytes.as_bytes());
            let mut app1 = b"Exif\0\0".to_vec();
            app1.extend_from_slice(&tiff);
            out.extend_from_slice(&[0xFF, 0xE1]);
            out.extend_from_slice(&((app1.len() + 2) as u16).to_be_bytes());
            out.extend_from_slice(&app1);
        }
        // the smallest valid grey JPEG body: DQT, SOF0, DHT, SOS, data, EOI
        out.extend_from_slice(&[
            0xFF, 0xDB, 0x00, 0x43, 0x00, // DQT, 64 ones
        ]);
        out.extend(std::iter::repeat_n(1u8, 64));
        out.extend_from_slice(&[
            0xFF, 0xC0, 0x00, 0x0B, 0x08, 0x00, 0x08, 0x00, 0x08, 0x01, 0x01, 0x11, 0x00,
        ]);
        // DHT: DC table 0 with a single code (length 1) for category 0
        out.extend_from_slice(&[0xFF, 0xC4, 0x00, 0x14, 0x00, 0x01]);
        out.extend(std::iter::repeat_n(0u8, 15));
        out.push(0x00);
        // DHT: AC table 0 with a single code (length 1) for EOB (0x00)
        out.extend_from_slice(&[0xFF, 0xC4, 0x00, 0x14, 0x10, 0x01]);
        out.extend(std::iter::repeat_n(0u8, 15));
        out.push(0x00);
        out.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x08, 0x01, 0x01, 0x00, 0x00, 0x3F, 0x00]);
        out.push(0x00); // DC 0 (code "0"), AC EOB (code "0") -> bits 00, padded
        out.extend_from_slice(&[0xFF, 0xD9]);
        out
    }
}
