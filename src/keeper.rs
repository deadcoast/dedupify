//! Keeper policy (docs/RESEARCH.md §5): within a group the keeper is the
//! file with the earliest modification time; ties go to the shallowest
//! path, then the shorter file name, then lexicographic path.

use std::cmp::Ordering;
use std::path::Path;

use crate::exact::Group;
use crate::walk::Candidate;

pub const POLICY_LABEL: &str = "keep: oldest copy";

fn depth(p: &Path) -> usize {
    p.components().count()
}

fn name_len(p: &Path) -> usize {
    p.file_name().map(|n| n.len()).unwrap_or(0)
}

pub fn compare(a: &Candidate, b: &Candidate) -> Ordering {
    a.mtime
        .cmp(&b.mtime)
        .then_with(|| depth(&a.path).cmp(&depth(&b.path)))
        .then_with(|| name_len(&a.path).cmp(&name_len(&b.path)))
        .then_with(|| a.path.cmp(&b.path))
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
