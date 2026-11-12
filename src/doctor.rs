//! `doctor` — PASS / WARN / FAIL per check with a one-line fix.

use std::fs;
use std::path::Path;

use crate::cli::Invocation;
use crate::journal;
use crate::render::Renderer;
use crate::theme::Role;
use crate::util::{counted, fit, fmt_bytes, fmt_count, shorten_middle};
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
            "use Windows Terminal; the console host before Windows 10 1511 has no colour",
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

    // _milked, and whether it is on the root's volume (a link to another
    // drive makes every milk a verified copy)
    let milked = root.join(MILKED_DIR);
    let root_vol = crate::util::volume_id(root);
    if milked.is_dir() {
        let (n, b) = dir_size(&milked);
        let mut d = format!("{}   {}", counted(n, "file", "files"), fmt_bytes(b));
        let other =
            matches!((root_vol, crate::util::volume_id(&milked)), (Some(a), Some(b)) if a != b);
        if other {
            d.push_str("   on another volume: milk copies, reads back, then deletes");
        }
        v.push(pass(MILKED_DIR, d));
    } else {
        v.push(pass(MILKED_DIR, "absent (created by the first milk)"));
    }
    // folders of the root on other volumes (mount points; the scan never
    // follows links or junctions, so on Windows there are none to reach)
    if let Some(rv) = root_vol {
        let others = other_volumes(root, rv, 20_000);
        v.push(if others.is_empty() {
            pass("volumes", "one volume under this folder")
        } else {
            pass(
                "volumes",
                format!(
                    "{} on another volume (first: {}): milk copies them across, verified",
                    counted(others.len() as u64, "folder", "folders"),
                    crate::util::rel_display(&others[0], root)
                ),
            )
        });
    }

    // the remembered skip list (QOL-06)
    v.push(if inv.skip.is_empty() {
        pass("skip", "nothing skipped by name")
    } else {
        pass(
            "skip",
            format!(
                "{}   remembered; --unskip NAME or --unskip all forgets",
                inv.skip.join(", ")
            ),
        )
    });

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
    v.push(match journal::list_with_unreadable(root) {
        Ok((list, unreadable)) => {
            let pending = list.iter().find(|(_, j)| !j.consumed);
            let detail = match pending {
                Some((_, j)) => format!(
                    "pending: {}   {}   {}",
                    j.id,
                    j.command,
                    counted(j.entries.len() as u64, "entry", "entries")
                ),
                None => "nothing pending".to_string(),
            };
            if unreadable > 0 {
                warn(
                    "undo",
                    format!(
                        "{detail}   {} unreadable",
                        counted(unreadable, "journal", "journals")
                    ),
                    "a journal file is damaged; undo skips it — open the folder above to inspect",
                )
            } else {
                pass("undo", detail)
            }
        }
        Err(e) => warn("undo", e.to_string(), "journal folder unreadable"),
    });

    // recycle bin
    v.push(bin_check());

    // deep tier model (optional)
    let mp = crate::deep::model_path();
    v.push(if mp.is_file() {
        let size = fs::metadata(&mp).map(|m| m.len()).unwrap_or(0);
        // the published file, byte for byte (docs/DEEP.md)
        match sha256_file(&mp) {
            Some(h) if h == crate::deep::MODEL_SHA256 && size == crate::deep::MODEL_BYTES => pass(
                "deep",
                format!(
                    "{}   {}   SHA-256 matches",
                    mp.to_string_lossy(),
                    fmt_bytes(size)
                ),
            ),
            Some(h) => warn(
                "deep",
                format!(
                    "{}   {}   SHA-256 {}… is not the published file's",
                    mp.to_string_lossy(),
                    fmt_bytes(size),
                    &h[..12]
                ),
                format!(
                    "download it again (docs/DEEP.md); expected {}…",
                    &crate::deep::MODEL_SHA256[..12]
                ),
            ),
            None => warn(
                "deep",
                format!("{}   unreadable", mp.to_string_lossy()),
                "check the file's permissions",
            ),
        }
    } else {
        Check {
            name: "deep",
            status: Status::Pass,
            detail: "no model (optional)   docs/DEEP.md".to_string(),
            fix: format!("expected at {}", mp.to_string_lossy()),
        }
    });

    // ffmpeg (videos, HEIC, AVIF): where `tools` found it, and whether
    // that was PATH or an installer's own folder
    let (ff, fp) = (crate::tools::ffmpeg_at(), crate::tools::ffprobe_at());
    v.push(match (ff, fp) {
        (Some(a), Some(_)) => pass(
            "ffmpeg",
            if a.on_path {
                a.path.to_string_lossy().into_owned()
            } else {
                format!(
                    "{}   not on PATH, used from there",
                    a.path.to_string_lossy()
                )
            },
        ),
        (Some(_), None) => warn(
            "ffmpeg",
            "ffmpeg found, ffprobe not",
            "videos need both: reinstall ffmpeg (winget install Gyan.FFmpeg)",
        ),
        _ => warn(
            "ffmpeg",
            "ffmpeg / ffprobe not found",
            "needed for videos, HEIC and AVIF: winget install Gyan.FFmpeg",
        ),
    });
    // HEIC and AVIF: two built-in probes decoded the way a scan decodes
    // them, and their two halves checked (red left, blue right)
    v.push(heif_check(ff.is_some()));

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

const PROBE_HEIC: &[u8] = include_bytes!("../assets/doctor/probe.heic");
const PROBE_AVIF: &[u8] = include_bytes!("../assets/doctor/probe.avif");

/// Decode one probe; `Ok` when the left half reads red and the right blue.
fn probe(b: &[u8]) -> Result<(), String> {
    let d = crate::decode::load_heif_bytes(b).map_err(|e| e.to_string())?;
    let img = d.img.to_rgb8();
    if img.dimensions() != (64, 64) {
        return Err(format!("decoded {}×{}", img.width(), img.height()));
    }
    let l = img.get_pixel(16, 32).0;
    let r = img.get_pixel(48, 32).0;
    if l[0] > 150 && l[2] < 100 && r[2] > 150 && r[0] < 100 {
        Ok(())
    } else {
        Err("decoded to the wrong colours".into())
    }
}

fn heif_check(ffmpeg: bool) -> Check {
    if !ffmpeg {
        return warn(
            "heif",
            "HEIC and AVIF photos are not decoded without ffmpeg",
            "winget install Gyan.FFmpeg",
        );
    }
    match (probe(PROBE_HEIC), probe(PROBE_AVIF)) {
        (Ok(()), Ok(())) => pass("heif", "HEIC and AVIF decode (probe pictures checked)"),
        (Err(e), Ok(())) => warn(
            "heif",
            format!("HEIC does not decode: {e}"),
            "an ffmpeg build with the HEVC decoder (any full build)",
        ),
        (Ok(()), Err(e)) => warn(
            "heif",
            format!("AVIF does not decode: {e}"),
            "an ffmpeg build with libdav1d or libaom (any full build)",
        ),
        (Err(e), Err(_)) => warn(
            "heif",
            format!("HEIC and AVIF do not decode: {e}"),
            "an ffmpeg full build (HEVC, libdav1d)",
        ),
    }
}

/// SHA-256 of a file, hex.
fn sha256_file(p: &Path) -> Option<String> {
    use sha2::{Digest, Sha256};
    let mut f = fs::File::open(p).ok()?;
    let mut h = Sha256::new();
    std::io::copy(&mut f, &mut h).ok()?;
    Some(
        h.finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>(),
    )
}

/// Folders under `root` (up to `limit` of them looked at) whose volume is
/// not `root_vol`. A folder on another volume is reported, not entered.
fn other_volumes(root: &Path, root_vol: u64, limit: usize) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let mut seen = 0usize;
    let mut it = walkdir::WalkDir::new(root).follow_links(false).into_iter();
    while let Some(e) = it.next() {
        let Ok(e) = e else { continue };
        if !e.file_type().is_dir() || e.depth() == 0 {
            continue;
        }
        seen += 1;
        if seen > limit {
            break;
        }
        if crate::util::volume_id(e.path()).is_some_and(|v| v != root_vol) {
            out.push(e.path().to_path_buf());
            it.skip_current_dir();
        }
    }
    out
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
        // margin, tag, name column, then the detail shortened in the middle
        // (a path keeps its head and its tail) rather than wrapped
        let room = t.width.saturating_sub(1 + 4 + 2 + 8 + 2 + 1).max(12);
        println!(
            " {}  {}  {}",
            t.paint_bold(role, tag),
            t.paint(Role::Plate, &fit(c.name, 8)),
            t.paint(
                Role::Dupe,
                &shorten_middle(&c.detail, room, t.glyphs.ellipsis)
            )
        );
        if !c.fix.is_empty() {
            // under a WARN or FAIL the line is what to do, and stands out;
            // under a PASS it is a note, as quiet as the detail above it
            // (the deep row's model path outshone its own PASS, seen on
            // the 2026-11-12 Windows picture) [CLAUDE]
            let tone = if c.status == Status::Pass {
                Role::Dupe
            } else {
                Role::Plate
            };
            let room = t.width.saturating_sub(17 + 1).max(12);
            println!(
                "                 {}",
                t.paint(tone, &shorten_middle(&c.fix, room, t.glyphs.ellipsis))
            );
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
