//! The journal (docs/RESEARCH.md §4): one JSON file per change, written
//! only by `milk` and `purge`, stored under the OS local-data directory,
//! never inside the scanned tree. While a change runs, `Writer` keeps the
//! file current (atomic replace, at most every 250 ms) so an interrupted
//! or killed run still leaves `--undo` everything it did.
//!
//! `%LOCALAPPDATA%\dedupe\journal\<root-key>\<stamp>.json` on Windows,
//! `~/.local/share/dedupe/journal/<root-key>/<stamp>.json` elsewhere.
//! `<root-key>` is the first 16 hex digits of BLAKE3 over the canonical
//! root path, so one root's journals sit together.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use serde::{Deserialize, Serialize};

use crate::util::stamp;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    /// `milk`: renamed within the volume, or (`copied`) across volumes:
    /// copied, verified, the original deleted (Phase 10).
    Move {
        from: PathBuf,
        to: PathBuf,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        copied: bool,
    },
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

/// `%LOCALAPPDATA%\dedupe` on Windows, `~/.local/share/dedupe` elsewhere:
/// the one folder `dedupe` owns outside the scanned tree (journals, and
/// the optional model under `models/`).
pub fn app_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("dedupe")
}

pub fn base_dir() -> PathBuf {
    if let Ok(p) = std::env::var(ENV_JOURNAL_DIR) {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    app_dir().join("journal")
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
    rewrite(&path, j)?;
    Ok(path)
}

/// How often the journal on disk is brought up to date while a change
/// runs, so a hard kill (task manager, power) loses at most this much.
pub const FLUSH_EVERY: Duration = Duration::from_millis(250);

/// A journal being written as a change runs. Nothing touches the disk
/// until the first entry; after that the file is rewritten at most every
/// `FLUSH_EVERY`, and once more by `finish`. One run, one file.
pub struct Writer {
    pub journal: Journal,
    path: Option<PathBuf>,
    last_flush: Instant,
    /// First error from a flush; reported once by `finish`.
    error: Option<io::Error>,
}

impl Writer {
    pub fn new(root: &Path, command: &str) -> Writer {
        Writer {
            journal: Journal::new(root, command),
            path: None,
            last_flush: Instant::now(),
            error: None,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.journal.entries.is_empty()
    }

    pub fn record(&mut self, e: Entry) {
        self.journal.entries.push(e);
        if self.path.is_none() || self.last_flush.elapsed() >= FLUSH_EVERY {
            self.flush();
        }
    }

    fn flush(&mut self) {
        if self.journal.entries.is_empty() {
            return;
        }
        let res = match &self.path {
            Some(p) => rewrite(p, &self.journal),
            None => write(&mut self.journal).map(|p| {
                self.path = Some(p);
            }),
        };
        if let Err(e) = res {
            if self.error.is_none() {
                self.error = Some(e);
            }
        }
        self.last_flush = Instant::now();
    }

    /// Final write. `Ok(None)` when nothing was recorded; `Ok(Some(id))`
    /// when the journal is on disk; `Err` when it could not be written.
    pub fn finish(mut self) -> io::Result<Option<String>> {
        if self.journal.entries.is_empty() {
            return Ok(None);
        }
        self.flush();
        match self.error {
            Some(e) => Err(e),
            None => Ok(Some(self.journal.id.clone())),
        }
    }
}

/// Replace `path` atomically: the new text goes to a sibling `.tmp`, is
/// flushed to the disk, and then takes the journal's name in one rename —
/// so a kill mid-write leaves the previous journal intact, and so does a
/// power cut: the name never points at text still in memory (Phase 13,
/// TECH-09). On Unix the folder is flushed after, which holds the rename.
fn rewrite(path: &Path, j: &Journal) -> io::Result<()> {
    use std::io::Write as _;
    let text = serde_json::to_string_pretty(j).map_err(|e| io::Error::other(e.to_string()))?;
    let tmp = path.with_extension("json.tmp");
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    #[cfg(unix)]
    if let Some(dir) = path.parent() {
        if let Ok(d) = fs::File::open(dir) {
            let _ = d.sync_all();
        }
    }
    Ok(())
}

pub fn read(path: &Path) -> io::Result<Journal> {
    let text = fs::read_to_string(path)?;
    serde_json::from_str(&text).map_err(|e| io::Error::other(e.to_string()))
}

/// All journals for `root`, newest first (by file name, which is the stamp).
pub fn list(root: &Path) -> io::Result<Vec<(PathBuf, Journal)>> {
    Ok(list_with_unreadable(root)?.0)
}

/// `list`, plus how many `.json` files in the folder could not be parsed —
/// a truncated or hand-edited journal is reported by `doctor`, never
/// silently passed over.
pub fn list_with_unreadable(root: &Path) -> io::Result<(Vec<(PathBuf, Journal)>, u64)> {
    let dir = dir_for(root);
    let mut out = Vec::new();
    let mut unreadable = 0;
    let rd = match fs::read_dir(&dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok((out, 0)),
        Err(e) => return Err(e),
    };
    for e in rd {
        let e = e?;
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) == Some("json") {
            match read(&p) {
                Ok(j) => out.push((p, j)),
                Err(_) => unreadable += 1,
            }
        }
    }
    out.sort_by_key(|(_, j)| std::cmp::Reverse(id_key(&j.id)));
    Ok((out, unreadable))
}

/// Every root with a pending journal, newest first: (root, command, entries).
/// For the `--undo` that finds nothing under its own root.
pub fn pending_elsewhere(except: &Path) -> Vec<(PathBuf, String, usize)> {
    let mut out: Vec<(String, PathBuf, String, usize)> = Vec::new();
    let Ok(rd) = fs::read_dir(base_dir()) else {
        return Vec::new();
    };
    for sub in rd.flatten() {
        let Ok(files) = fs::read_dir(sub.path()) else {
            continue;
        };
        let mut newest: Option<(String, Journal)> = None;
        for f in files.flatten() {
            let p = f.path();
            if p.extension().and_then(|x| x.to_str()) != Some("json") {
                continue;
            }
            if let Ok(j) = read(&p) {
                if !j.consumed && j.root != except {
                    let newer = newest
                        .as_ref()
                        .is_none_or(|(id, _)| id_key(&j.id) > id_key(id));
                    if newer {
                        newest = Some((j.id.clone(), j));
                    }
                }
            }
        }
        if let Some((id, j)) = newest {
            out.push((id, j.root, j.command, j.entries.len()));
        }
    }
    out.sort_by_key(|(id, _, _, _)| std::cmp::Reverse(id_key(id)));
    out.into_iter().map(|(_, r, c, n)| (r, c, n)).collect()
}

/// Rewrite `path` with only the entries not yet undone, leaving the
/// journal pending so the next `--undo` picks up where this one stopped.
pub fn keep_remaining(path: &Path, j: &Journal, remaining: Vec<Entry>) -> io::Result<()> {
    let mut rest = j.clone();
    rest.entries = remaining;
    rest.consumed = false;
    rewrite(path, &rest)
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
    rewrite(path, j)
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
                copied: true,
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
        // a cross-volume move says so; a rename does not write the field,
        // and a journal from before Phase 10 (no field) reads as a rename
        assert_eq!(read(&p1).unwrap().entries, j1.entries);
        let old: Entry = serde_json::from_str(
            r#"{"op":"move","from":"/r/a","to":"/r/_milked/a","size":1,"hash":""}"#,
        )
        .unwrap();
        assert!(matches!(old.op, Op::Move { copied: false, .. }));
        let txt = serde_json::to_string(&Entry {
            op: Op::Move {
                from: "/r/a".into(),
                to: "/r/_milked/a".into(),
                copied: false,
            },
            size: 1,
            hash: String::new(),
        })
        .unwrap();
        assert!(!txt.contains("copied"), "{txt}");

        // the writer: nothing on disk before the first entry, one file after
        let before = list(&root).unwrap().len();
        let w = Writer::new(&root, "milk");
        assert!(w.is_empty());
        assert_eq!(w.finish().unwrap(), None);
        assert_eq!(list(&root).unwrap().len(), before);
        let mut w = Writer::new(&root, "milk");
        for i in 0..5 {
            w.record(Entry {
                op: Op::Move {
                    from: root.join(format!("f{i}")),
                    to: root.join(format!("_milked/f{i}")),
                    copied: false,
                },
                size: 1,
                hash: String::new(),
            });
        }
        let id = w.finish().unwrap().unwrap();
        let all = list(&root).unwrap();
        assert_eq!(all.len(), before + 1);
        let (lp, lj) = latest_pending(&root).unwrap().unwrap();
        assert_eq!(lj.id, id);
        assert_eq!(lj.entries.len(), 5);
        assert!(
            !lp.with_extension("json.tmp").exists(),
            "no temp file left behind"
        );

        // keep_remaining leaves a shorter, still pending journal
        keep_remaining(&lp, &lj, lj.entries[3..].to_vec()).unwrap();
        let (_, again) = latest_pending(&root).unwrap().unwrap();
        assert_eq!(again.entries.len(), 2);

        // an unreadable file is counted, not hidden
        fs::write(dir_for(&root).join("broken.json"), b"{").unwrap();
        let (_, unreadable) = list_with_unreadable(&root).unwrap();
        assert_eq!(unreadable, 1);
        std::env::remove_var(ENV_JOURNAL_DIR);
    }
}
