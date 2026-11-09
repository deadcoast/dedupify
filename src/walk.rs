//! Discovery: turn a root into candidates.
//!
//! Rules: never follow symlinks or junctions; never enter `_milked`;
//! zero-byte files are not candidates (a thousand empty files are not a
//! thousand copies of anything worth moving); directory metadata supplies
//! size and modification time, so this stage opens no file. A folder or
//! file whose name is on the skip list is passed over whole (`skip`), and
//! files under the size floor are not candidates (`--min`).

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use walkdir::WalkDir;

use crate::kind::Kind;
use crate::MILKED_DIR;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub path: PathBuf,
    pub size: u64,
    pub mtime: SystemTime,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct ScanStats {
    /// Files that became candidates.
    pub files: u64,
    /// Their total bytes.
    pub bytes: u64,
    pub dirs: u64,
    /// Files passed over by the kind filter.
    pub filtered: u64,
    /// Zero-byte files passed over.
    pub empty: u64,
    /// Entries that could not be read (permission, vanished mid-scan).
    pub errors: u64,
    /// Folders and files passed over by name (a skipped folder counts once).
    pub skipped: u64,
    /// Files under the size floor, and the floor.
    pub small: u64,
    pub min: u64,
}

/// What the scan leaves out besides `_milked`: names on the skip list, and
/// files under a size floor.
#[derive(Debug, Clone, Copy, Default)]
pub struct Filter<'a> {
    pub skip: &'a [String],
    pub min: u64,
}

/// `_milked` by name; case-insensitive on Windows, where the file system
/// is, so a hand-renamed `_Milked` is still never scanned.
fn is_milked_dir(name: &std::ffi::OsStr) -> bool {
    if cfg!(windows) {
        name.to_string_lossy().eq_ignore_ascii_case(MILKED_DIR)
    } else {
        name == MILKED_DIR
    }
}

/// Walk `root`. `recurse = false` reads only the root's own entries.
/// `progress` is called every 256 files with the running stats.
pub fn scan(
    root: &Path,
    recurse: bool,
    kind: Kind,
    filter: Filter<'_>,
    progress: &mut dyn FnMut(&ScanStats),
) -> (Vec<Candidate>, ScanStats) {
    let mut stats = ScanStats {
        min: filter.min,
        ..Default::default()
    };
    let mut out: Vec<Candidate> = Vec::new();
    let depth = if recurse { usize::MAX } else { 1 };
    let mut seen: u64 = 0;
    let skipped = std::cell::Cell::new(0u64);

    let walker = WalkDir::new(root)
        .follow_links(false)
        .max_depth(depth)
        .into_iter()
        .filter_entry(|e| {
            if e.depth() == 0 {
                return true; // the root itself is never skipped
            }
            if e.file_type().is_dir() && is_milked_dir(e.file_name()) {
                return false;
            }
            if crate::skip::skipped(filter.skip, e.file_name()) {
                skipped.set(skipped.get() + 1);
                return false;
            }
            true
        });

    for entry in walker {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => {
                stats.errors += 1;
                continue;
            }
        };
        let ft = entry.file_type();
        if ft.is_dir() {
            stats.dirs += 1;
            continue;
        }
        if !ft.is_file() {
            // symlink, device, pipe — never a candidate
            continue;
        }
        seen += 1;
        if !kind.matches(entry.path()) {
            stats.filtered += 1;
        } else {
            match entry.metadata() {
                Ok(md) => {
                    let size = md.len();
                    if size == 0 {
                        stats.empty += 1;
                    } else if size < filter.min {
                        stats.small += 1;
                    } else {
                        let mtime = md.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                        stats.files += 1;
                        stats.bytes += size;
                        out.push(Candidate {
                            path: entry.into_path(),
                            size,
                            mtime,
                        });
                    }
                }
                Err(_) => stats.errors += 1,
            }
        }
        if seen.is_multiple_of(256) {
            stats.skipped = skipped.get();
            progress(&stats);
        }
    }
    stats.skipped = skipped.get();
    progress(&stats);
    (out, stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn mk(dir: &Path, rel: &str, bytes: &[u8]) {
        let p = dir.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, bytes).unwrap();
    }

    #[test]
    fn top_level_vs_recursive() {
        let d = tempfile::tempdir().unwrap();
        mk(d.path(), "a.txt", b"aaa");
        mk(d.path(), "sub/b.txt", b"bbb");
        mk(d.path(), "sub/deeper/c.txt", b"ccc");
        let (top, s) = scan(d.path(), false, Kind::All, Filter::default(), &mut |_| {});
        assert_eq!(top.len(), 1);
        assert_eq!(s.files, 1);
        let (all, s) = scan(d.path(), true, Kind::All, Filter::default(), &mut |_| {});
        assert_eq!(all.len(), 3);
        assert_eq!(s.bytes, 9);
    }

    #[test]
    fn skips_milked_empty_and_filtered() {
        let d = tempfile::tempdir().unwrap();
        mk(d.path(), "x.jpg", b"img");
        mk(d.path(), "y.mp4", b"vid");
        mk(d.path(), "empty.jpg", b"");
        mk(d.path(), "_milked/old.jpg", b"img");
        mk(d.path(), "sub/_milked/old2.jpg", b"img");
        let (imgs, s) = scan(d.path(), true, Kind::Image, Filter::default(), &mut |_| {});
        assert_eq!(imgs.len(), 1);
        assert!(imgs[0].path.ends_with("x.jpg"));
        assert_eq!(s.filtered, 1);
        assert_eq!(s.empty, 1);
    }

    #[test]
    fn skip_names_and_the_size_floor() {
        let d = tempfile::tempdir().unwrap();
        mk(d.path(), "keep/a.jpg", &[1; 5000]);
        mk(d.path(), "keep/thumb.jpg", &[1; 50]);
        mk(d.path(), "node_modules/x/b.jpg", &[2; 5000]);
        mk(d.path(), "keep/c.tmp", &[3; 5000]);
        let skip = vec!["node_modules".to_string(), "*.tmp".to_string()];
        let f = Filter {
            skip: &skip,
            min: 1000,
        };
        let (c, s) = scan(d.path(), true, Kind::All, f, &mut |_| {});
        assert_eq!(c.len(), 1, "{c:?}");
        assert!(c[0].path.ends_with("a.jpg"));
        assert_eq!(s.skipped, 2, "the folder once, the file once");
        assert_eq!(s.small, 1);
        // the root itself is never skipped, whatever its name
        let skip_root = vec![d.path().file_name().unwrap().to_string_lossy().into_owned()];
        let (c, _) = scan(
            d.path(),
            true,
            Kind::All,
            Filter {
                skip: &skip_root,
                min: 0,
            },
            &mut |_| {},
        );
        assert_eq!(c.len(), 4);
    }
}
