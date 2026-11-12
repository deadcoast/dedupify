//! Discovery: turn a root into candidates.
//!
//! Rules: never follow symlinks or junctions; never enter `_milked`, nor
//! the operating system's trash and system folders (`system_folder`);
//! never read a file a sync client keeps online only (`online_only`);
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
    /// The operating system's trash and system folders passed over.
    pub system: u64,
    /// Files of the kind asked for that are online only, left unread.
    pub online: u64,
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

/// The operating system's own folders, never entered (Phase 13, CORE-09).
/// A file in the trash is one the person already threw away: elected a
/// group's keeper (it was the oldest copy), it made the live file the
/// duplicate, which `milk` would move and `purge` would bin beside it. The
/// system's folders hold restore points and indexes, and refuse reading.
/// Windows' names on every system, letter case ignored (a drive formatted
/// there carries them wherever it is read); the Unix trashes by name
/// (`.Trash`, `.Trashes`, `.Trash-1000`) and the person's own freedesktop
/// trash by its path. [CLAUDE]
pub fn system_folder(path: &Path, name: &std::ffi::OsStr) -> bool {
    const WINDOWS: [&str; 4] = [
        "$RECYCLE.BIN",
        "RECYCLER",
        "RECYCLED",
        "System Volume Information",
    ];
    let n = name.to_string_lossy();
    if WINDOWS.iter().any(|w| n.eq_ignore_ascii_case(w)) || n == ".Trash" || n == ".Trashes" {
        return true;
    }
    if let Some(uid) = n.strip_prefix(".Trash-") {
        if !uid.is_empty() && uid.bytes().all(|b| b.is_ascii_digit()) {
            return true;
        }
    }
    home_trash().is_some_and(|t| t == path)
}

/// Windows' marks of a file whose bytes are elsewhere: `OFFLINE`
/// (archival storage), `RECALL_ON_OPEN` and `RECALL_ON_DATA_ACCESS` (a sync
/// client's placeholder: OneDrive's Files On-Demand, Dropbox, iCloud for
/// Windows). Reading such a file fetches it.
pub const FILE_ATTRIBUTE_OFFLINE: u32 = 0x0000_1000;
pub const FILE_ATTRIBUTE_RECALL_ON_OPEN: u32 = 0x0004_0000;
pub const FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS: u32 = 0x0040_0000;
/// macOS's mark of a file whose bytes are in iCloud Drive (`SF_DATALESS`).
pub const SF_DATALESS: u32 = 0x4000_0000;

/// By Windows file attributes: are the bytes elsewhere?
pub fn online_only_attributes(attributes: u32) -> bool {
    attributes
        & (FILE_ATTRIBUTE_OFFLINE
            | FILE_ATTRIBUTE_RECALL_ON_OPEN
            | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS)
        != 0
}

/// Online only: a sync client keeps the file's bytes in its cloud, and
/// reading them, even the 64 KiB a sample takes, downloads the file
/// (Phase 13, CORE-10). Such a file is not a candidate; the SCAN line
/// counts it. The walk's own metadata says so (Windows: the directory
/// listing's attributes; macOS: `lstat`'s flags), so nothing is opened to
/// ask. Elsewhere no file system marks it. [CLAUDE]
pub fn online_only(md: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        online_only_attributes(md.file_attributes())
    }
    #[cfg(target_os = "macos")]
    {
        use std::os::macos::fs::MetadataExt;
        md.st_flags() & SF_DATALESS != 0
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = md;
        false
    }
}

/// The freedesktop trash of the person running this (`$XDG_DATA_HOME/Trash`,
/// by default `~/.local/share/Trash`), as the walk will meet it: resolved.
/// macOS keeps its trash in `~/.Trash`, by name above; Windows in
/// `$RECYCLE.BIN`.
fn home_trash() -> Option<&'static PathBuf> {
    static TRASH: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    TRASH
        .get_or_init(|| {
            if cfg!(any(windows, target_os = "macos")) {
                return None;
            }
            let t = dirs::data_dir()?.join("Trash");
            Some(crate::util::strip_verbatim(&t.canonicalize().unwrap_or(t)))
        })
        .as_ref()
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
    let system = std::cell::Cell::new(0u64);

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
            if e.file_type().is_dir() && system_folder(e.path(), e.file_name()) {
                system.set(system.get() + 1);
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
                Ok(md) if online_only(&md) => stats.online += 1,
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
            stats.system = system.get();
            progress(&stats);
        }
    }
    stats.skipped = skipped.get();
    stats.system = system.get();
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
    fn the_bin_and_the_system_folders_are_never_entered() {
        let d = tempfile::tempdir().unwrap();
        mk(d.path(), "photos/IMG_0041.jpg", &[7; 3000]);
        for bin in [
            "$RECYCLE.BIN/S-1-5-21-1000/$RX4K2P1.jpg",
            "$Recycle.Bin/S-1-5-21-1000/$RX4K2P2.jpg",
            "RECYCLER/x.jpg",
            "System Volume Information/x.jpg",
            ".Trash/x.jpg",
            ".Trashes/501/x.jpg",
            ".Trash-1000/files/IMG_0041.jpg",
            "sub/.Trash-0/files/y.jpg",
        ] {
            mk(d.path(), bin, &[7; 3000]);
        }
        // names like them that are not them
        mk(d.path(), ".Trash-me/a.jpg", &[7; 3000]);
        mk(d.path(), "Trash/b.jpg", &[7; 3000]);
        let (c, s) = scan(d.path(), true, Kind::All, Filter::default(), &mut |_| {});
        let mut names: Vec<String> = c
            .iter()
            .map(|c| {
                c.path
                    .strip_prefix(d.path())
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();
        names.sort();
        assert_eq!(
            names,
            [".Trash-me/a.jpg", "Trash/b.jpg", "photos/IMG_0041.jpg"]
        );
        // on Windows the two letter cases of the bin are one folder
        assert_eq!(s.system, if cfg!(windows) { 7 } else { 8 });
        // a file of that name is not a folder: a candidate as any other
        assert!(!system_folder(
            Path::new("/x/notes"),
            std::ffi::OsStr::new("notes")
        ));
    }

    #[test]
    fn the_marks_of_a_file_kept_online() {
        // a OneDrive placeholder as Windows lists it: archive, unpinned,
        // recall on data access
        assert!(online_only_attributes(
            0x0000_0020 | 0x0010_0000 | 0x0040_0000
        ));
        assert!(online_only_attributes(FILE_ATTRIBUTE_RECALL_ON_OPEN));
        assert!(online_only_attributes(FILE_ATTRIBUTE_OFFLINE));
        // local: normal, archive, read-only, hidden, pinned (kept on the
        // device), sparse, compressed
        for local in [0x80, 0x20, 0x01, 0x02, 0x0008_0000 | 0x20, 0x200, 0x800] {
            assert!(!online_only_attributes(local), "{local:#x}");
        }
        // a file this test wrote is local everywhere
        let d = tempfile::tempdir().unwrap();
        mk(d.path(), "a.jpg", b"local");
        assert!(!online_only(&fs::metadata(d.path().join("a.jpg")).unwrap()));
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
