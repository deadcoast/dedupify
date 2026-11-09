//! The last folder looked at (QOL-03). `find`, `milk` and `purge` record
//! their root, scope and kind in the app directory, beside the journals —
//! never in the scanned folder — and the bare `dedupe` menu offers it as
//! its first line, `again`, which runs `find` there: most use is sorting
//! and re-sorting the same few folders. [CLAUDE]
//!
//! One small JSON file, overwritten each run; a missing, unreadable or
//! stale record (the folder is gone) is simply not offered.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::cli::Invocation;
use crate::journal::base_dir;

const FILE: &str = "recent.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recent {
    /// Absolute root.
    pub root: PathBuf,
    pub recurse: bool,
    /// `""`, `"img"` or `"vid"`.
    pub kind: String,
    /// `--min` as typed, when the run had a size floor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<String>,
}

impl Recent {
    pub fn of(inv: &Invocation) -> Recent {
        Recent {
            root: inv.root.clone(),
            recurse: inv.recurse,
            kind: inv.kind.token().to_string(),
            min: inv.min_display.clone(),
        }
    }

    /// The argv `again` runs: `find <root> [.] [kind]`.
    pub fn args(&self) -> Vec<String> {
        let mut v = vec!["find".to_string(), self.root.to_string_lossy().into_owned()];
        if self.recurse {
            v.push(".".into());
        }
        if !self.kind.is_empty() {
            v.push(self.kind.clone());
        }
        if let Some(m) = &self.min {
            v.push("--min".into());
            v.push(m.clone());
        }
        v
    }

    /// `C:\Photos  .  img` — the line the menu shows.
    pub fn label(&self) -> String {
        let mut s = self.root.to_string_lossy().into_owned();
        if self.recurse {
            s.push_str("  .");
        }
        if !self.kind.is_empty() {
            s.push_str("  ");
            s.push_str(&self.kind);
        }
        if let Some(m) = &self.min {
            s.push_str("  min ");
            s.push_str(m);
        }
        s
    }
}

fn path_in(dir: &Path) -> PathBuf {
    dir.join(FILE)
}

/// Record `inv`; failures are silent (a convenience, never a reason to
/// stop a run).
pub fn record(inv: &Invocation) {
    record_in(&base_dir(), &Recent::of(inv));
}

pub fn record_in(dir: &Path, r: &Recent) {
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    if let Ok(json) = serde_json::to_string_pretty(r) {
        let tmp = dir.join(format!("{FILE}.tmp"));
        if std::fs::write(&tmp, json).is_ok() {
            let _ = std::fs::rename(&tmp, path_in(dir));
        }
    }
}

/// The last record, when its folder still exists.
pub fn load() -> Option<Recent> {
    load_in(&base_dir())
}

pub fn load_in(dir: &Path) -> Option<Recent> {
    let text = std::fs::read_to_string(path_in(dir)).ok()?;
    let r: Recent = serde_json::from_str(&text).ok()?;
    r.root.is_dir().then_some(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_stale_folders_are_not_offered() {
        let app = tempfile::tempdir().unwrap();
        let photos = tempfile::tempdir().unwrap();
        let r = Recent {
            root: photos.path().to_path_buf(),
            recurse: true,
            kind: "img".into(),
            min: None,
        };
        assert_eq!(load_in(app.path()), None);
        // a size floor travels with the record and its command
        let with_min = Recent {
            min: Some("100k".into()),
            ..r.clone()
        };
        assert_eq!(
            with_min.args().last().map(String::as_str),
            Some("100k"),
            "{:?}",
            with_min.args()
        );
        assert!(with_min.label().ends_with("min 100k"));
        record_in(app.path(), &r);
        assert_eq!(load_in(app.path()), Some(r.clone()));
        assert_eq!(
            r.args(),
            vec![
                "find".to_string(),
                photos.path().to_string_lossy().into_owned(),
                ".".into(),
                "img".into()
            ]
        );
        drop(photos);
        assert_eq!(load_in(app.path()), None, "folder gone: not offered");
    }
}
