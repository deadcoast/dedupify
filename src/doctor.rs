//! `doctor` — PASS / WARN / FAIL per check with a one-line fix.

use std::fs;
use std::path::Path;
use std::process::Command as Proc;

use crate::cli::Invocation;
use crate::journal;
use crate::render::Renderer;
use crate::theme::Role;
use crate::util::{fit, fmt_bytes, fmt_count};
use crate::MILKED_DIR;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Pass,
    Warn,
    Fail,
}

#[derive(Debug, Clone)]
pub struct Check {
    pub name: &'static str,
    pub status: Status,
    pub detail: String,
    pub fix: String,
}

fn pass(name: &'static str, detail: impl Into<String>) -> Check {
    Check {
        name,
        status: Status::Pass,
        detail: detail.into(),
        fix: String::new(),
    }
}
fn warn(name: &'static str, detail: impl Into<String>, fix: impl Into<String>) -> Check {
    Check {
        name,
        status: Status::Warn,
        detail: detail.into(),
        fix: fix.into(),
    }
}
fn fail(name: &'static str, detail: impl Into<String>, fix: impl Into<String>) -> Check {
    Check {
        name,
        status: Status::Fail,
        detail: detail.into(),
        fix: fix.into(),
    }
}

pub fn checks(inv: &Invocation, r: &Renderer) -> Vec<Check> {
    let t = &r.theme;
    let mut v = Vec::new();

    // terminal
    v.push(if t.color {
        pass("colour", "24-bit ANSI colour")
    } else if std::env::var_os("NO_COLOR").is_some() {
        pass("colour", "off by NO_COLOR")
    } else if !t.live {
        pass("colour", "off: output is not a terminal")
    } else {
        warn(
            "colour",
            "terminal reports no ANSI support",
            "use Windows Terminal or PowerShell 7; conhost before Windows 10 1511 has no colour",
        )
    });
    v.push(if t.unicode {
        pass("glyphs", "box drawing and block characters")
    } else {
        pass("glyphs", "ASCII set")
    });
    v.push(if t.width >= 80 {
        pass("width", format!("{} cells", t.width))
    } else {
        warn(
            "width",
            format!("{} cells", t.width),
            "widen the window to 80+ so group lines keep their dates",
        )
    });

    // root
    let root = &inv.root;
    v.push(match fs::read_dir(root) {
        Ok(_) => pass("folder", root.to_string_lossy()),
        Err(e) => fail(
            "folder",
            format!("{}  {}", root.to_string_lossy(), e),
            "pick a folder you can read",
        ),
    });
    v.push(match writable(root) {
        Ok(()) => pass("write", "milk and purge can change this folder"),
        Err(e) => warn(
            "write",
            e,
            "find still works; milk and purge need write access here",
        ),
    });

    // _milked
    let milked = root.join(MILKED_DIR);
    if milked.is_dir() {
        let (n, b) = dir_size(&milked);
        v.push(pass(
            MILKED_DIR,
            format!("{} files   {}", fmt_count(n), fmt_bytes(b)),
        ));
    } else {
        v.push(pass(MILKED_DIR, "absent (created by the first milk)"));
    }

    // long paths (Windows MAX_PATH is 260)
    let longest = longest_path_len(root, 2_000);
    v.push(if cfg!(windows) && longest >= 260 {
        warn(
            "paths",
            format!("longest path seen: {longest} chars"),
            "enable LongPathsEnabled in HKLM\\SYSTEM\\CurrentControlSet\\Control\\FileSystem or shorten the tree",
        )
    } else {
        pass("paths", format!("longest path seen: {longest} chars"))
    });

    // journal
    let jdir = journal::dir_for(root);
    let journal_ok = fs::create_dir_all(&jdir)
        .map_err(|e| crate::actions::os_err(&e))
        .and_then(|_| writable(&jdir));
    v.push(match journal_ok {
        Ok(()) => pass("journal", jdir.to_string_lossy()),
        Err(e) => fail(
            "journal",
            format!("{}  {}", jdir.to_string_lossy(), e),
            "set DEDUPE_JOURNAL_DIR to a writable folder; undo needs it",
        ),
    });
    v.push(match journal::latest_pending(root) {
        Ok(Some((_, j))) => pass(
            "undo",
            format!(
                "pending: {}   {}   {} entries",
                j.id,
                j.command,
                j.entries.len()
            ),
        ),
        Ok(None) => pass("undo", "nothing pending"),
        Err(e) => warn("undo", e.to_string(), "journal folder unreadable"),
    });

    // recycle bin
    v.push(bin_check());

    // ffmpeg (video phase)
    v.push(match (which("ffmpeg"), which("ffprobe")) {
        (Some(a), Some(_)) => pass("ffmpeg", a),
        _ => warn(
            "ffmpeg",
            "ffmpeg / ffprobe not on PATH",
            "needed for the video phase only: winget install Gyan.FFmpeg",
        ),
    });

    v
}

#[cfg(any(windows, all(unix, not(target_os = "macos"))))]
fn bin_check() -> Check {
    match trash::os_limited::list() {
        Ok(items) => pass(
            "bin",
            format!(
                "Recycle Bin reachable   {} items",
                fmt_count(items.len() as u64)
            ),
        ),
        Err(e) => warn(
            "bin",
            e.to_string(),
            "purge needs the Recycle Bin; undo after purge reads it",
        ),
    }
}

#[cfg(not(any(windows, all(unix, not(target_os = "macos")))))]
fn bin_check() -> Check {
    warn(
        "bin",
        "listing the bin is unsupported here",
        "purge works; undo after purge must be done in Finder",
    )
}

fn writable(dir: &Path) -> Result<(), String> {
    let probe = dir.join(".dedupe-write-probe");
    match fs::write(&probe, b"") {
        Ok(()) => {
            let _ = fs::remove_file(&probe);
            Ok(())
        }
        Err(e) => Err(crate::actions::os_err(&e)),
    }
}

fn dir_size(dir: &Path) -> (u64, u64) {
    let mut n = 0;
    let mut b = 0;
    for e in walkdir::WalkDir::new(dir).into_iter().flatten() {
        if e.file_type().is_file() {
            n += 1;
            b += e.metadata().map(|m| m.len()).unwrap_or(0);
        }
    }
    (n, b)
}

/// Longest path length among the first `limit` entries (bounded so doctor
/// stays quick on huge trees).
fn longest_path_len(root: &Path, limit: usize) -> usize {
    walkdir::WalkDir::new(root)
        .into_iter()
        .flatten()
        .take(limit)
        .map(|e| e.path().as_os_str().len())
        .max()
        .unwrap_or(0)
}

fn which(bin: &str) -> Option<String> {
    let out = if cfg!(windows) {
        Proc::new("where").arg(bin).output()
    } else {
        Proc::new("which").arg(bin).output()
    };
    match out {
        Ok(o) if o.status.success() => {
            let s = String::from_utf8_lossy(&o.stdout);
            s.lines()
                .next()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
        }
        _ => None,
    }
}

pub fn page(inv: &Invocation, r: &Renderer) -> i32 {
    let t = &r.theme;
    r.header(inv);
    let list = checks(inv, r);
    let mut worst = Status::Pass;
    for c in &list {
        let (tag, role) = match c.status {
            Status::Pass => ("PASS", Role::Ok),
            Status::Warn => ("WARN", Role::Warn),
            Status::Fail => ("FAIL", Role::Danger),
        };
        if c.status == Status::Fail || (c.status == Status::Warn && worst == Status::Pass) {
            worst = c.status;
        }
        println!(
            " {}  {}  {}",
            t.paint_bold(role, tag),
            t.paint(Role::Plate, &fit(c.name, 8)),
            t.paint(Role::Dupe, &c.detail)
        );
        if !c.fix.is_empty() {
            println!("                 {}", t.paint(Role::Plate, &c.fix));
        }
    }
    println!("{}", t.rule());
    let (summary, role) = match worst {
        Status::Pass => ("everything in place", Role::Ok),
        Status::Warn => ("usable, with the notes above", Role::Warn),
        Status::Fail => (
            "fix the FAIL lines before running milk or purge",
            Role::Danger,
        ),
    };
    println!(" {}", t.paint_bold(role, summary));
    if worst == Status::Fail {
        2
    } else {
        0
    }
}
