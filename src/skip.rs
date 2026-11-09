//! Names never scanned (QOL-06): `--skip NAME` leaves out every folder and
//! file called NAME — a catalog's preview cache, `node_modules`, a sync
//! tool's trash — and remembers it, so every later run, in any folder,
//! leaves it out too. `--unskip NAME` (or `--unskip all`) forgets. The
//! list is one small file in the app folder, beside `recent.json`; nothing
//! is written into the scanned tree. Every run that skips says so on its
//! header line. [CLAUDE, Phase 10]
//!
//! A name matches a file or folder name whole, ignoring case on Windows
//! (where the file system does); `*` stands for any run of characters and
//! `?` for one (`*.tmp`, `Lightroom*`).

use std::path::Path;

use crate::journal::base_dir;

const FILE: &str = "skip.json";

/// The remembered names, in the order they were added.
pub fn load() -> Vec<String> {
    load_in(&base_dir())
}

pub fn load_in(dir: &Path) -> Vec<String> {
    std::fs::read(dir.join(FILE))
        .ok()
        .and_then(|b| serde_json::from_slice::<Vec<String>>(&b).ok())
        .unwrap_or_default()
}

/// Write the list (atomically); an empty list removes the file.
pub fn save_in(dir: &Path, names: &[String]) -> std::io::Result<()> {
    let path = dir.join(FILE);
    if names.is_empty() {
        return match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    }
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!("{FILE}.tmp"));
    std::fs::write(&tmp, serde_json::to_vec_pretty(names).unwrap_or_default())?;
    std::fs::rename(&tmp, &path)
}

/// Apply `--skip` and `--unskip` to the remembered list and return the
/// list in force. A failure to write is not fatal: the run still skips
/// what it was told to.
pub fn resolve(add: &[String], remove: &[String]) -> Vec<String> {
    resolve_in(&base_dir(), add, remove)
}

pub fn resolve_in(dir: &Path, add: &[String], remove: &[String]) -> Vec<String> {
    let mut names = load_in(dir);
    let before = names.clone();
    for r in remove {
        if r.eq_ignore_ascii_case("all") {
            names.clear();
        } else {
            names.retain(|n| !same(n, r));
        }
    }
    for a in add {
        if !a.is_empty() && !names.iter().any(|n| same(n, a)) {
            names.push(a.clone());
        }
    }
    if names != before {
        let _ = save_in(dir, &names);
    }
    names
}

fn same(a: &str, b: &str) -> bool {
    if cfg!(windows) {
        a.eq_ignore_ascii_case(b)
    } else {
        a == b
    }
}

/// Does `name` match the pattern `pat` (`*`, `?`)?
pub fn matches(pat: &str, name: &str) -> bool {
    let fold = |s: &str| -> Vec<char> {
        if cfg!(windows) {
            s.chars().flat_map(char::to_lowercase).collect()
        } else {
            s.chars().collect()
        }
    };
    let (p, n) = (fold(pat), fold(name));
    // iterative wildcard match with one backtrack point
    let (mut pi, mut ni) = (0usize, 0usize);
    let (mut star, mut mark) = (None::<usize>, 0usize);
    while ni < n.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == n[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ni;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ni = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// Is `name` one of `names`?
pub fn skipped(names: &[String], name: &std::ffi::OsStr) -> bool {
    if names.is_empty() {
        return false;
    }
    let n = name.to_string_lossy();
    names.iter().any(|p| matches(p, &n))
}

/// `100k`, `2.5m`, `1g`, `500` (bytes); units are 1024-based, as sizes are
/// printed. `None` when the text is not a size.
pub fn parse_size(s: &str) -> Option<u64> {
    let t = s.trim().to_ascii_lowercase();
    let t = t.strip_suffix('b').unwrap_or(&t).to_string();
    let (num, mult) = match t.chars().last()? {
        'k' => (&t[..t.len() - 1], 1024f64),
        'm' => (&t[..t.len() - 1], 1024f64 * 1024.0),
        'g' => (&t[..t.len() - 1], 1024f64 * 1024.0 * 1024.0),
        c if c.is_ascii_digit() => (&t[..], 1f64),
        _ => return None,
    };
    let v: f64 = num.trim().parse().ok()?;
    if !v.is_finite() || v < 0.0 {
        return None;
    }
    Some((v * mult).round() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcards_match_whole_names() {
        assert!(matches("node_modules", "node_modules"));
        assert!(!matches("node_modules", "node_modules2"));
        assert!(matches("*.tmp", "a.tmp"));
        assert!(!matches("*.tmp", "a.tmp.jpg"));
        assert!(matches("Lightroom*", "Lightroom Catalog Previews.lrdata"));
        assert!(matches("IMG_????.JPG", "IMG_0041.JPG"));
        assert!(!matches("IMG_????.JPG", "IMG_041.JPG"));
        assert!(matches("*", "anything"));
        assert!(matches("a*b*c", "aXXbYYc"));
        assert!(!matches("a*b*c", "aXXbYY"));
        assert_eq!(matches("Photos", "photos"), cfg!(windows));
    }

    #[test]
    fn sizes_parse_in_printed_units() {
        assert_eq!(parse_size("100k"), Some(102_400));
        assert_eq!(parse_size("100KB"), Some(102_400));
        assert_eq!(parse_size("2.5m"), Some(2_621_440));
        assert_eq!(parse_size("1g"), Some(1_073_741_824));
        assert_eq!(parse_size("500"), Some(500));
        assert_eq!(parse_size("500b"), Some(500));
        assert_eq!(parse_size("x"), None);
        assert_eq!(parse_size("-1k"), None);
        assert_eq!(parse_size(""), None);
    }

    #[test]
    fn the_list_is_remembered_and_forgotten() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path();
        assert!(load_in(p).is_empty());
        let v = resolve_in(p, &["node_modules".into(), "*.tmp".into()], &[]);
        assert_eq!(v, vec!["node_modules", "*.tmp"]);
        // remembered, no duplicates
        let v = resolve_in(p, &["node_modules".into()], &[]);
        assert_eq!(v.len(), 2);
        assert_eq!(load_in(p), v);
        let v = resolve_in(p, &[], &["*.tmp".into()]);
        assert_eq!(v, vec!["node_modules"]);
        let v = resolve_in(p, &[], &["all".into()]);
        assert!(v.is_empty());
        assert!(!p.join(FILE).exists(), "an empty list leaves no file");
    }
}
