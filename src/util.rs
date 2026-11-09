//! Formatting helpers shared by the renderer, journal and doctor.

use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Binary units with the labels Windows Explorer shows (KB = 1024 bytes),
/// so a reclaimable total here matches a folder's Properties dialog.
pub fn fmt_bytes(n: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];
    if n < 1024 {
        return format!("{n} B");
    }
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if v >= 100.0 {
        format!("{v:.0} {}", UNITS[i])
    } else if v >= 10.0 {
        format!("{v:.1} {}", UNITS[i])
    } else {
        format!("{v:.2} {}", UNITS[i])
    }
}

/// Thousands separators: 12480 → "12,480".
pub fn fmt_count(n: u64) -> String {
    let s = n.to_string();
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && (bytes.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(*b as char);
    }
    out
}

pub fn fmt_duration(d: Duration) -> String {
    let s = d.as_secs_f64();
    if s < 10.0 {
        format!("{s:.1} s")
    } else if s < 600.0 {
        format!("{s:.0} s")
    } else {
        let m = (s / 60.0).floor();
        let r = s - m * 60.0;
        format!("{m:.0} m {r:.0} s")
    }
}

/// `m:ss` under an hour, `h:mm:ss` above.
pub fn fmt_clock(secs: f64) -> String {
    let s = secs.round().max(0.0) as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s % 3600) / 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// Display width in terminal cells.
pub fn width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

/// Display width of text that may carry ANSI SGR sequences (`ESC [ … m`),
/// which take no cells.
pub fn visible_width(s: &str) -> usize {
    let mut w = 0;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for d in chars.by_ref() {
                    if d.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        w += UnicodeWidthChar::width(c).unwrap_or(0);
    }
    w
}

/// "1 file" / "3 files".
pub fn plural<'a>(n: u64, one: &'a str, many: &'a str) -> &'a str {
    if n == 1 {
        one
    } else {
        many
    }
}

/// `n` with its noun: "1 file", "12,480 files".
pub fn counted(n: u64, one: &str, many: &str) -> String {
    format!("{} {}", fmt_count(n), plural(n, one, many))
}

/// Join `segments` with three spaces, dropping trailing segments while
/// the visible width exceeds `max`. The first segment always stays.
pub fn join_fit(segments: &[String], max: usize) -> String {
    let mut n = segments.len();
    while n > 1 {
        let line = segments[..n].join("   ");
        if visible_width(&line) <= max {
            return line;
        }
        n -= 1;
    }
    segments.first().cloned().unwrap_or_default()
}

/// Fit `s` into `max` cells by cutting the middle and inserting `ellipsis`.
/// Keeps the tail (file name) longer than the head.
pub fn shorten_middle(s: &str, max: usize, ellipsis: &str) -> String {
    let w = width(s);
    if w <= max {
        return s.to_string();
    }
    let ew = width(ellipsis);
    if max <= ew + 2 {
        // nothing sensible fits; take the tail
        return take_tail(s, max);
    }
    let avail = max - ew;
    let tail_cells = (avail * 3) / 5;
    let head_cells = avail - tail_cells;
    let head = take_head(s, head_cells);
    let tail = take_tail(s, tail_cells);
    format!("{head}{ellipsis}{tail}")
}

fn take_head(s: &str, cells: usize) -> String {
    let mut out = String::new();
    let mut used = 0;
    for c in s.chars() {
        let cw = UnicodeWidthChar::width(c).unwrap_or(0);
        if used + cw > cells {
            break;
        }
        out.push(c);
        used += cw;
    }
    out
}

fn take_tail(s: &str, cells: usize) -> String {
    let mut buf: Vec<char> = Vec::new();
    let mut used = 0;
    for c in s.chars().rev() {
        let cw = UnicodeWidthChar::width(c).unwrap_or(0);
        if used + cw > cells {
            break;
        }
        buf.push(c);
        used += cw;
    }
    buf.iter().rev().collect()
}

/// Pad or cut `s` to exactly `cells` display cells (left-aligned).
pub fn fit(s: &str, cells: usize) -> String {
    let w = width(s);
    if w == cells {
        s.to_string()
    } else if w < cells {
        let mut out = s.to_string();
        out.push_str(&" ".repeat(cells - w));
        out
    } else {
        take_head(s, cells)
    }
}

/// Right-align `s` in `cells`.
pub fn fit_right(s: &str, cells: usize) -> String {
    let w = width(s);
    if w >= cells {
        take_tail(s, cells)
    } else {
        format!("{}{}", " ".repeat(cells - w), s)
    }
}

/// `path` relative to `root`, as a string in the platform's separators.
/// Falls back to the full path when `path` is not under `root`.
pub fn rel_display(path: &Path, root: &Path) -> String {
    match path.strip_prefix(root) {
        Ok(r) => r.to_string_lossy().into_owned(),
        Err(_) => path.to_string_lossy().into_owned(),
    }
}

/// Strip the `\\?\` verbatim prefix `std::fs::canonicalize` adds on
/// Windows (`\\?\C:\x` → `C:\x`, `\\?\UNC\srv\share` → `\\srv\share`).
/// std re-adds it internally for long paths, so nothing is lost; the user
/// sees the path as Explorer shows it.
pub fn strip_verbatim(p: &Path) -> std::path::PathBuf {
    let s = p.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        return std::path::PathBuf::from(format!(r"\\{rest}"));
    }
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        return std::path::PathBuf::from(rest);
    }
    p.to_path_buf()
}

/// `<root as the user typed it, or ".">` + separator + `name`: the short
/// form of a path under the root, for lines that would otherwise carry the
/// full absolute root again.
pub fn under_root(root_display: Option<&str>, name: &str) -> String {
    let base = root_display.unwrap_or(".");
    let sep = std::path::MAIN_SEPARATOR;
    if base.ends_with(sep) || base.ends_with('/') {
        format!("{base}{name}")
    } else {
        format!("{base}{sep}{name}")
    }
}

/// Civil date from a Unix timestamp (days since 1970-01-01), proleptic
/// Gregorian. Algorithm: Howard Hinnant, "chrono-Compatible Low-Level Date
/// Algorithms" (civil_from_days).
pub fn civil_from_unix(secs: i64) -> (i64, u32, u32) {
    let days = secs.div_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Days since 1970-01-01 for a civil date (Hinnant's `days_from_civil`,
/// the inverse of `civil_from_unix`).
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400; // [0, 399]
    let m = m as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d as i64 - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146_097 + doe - 719_468
}

/// `YYYY-MM-DD HH:MM` in the local calendar for a journal stamp
/// (`YYYYMMDD-HHMMSS`, UTC). The stamp itself when it does not parse.
pub fn fmt_stamp_local(stamp: &str) -> String {
    let digits: String = stamp.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() < 14 {
        return stamp.to_string();
    }
    let n = |a: usize, b: usize| digits[a..b].parse::<i64>().unwrap_or(0);
    let (y, mo, d, h, mi, se) = (n(0, 4), n(4, 6), n(6, 8), n(8, 10), n(10, 12), n(12, 14));
    let secs = days_from_civil(y, mo as u32, d as u32) * 86_400
        + h * 3600
        + mi * 60
        + se
        + local_offset_secs();
    let (ly, lm, ld) = civil_from_unix(secs);
    let tod = secs.rem_euclid(86_400);
    format!(
        "{ly:04}-{lm:02}-{ld:02} {:02}:{:02}",
        tod / 3600,
        (tod % 3600) / 60
    )
}

/// The machine's UTC offset in seconds, read once at start-up (before any
/// worker thread exists — `time` refuses the lookup in a multi-threaded
/// process on Unix). Zero when it cannot be determined.
static LOCAL_OFFSET: std::sync::OnceLock<i64> = std::sync::OnceLock::new();

/// Call first thing in `main`.
pub fn init_local_offset() {
    LOCAL_OFFSET.get_or_init(|| {
        time::UtcOffset::current_local_offset()
            .map(|o| o.whole_seconds() as i64)
            .unwrap_or(0)
    });
}

pub fn local_offset_secs() -> i64 {
    *LOCAL_OFFSET.get().unwrap_or(&0)
}

/// `YYYY-MM-DD` for a file time, in the local calendar (what Explorer
/// shows beside the file). The journal id stays UTC (`stamp`).
pub fn fmt_date(t: SystemTime) -> String {
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => {
            let (y, m, dd) = civil_from_unix(d.as_secs() as i64 + local_offset_secs());
            format!("{y:04}-{m:02}-{dd:02}")
        }
        Err(_) => "----------".to_string(),
    }
}

/// `YYYYMMDD-HHMMSS` (UTC) — the journal id stem.
pub fn stamp(t: SystemTime) -> String {
    let secs = t
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0) as i64;
    let (y, m, d) = civil_from_unix(secs);
    let sod = secs.rem_euclid(86_400);
    format!(
        "{y:04}{m:02}{d:02}-{:02}{:02}{:02}",
        sod / 3600,
        (sod % 3600) / 60,
        sod % 60
    )
}

pub fn unix_secs(t: SystemTime) -> i64 {
    t.duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// The volume `path` lives on: the device number on Unix, the volume
/// serial number on Windows. `None` when it cannot be read. Two paths with
/// different ids are on different volumes: a rename between them fails.
pub fn volume_id(path: &std::path::Path) -> Option<u64> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata(path).ok().map(|m| m.dev())
    }
    #[cfg(windows)]
    {
        let h = winapi_util::Handle::from_path_any(path).ok()?;
        winapi_util::file::information(&h)
            .ok()
            .map(|i| i.volume_serial_number())
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        None
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn days_from_civil_inverts_civil_from_unix() {
        for days in [-1_000_000i64, -1, 0, 1, 10_957, 20_401, 2_932_896] {
            let (y, m, d) = civil_from_unix(days * 86_400);
            assert_eq!(days_from_civil(y, m, d), days, "{y}-{m}-{d}");
        }
        // a journal stamp, at UTC (no offset initialised in tests)
        assert_eq!(fmt_stamp_local("20261109-101131"), "2026-11-09 10:11");
        assert_eq!(fmt_stamp_local("garbage"), "garbage");
    }

    use super::*;

    #[test]
    fn bytes() {
        assert_eq!(fmt_bytes(0), "0 B");
        assert_eq!(fmt_bytes(1023), "1023 B");
        assert_eq!(fmt_bytes(1024), "1.00 KB");
        assert_eq!(fmt_bytes(25_270_681), "24.1 MB");
        assert_eq!(fmt_bytes(41_017_651_200), "38.2 GB");
        assert_eq!(fmt_bytes(150 * 1024 * 1024), "150 MB");
    }

    #[test]
    fn counts() {
        assert_eq!(fmt_count(0), "0");
        assert_eq!(fmt_count(999), "999");
        assert_eq!(fmt_count(1000), "1,000");
        assert_eq!(fmt_count(12480), "12,480");
        assert_eq!(fmt_count(1_234_567_890), "1,234,567,890");
    }

    #[test]
    fn shorten() {
        let s = "2019/trip/somewhere/very/deep/IMG_0041.jpg";
        let out = shorten_middle(s, 24, "…");
        assert!(width(&out) <= 24, "{out}");
        assert!(out.ends_with("IMG_0041.jpg"), "{out}");
        assert!(out.starts_with("2019"), "{out}");
        assert_eq!(shorten_middle("short", 24, "…"), "short");
        let ascii = shorten_middle(s, 24, "...");
        assert!(width(&ascii) <= 24, "{ascii}");
    }

    #[test]
    fn visible_width_ignores_sgr() {
        assert_eq!(visible_width("\x1b[38;2;1;2;3mab\x1b[0m"), 2);
        assert_eq!(visible_width("plain"), 5);
        assert_eq!(visible_width("\x1b[1m×\x1b[22m"), 1);
    }

    #[test]
    fn join_fit_drops_from_the_right() {
        let segs = vec!["aaaa".to_string(), "bbbb".to_string(), "cccc".to_string()];
        assert_eq!(join_fit(&segs, 100), "aaaa   bbbb   cccc");
        assert_eq!(join_fit(&segs, 12), "aaaa   bbbb");
        assert_eq!(join_fit(&segs, 2), "aaaa");
        assert_eq!(counted(1, "file", "files"), "1 file");
        assert_eq!(counted(1200, "file", "files"), "1,200 files");
    }

    #[test]
    fn verbatim_prefix_is_stripped() {
        assert_eq!(
            strip_verbatim(Path::new(r"\\?\C:\Users\x")),
            std::path::PathBuf::from(r"C:\Users\x")
        );
        assert_eq!(
            strip_verbatim(Path::new(r"\\?\UNC\srv\share\d")),
            std::path::PathBuf::from(r"\\srv\share\d")
        );
        assert_eq!(
            strip_verbatim(Path::new("/plain/path")),
            std::path::PathBuf::from("/plain/path")
        );
        let sep = std::path::MAIN_SEPARATOR;
        assert_eq!(under_root(None, "_milked"), format!(".{sep}_milked"));
        assert_eq!(
            under_root(Some("pics"), "_milked"),
            format!("pics{sep}_milked")
        );
    }

    #[test]
    fn fitting() {
        assert_eq!(fit("ab", 4), "ab  ");
        assert_eq!(fit("abcdef", 4), "abcd");
        assert_eq!(fit_right("ab", 4), "  ab");
    }

    #[test]
    fn civil_dates() {
        assert_eq!(civil_from_unix(0), (1970, 1, 1));
        assert_eq!(civil_from_unix(951_782_400), (2000, 2, 29));
        assert_eq!(civil_from_unix(1_560_297_600), (2019, 6, 12));
        assert_eq!(civil_from_unix(-86_400), (1969, 12, 31));
        assert_eq!(
            stamp(UNIX_EPOCH + Duration::from_secs(1_560_297_600 + 3_723)),
            "20190612-010203"
        );
    }
}
