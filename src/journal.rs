//! The journal (docs/RESEARCH.md §4): one JSON file per change, written
//! only by `milk` and `purge`, stored under the OS local-data directory,
//! never inside the scanned tree.
//!
//! `%LOCALAPPDATA%\dedupe\journal\<root-key>\<stamp>.json` on Windows,
//! `~/.local/share/dedupe/journal/<root-key>/<stamp>.json` elsewhere.
//! `<root-key>` is the first 16 hex digits of BLAKE3 over the canonical
//! root path, so one root's journals sit together.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::util::stamp;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    /// `milk`: renamed within the volume.
    Move { from: PathBuf, to: PathBuf },
    /// `purge`: sent to the system Recycle Bin at `time_deleted` (unix s).
    Trash { path: PathBuf, time_deleted: i64 },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Entry {
    #[serde(flatten)]
    pub op: Op,
    pub size: u64,
    pub hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Journal {
    pub id: String,
    pub root: PathBuf,
    pub command: String,
    pub created_utc: String,
    pub consumed: bool,
    pub entries: Vec<Entry>,
}

impl Journal {
    pub fn new(root: &Path, command: &str) -> Journal {
        let now = SystemTime::now();
        let s = stamp(now);
        Journal {
            id: s.clone(),
            root: root.to_path_buf(),
            command: command.to_string(),
            created_utc: s,
            consumed: false,
            entries: Vec::new(),
        }
    }
}

/// Override for tests and for users who want journals elsewhere.
pub const ENV_JOURNAL_DIR: &str = "DEDUPE_JOURNAL_DIR";

pub fn base_dir() -> PathBuf {
    if let Ok(p) = std::env::var(ENV_JOURNAL_DIR) {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("dedupe")
        .join("journal")
}

pub fn root_key(root: &Path) -> String {
    let h = blake3::hash(root.to_string_lossy().as_bytes());
    h.to_hex()[..16].to_string()
}

pub fn dir_for(root: &Path) -> PathBuf {
    base_dir().join(root_key(root))
}

/// Write `j`; the file name is the journal id. A second journal in the
/// same second gets a `-2`, `-3` … suffix.
pub fn write(j: &mut Journal) -> io::Result<PathBuf> {
    let dir = dir_for(&j.root);
    fs::create_dir_all(&dir)?;
    let mut path = dir.join(format!("{}.json", j.id));
    let mut n = 1;
    while path.exists() {
        n += 1;
        j.id = format!("{}-{n}", j.created_utc);
        path = dir.join(format!("{}.json", j.id));
    }
    let text = serde_json::to_string_pretty(j).map_err(|e| io::Error::other(e.to_string()))?;
    fs::write(&path, text)?;
    Ok(path)
}

pub fn read(path: &Path) -> io::Result<Journal> {
    let text = fs::read_to_string(path)?;
    serde_json::from_str(&text).map_err(|e| io::Error::other(e.to_string()))
}

/// All journals for `root`, newest first (by file name, which is the stamp).
pub fn list(root: &Path) -> io::Result<Vec<(PathBuf, Journal)>> {
    let dir = dir_for(root);
    let mut out = Vec::new();
    let rd = match fs::read_dir(&dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(e),
    };
    for e in rd {
        let e = e?;
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) == Some("json") {
            if let Ok(j) = read(&p) {
                out.push((p, j));
            }
        }
    }
    out.sort_by_key(|(_, j)| std::cmp::Reverse(id_key(&j.id)));
    Ok(out)
}

/// `YYYYMMDD-HHMMSS[-N]` → (stamp, N); N defaults to 1 so a same-second
/// `-2` journal sorts as newer than the bare stamp.
fn id_key(id: &str) -> (String, u32) {
    let stamp_len = "YYYYMMDD-HHMMSS".len();
    if id.len() > stamp_len {
        let n = id[stamp_len..]
            .trim_start_matches('-')
            .parse::<u32>()
            .unwrap_or(1);
        (id[..stamp_len].to_string(), n)
    } else {
        (id.to_string(), 1)
    }
}

/// The newest journal that has not been undone.
pub fn latest_pending(root: &Path) -> io::Result<Option<(PathBuf, Journal)>> {
    Ok(list(root)?.into_iter().find(|(_, j)| !j.consumed))
}

pub fn mark_consumed(path: &Path, j: &mut Journal) -> io::Result<()> {
    j.consumed = true;
    let text = serde_json::to_string_pretty(j).map_err(|e| io::Error::other(e.to_string()))?;
    fs::write(path, text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_list_consume_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var(ENV_JOURNAL_DIR, tmp.path());
        let root = tmp.path().join("root");
        fs::create_dir_all(&root).unwrap();

        let mut j1 = Journal::new(&root, "milk");
        j1.entries.push(Entry {
            op: Op::Move {
                from: root.join("a"),
                to: root.join("_milked/a"),
            },
            size: 3,
            hash: "abc".into(),
        });
        let p1 = write(&mut j1).unwrap();
        let mut j2 = Journal::new(&root, "purge");
        j2.entries.push(Entry {
            op: Op::Trash {
                path: root.join("b"),
                time_deleted: 7,
            },
            size: 3,
            hash: "def".into(),
        });
        let p2 = write(&mut j2).unwrap();
        assert_ne!(p1, p2, "same-second journals must not collide");

        let (lp, mut lj) = latest_pending(&root).unwrap().unwrap();
        assert_eq!(lp, p2);
        assert_eq!(lj.command, "purge");
        mark_consumed(&lp, &mut lj).unwrap();
        let (lp, _) = latest_pending(&root).unwrap().unwrap();
        assert_eq!(lp, p1);

        let back = read(&p2).unwrap();
        assert!(back.consumed);
        assert_eq!(back.entries, j2.entries);
        std::env::remove_var(ENV_JOURNAL_DIR);
    }
}
