//! Keeper policy (docs/RESEARCH.md §5): within a group the keeper is the
//! file that does not look derived (CORE-07, Phase 11), then the one with
//! the earliest modification time; ties go to the shallowest path, then
//! the shorter file name, then lexicographic path.
//!
//! A modification time says when a file was last written, which a
//! download, a sync or a copy tool resets: a copy can be "older" than its
//! original. A copy's name and folder are written by the tool that made
//! it, and they say what it is. [CLAUDE, Phase 11]

use std::cmp::Ordering;
use std::path::Path;

use crate::exact::Group;
use crate::walk::Candidate;

pub const POLICY_LABEL: &str = "keep: not named or filed as a copy, then oldest";

/// Folder names a copy is filed under: exports, downloads, temporary
/// files, caches, thumbnails, backups.
const DERIVED_FOLDERS: &[&str] = &[
    "export",
    "exports",
    "exported",
    "download",
    "downloads",
    "temp",
    "tmp",
    "cache",
    ".cache",
    "thumbnails",
    ".thumbnails",
    "thumbs",
    "backup",
    "backups",
    "copies",
];

/// How derived a path looks: one mark for a copy's name, and one for each
/// copy's folder on the path. Names: `x (1)`, `x(2)`, `x - Copy`,
/// `x copy`, `x_copy`, `Copy of x`, `x-export`, `x_export`, `x-edited`
/// (case ignored). Folders are counted, not merely noticed: a folder above
/// the scanned root (a scan inside `Downloads` or a temporary folder)
/// adds the same mark to every file in a group, and the `exports` below it
/// still tells the copy from the original.
pub fn derived(p: &Path) -> u8 {
    let stem = p
        .file_stem()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let words: Vec<&str> = stem
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    let numbered = stem.trim_end().ends_with(')')
        && stem
            .trim_end()
            .trim_end_matches(')')
            .rsplit('(')
            .next()
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
    let name = numbered
        || words
            .iter()
            .any(|w| matches!(*w, "copy" | "export" | "exported" | "edited"));
    let folders = p
        .parent()
        .map(|d| {
            d.components()
                .filter(|c| {
                    let c = c.as_os_str().to_string_lossy().to_lowercase();
                    DERIVED_FOLDERS.contains(&c.as_str())
                })
                .count()
        })
        .unwrap_or(0);
    (name as usize + folders).min(u8::MAX as usize) as u8
}

pub fn compare(a: &Candidate, b: &Candidate) -> Ordering {
    derived(&a.path)
        .cmp(&derived(&b.path))
        .then_with(|| a.mtime.cmp(&b.mtime))
        .then_with(|| depth(&a.path).cmp(&depth(&b.path)))
        .then_with(|| name_len(&a.path).cmp(&name_len(&b.path)))
        .then_with(|| a.path.cmp(&b.path))
}

fn depth(p: &Path) -> usize {
    p.components().count()
}

fn name_len(p: &Path) -> usize {
    p.file_name().map(|n| n.len()).unwrap_or(0)
}

/// Put the keeper at index 0 of every group's members.
pub fn order(groups: &mut [Group]) {
    for g in groups.iter_mut() {
        g.members.sort_by(compare);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::{Duration, UNIX_EPOCH};

    fn c(p: &str, secs: u64) -> Candidate {
        Candidate {
            path: PathBuf::from(p),
            size: 1,
            mtime: UNIX_EPOCH + Duration::from_secs(secs),
        }
    }

    #[test]
    fn a_copy_by_name_or_folder_never_keeps() {
        // the copy is older: a download or a sync reset the original's time
        let mut v = [
            c("photos/exports/graf1_mirror.png", 100),
            c("photos/graf1.png", 200),
        ];
        v.sort_by(compare);
        assert!(v[0].path.ends_with("photos/graf1.png"));
        for (copy, original) in [
            ("a/x (1).jpg", "a/x.jpg"),
            ("a/x(2).jpg", "a/x.jpg"),
            ("a/x - Copy.jpg", "a/x.jpg"),
            ("a/vtest - Copy.avi", "a/vtest.avi"),
            ("a/Copy of x.jpg", "a/x.jpg"),
            ("a/home-copy.jpg", "a/home.jpg"),
            ("a/x_export.jpg", "a/x.jpg"),
            ("a/IMG_0012-edited.jpg", "a/IMG_0012.jpg"),
            ("Downloads/x.jpg", "Pictures/x.jpg"),
            ("a/tmp/x.jpg", "a/x.jpg"),
            ("Backup/2019/x.jpg", "2019/x.jpg"),
        ] {
            let mut v = [c(copy, 1), c(original, 2)];
            v.sort_by(compare);
            assert_eq!(v[0].path, PathBuf::from(original), "{copy}");
        }
        // names that only look alike
        for name in [
            "a/copyright.jpg",
            "a/x (a).jpg",
            "a/x ().jpg",
            "a/IMG_0012.jpg",
            "a/exports.jpg",
        ] {
            assert_eq!(derived(Path::new(name)), 0, "{name}");
        }
        assert_eq!(derived(Path::new("Downloads/x (1).jpg")), 2);
        // a scan inside a temporary or downloads folder: the folder above
        // marks both, the exports below still decides
        let mut v = [
            c("/tmp/scan/photos/exports/graf1_mirror.png", 100),
            c("/tmp/scan/photos/graf1.png", 200),
        ];
        v.sort_by(compare);
        assert!(v[0].path.ends_with("photos/graf1.png"));
        assert_eq!(derived(Path::new("/tmp/Downloads/exports/x.jpg")), 3);
    }

    #[test]
    fn oldest_wins_then_shallowest_then_shortest() {
        let mut v = [
            c("backup/old/IMG_0041 (1).jpg", 200),
            c("2019/trip/IMG_0041.jpg", 100),
            c("backup/IMG_0041.jpg", 200),
        ];
        v.sort_by(compare);
        assert!(v[0].path.ends_with("2019/trip/IMG_0041.jpg"));
        assert!(v[1].path.ends_with("backup/IMG_0041.jpg"));

        let mut v = [c("a/b/x.jpg", 5), c("a/x.jpg", 5)];
        v.sort_by(compare);
        assert_eq!(v[0].path, PathBuf::from("a/x.jpg"));

        let mut v = [c("a/x (1).jpg", 5), c("a/x.jpg", 5)];
        v.sort_by(compare);
        assert_eq!(v[0].path, PathBuf::from("a/x.jpg"));
    }
}
