//! File kind filter: the `img` / `vid` positional tokens.

use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    All,
    Image,
    Video,
}

/// Extensions treated as images. Lower-case, no dot: raster formats, the
/// HEIF family and camera RAWs (`decode` has a route for each).
pub const IMAGE_EXT: &[&str] = &[
    "jpg", "jpeg", "jpe", "jfif", "png", "gif", "webp", "bmp", "tif", "tiff", "heic", "heif",
    "hif", "avif", "jxl", "psd", "ico", "dng", "cr2", "cr3", "crw", "nef", "nrw", "arw", "sr2",
    "srf", "orf", "rw2", "rwl", "raf", "pef", "srw", "x3f", "3fr", "fff", "iiq", "erf", "kdc",
    "dcr", "mrw", "mos",
];

/// Camera RAW extensions: decoded through their embedded preview, and
/// never acted on by the near tier (`raw`, `near`). [CLAUDE, Phase 10]
pub const RAW_EXT: &[&str] = &[
    "dng", "cr2", "cr3", "crw", "nef", "nrw", "arw", "sr2", "srf", "orf", "rw2", "rwl", "raf",
    "pef", "srw", "x3f", "3fr", "fff", "iiq", "erf", "kdc", "dcr", "mrw", "mos",
];

/// A camera RAW, by extension.
pub fn is_raw(path: &Path) -> bool {
    has_ext(path, RAW_EXT)
}

/// Extensions treated as videos.
pub const VIDEO_EXT: &[&str] = &[
    "mp4", "m4v", "mkv", "mov", "avi", "webm", "wmv", "flv", "mpg", "mpeg", "ts", "m2ts", "mts",
    "3gp", "ogv", "vob", "divx",
];

impl Kind {
    /// Parse a positional token. `None` when the token is not a kind word.
    pub fn from_token(tok: &str) -> Option<Kind> {
        match tok.to_ascii_lowercase().as_str() {
            "img" | "image" | "images" => Some(Kind::Image),
            "vid" | "video" | "videos" => Some(Kind::Video),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Kind::All => "all kinds",
            Kind::Image => "images",
            Kind::Video => "videos",
        }
    }

    /// The token that reproduces this kind on the command line (empty for All).
    pub fn token(self) -> &'static str {
        match self {
            Kind::All => "",
            Kind::Image => "img",
            Kind::Video => "vid",
        }
    }

    pub fn matches(self, path: &Path) -> bool {
        match self {
            Kind::All => true,
            Kind::Image => has_ext(path, IMAGE_EXT),
            Kind::Video => has_ext(path, VIDEO_EXT),
        }
    }
}

pub(crate) fn has_ext(path: &Path, set: &[&str]) -> bool {
    match path.extension().and_then(|e| e.to_str()) {
        Some(e) => {
            let e = e.to_ascii_lowercase();
            set.iter().any(|s| *s == e)
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_parse() {
        assert_eq!(Kind::from_token("img"), Some(Kind::Image));
        assert_eq!(Kind::from_token("IMAGE"), Some(Kind::Image));
        assert_eq!(Kind::from_token("vid"), Some(Kind::Video));
        assert_eq!(Kind::from_token("video"), Some(Kind::Video));
        assert_eq!(Kind::from_token("imgs"), None);
        assert_eq!(Kind::from_token("."), None);
    }

    #[test]
    fn extension_match_is_case_insensitive() {
        assert!(Kind::Image.matches(Path::new("a/B.JPG")));
        assert!(Kind::Video.matches(Path::new("x.MkV")));
        assert!(!Kind::Image.matches(Path::new("x.mkv")));
        assert!(!Kind::Video.matches(Path::new("noext")));
        assert!(Kind::All.matches(Path::new("anything.bin")));
    }
}
