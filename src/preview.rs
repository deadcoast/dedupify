//! Looking at a file before deciding on it (NF-03, NF-04).
//!
//! The workspace's `v` draws the keeper and the member side by side in the
//! terminal when it speaks an inline-graphics protocol — sixel (Windows
//! Terminal 1.22+, foot, mlterm, WezTerm), the kitty graphics protocol
//! (kitty, Ghostty) or iTerm2's inline images (iTerm2, WezTerm) — and opens
//! both with the system viewer when it speaks none. `o` reveals a file in
//! the file manager (Explorer with the file selected on Windows). [CLAUDE]
//!
//! The protocol is chosen from the environment, never by querying the
//! terminal: a query's reply arrives on the same input the key loop reads,
//! and on Windows crossterm reads console input records, not the VT reply.
//! On Windows the terminal is also known by its window: Windows hands a
//! console started outside it to its default terminal without setting
//! `WT_SESSION` (the Windows run of Phase 16 drew no picture for that
//! reason), and the console's hidden window is then owned by Windows
//! Terminal's own, class `CASCADIA_HOSTING_WINDOW_CLASS`.
//! `DEDUPE_GRAPHICS=sixel|kitty|iterm|none` overrides the choice.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use image::{imageops::FilterType, DynamicImage, RgbImage};

pub const ENV_GRAPHICS: &str = "DEDUPE_GRAPHICS";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    Sixel,
    Kitty,
    Iterm,
    None,
}

impl Protocol {
    pub fn name(self) -> &'static str {
        match self {
            Protocol::Sixel => "sixel",
            Protocol::Kitty => "kitty",
            Protocol::Iterm => "iTerm2",
            Protocol::None => "none",
        }
    }
}

/// The protocol for this terminal, from the environment, then (Windows)
/// from the window that shows the console.
pub fn detect() -> Protocol {
    let var = |k: &str| std::env::var(k).unwrap_or_default();
    match detect_from(&|k: &str| var(k)) {
        Protocol::None if var(ENV_GRAPHICS).is_empty() && windows_terminal() => Protocol::Sixel,
        p => p,
    }
}

/// True when this console is shown by Windows Terminal: its window, or
/// the hidden stand-in Windows Terminal owns, belongs to the class
/// `CASCADIA_HOSTING_WINDOW_CLASS`.
#[cfg(windows)]
pub fn windows_terminal() -> bool {
    use windows::Win32::System::Console::GetConsoleWindow;
    use windows::Win32::UI::WindowsAndMessaging::{GetAncestor, GetClassNameW, GA_ROOTOWNER};
    let class = |h: windows::Win32::Foundation::HWND| -> String {
        let mut buf = [0u16; 128];
        // SAFETY: the buffer outlives the call and its length is passed
        let n = unsafe { GetClassNameW(h, &mut buf) };
        String::from_utf16_lossy(&buf[..n.max(0) as usize])
    };
    // SAFETY: no arguments; a null window when there is no console
    let con = unsafe { GetConsoleWindow() };
    if con.is_invalid() {
        return false;
    }
    // SAFETY: `con` is a window handle the system just returned
    let owner = unsafe { GetAncestor(con, GA_ROOTOWNER) };
    [con, owner]
        .into_iter()
        .any(|h| !h.is_invalid() && class(h) == "CASCADIA_HOSTING_WINDOW_CLASS")
}

#[cfg(not(windows))]
pub fn windows_terminal() -> bool {
    false
}

/// `detect` on an injected environment (tests).
pub fn detect_from(var: &dyn Fn(&str) -> String) -> Protocol {
    match var(ENV_GRAPHICS).to_ascii_lowercase().as_str() {
        "sixel" => return Protocol::Sixel,
        "kitty" => return Protocol::Kitty,
        "iterm" | "iterm2" => return Protocol::Iterm,
        "none" | "off" => return Protocol::None,
        _ => {}
    }
    let term = var("TERM").to_ascii_lowercase();
    let program = var("TERM_PROGRAM").to_ascii_lowercase();
    if !var("KITTY_WINDOW_ID").is_empty() || term.contains("kitty") || program == "ghostty" {
        return Protocol::Kitty;
    }
    if program == "iterm.app" || program == "wezterm" {
        return Protocol::Iterm;
    }
    if !var("WT_SESSION").is_empty() || term.starts_with("foot") || term.contains("mlterm") {
        return Protocol::Sixel;
    }
    Protocol::None
}

/// What to show for one file: a still decoded from the file, or a frame a
/// third of the way into a video.
#[derive(Debug, Clone)]
pub enum Visual {
    Image(PathBuf),
    Video(PathBuf, f64),
}

impl Visual {
    pub fn path(&self) -> &Path {
        match self {
            Visual::Image(p) | Visual::Video(p, _) => p,
        }
    }

    /// Decode the still (`decode`): an image upright, a video's frame a
    /// third of the way in.
    pub fn load(&self) -> Option<DynamicImage> {
        match self {
            Visual::Image(p) => crate::decode::load_image(p).ok(),
            Visual::Video(p, duration) => crate::decode::video_frame(p, *duration),
        }
    }
}

/// Scale `img` to fit `max_w` × `max_h` pixels, aspect kept, never enlarged.
pub fn fit_pixels(img: &DynamicImage, max_w: u32, max_h: u32) -> RgbImage {
    let (w, h) = (img.width().max(1), img.height().max(1));
    let s = (max_w as f64 / w as f64)
        .min(max_h as f64 / h as f64)
        .min(1.0);
    let nw = ((w as f64 * s).round() as u32).max(1);
    let nh = ((h as f64 * s).round() as u32).max(1);
    img.resize_exact(nw, nh, FilterType::Triangle).to_rgb8()
}

/// The pixel size of one terminal cell, in the pixels a sixel image is
/// measured in: from the terminal when it reports it (Unix); on Windows
/// 10 × 20, the cell Windows Terminal and the console host lay every sixel
/// image out on whatever the font (they emulate the VT340's, and scale the
/// picture to the real cell — microsoft/terminal#17421; QOL-13); else
/// 9 × 18, a usual cell at the default font sizes.
pub fn cell_pixels() -> (u32, u32) {
    if let Ok(ws) = crossterm::terminal::window_size() {
        if ws.width > 0 && ws.height > 0 && ws.columns > 0 && ws.rows > 0 {
            return (
                (ws.width / ws.columns).max(1) as u32,
                (ws.height / ws.rows).max(1) as u32,
            );
        }
    }
    if cfg!(windows) {
        (10, 20)
    } else {
        (9, 18)
    }
}

// ---------------------------------------------------------------------------
// encoders
// ---------------------------------------------------------------------------

/// Sixel (DEC, VT340): six-pixel-high bands, one pass per colour per band,
/// run-length coded. Colours are the 6 × 6 × 6 cube with ordered (Bayer
/// 4 × 4) dithering — 216 registers, inside every sixel terminal's
/// palette, and smooth enough for a side-by-side check.
pub fn sixel(img: &RgbImage) -> String {
    const BAYER: [[f32; 4]; 4] = [
        [0.0, 8.0, 2.0, 10.0],
        [12.0, 4.0, 14.0, 6.0],
        [3.0, 11.0, 1.0, 9.0],
        [15.0, 7.0, 13.0, 5.0],
    ];
    let (w, h) = (img.width() as usize, img.height() as usize);
    // palette index per pixel
    let mut idx = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            let p = img.get_pixel(x as u32, y as u32).0;
            let d = (BAYER[y % 4][x % 4] + 0.5) / 16.0 - 0.5; // [-0.5, 0.5)
            let q = |c: u8| -> usize {
                let v = c as f32 / 51.0 + d;
                v.round().clamp(0.0, 5.0) as usize
            };
            idx[y * w + x] = (q(p[0]) * 36 + q(p[1]) * 6 + q(p[2])) as u8;
        }
    }
    let mut used = [false; 216];
    for &i in &idx {
        used[i as usize] = true;
    }
    let mut out = String::with_capacity(w * h / 2);
    out.push_str("\x1bPq");
    out.push_str(&format!("\"1;1;{w};{h}"));
    for (i, u) in used.iter().enumerate() {
        if *u {
            let (r, g, b) = (i / 36, (i / 6) % 6, i % 6);
            out.push_str(&format!("#{i};2;{};{};{}", r * 20, g * 20, b * 20));
        }
    }
    let mut band_y = 0;
    while band_y < h {
        let rows = (h - band_y).min(6);
        // colours present in this band
        let mut present = [false; 216];
        for dy in 0..rows {
            for x in 0..w {
                present[idx[(band_y + dy) * w + x] as usize] = true;
            }
        }
        let mut first = true;
        for (c, here) in present.iter().enumerate() {
            if !*here {
                continue;
            }
            if !first {
                out.push('$');
            }
            first = false;
            out.push_str(&format!("#{c}"));
            let mut run_ch = '\0';
            let mut run_n = 0usize;
            let flush = |out: &mut String, ch: char, n: usize| {
                if n == 0 {
                    return;
                }
                if n > 3 {
                    out.push_str(&format!("!{n}{ch}"));
                } else {
                    for _ in 0..n {
                        out.push(ch);
                    }
                }
            };
            for x in 0..w {
                let mut bits = 0u8;
                for dy in 0..rows {
                    if idx[(band_y + dy) * w + x] as usize == c {
                        bits |= 1 << dy;
                    }
                }
                let ch = (63 + bits) as char;
                if ch == run_ch {
                    run_n += 1;
                } else {
                    flush(&mut out, run_ch, run_n);
                    run_ch = ch;
                    run_n = 1;
                }
            }
            flush(&mut out, run_ch, run_n);
        }
        out.push('-');
        band_y += 6;
    }
    out.push_str("\x1b\\");
    out
}

/// Standard base64 (RFC 4648), for the kitty and iTerm2 payloads.
pub fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

fn png_bytes(img: &RgbImage) -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    let _ = DynamicImage::ImageRgb8(img.clone()).write_to(&mut buf, image::ImageFormat::Png);
    buf.into_inner()
}

/// Kitty graphics protocol: PNG payload, transmitted and displayed in one
/// command (`a=T`), in 4096-byte chunks, the cursor left where it was.
pub fn kitty(img: &RgbImage) -> String {
    let data = base64(&png_bytes(img));
    let chunks: Vec<&str> = data
        .as_bytes()
        .chunks(4096)
        .map(|c| std::str::from_utf8(c).unwrap_or(""))
        .collect();
    let mut out = String::new();
    for (i, c) in chunks.iter().enumerate() {
        let more = u8::from(i + 1 < chunks.len());
        if i == 0 {
            out.push_str(&format!("\x1b_Gf=100,a=T,C=1,q=2,m={more};{c}\x1b\\"));
        } else {
            out.push_str(&format!("\x1b_Gm={more};{c}\x1b\\"));
        }
    }
    out
}

/// iTerm2 inline image (OSC 1337), PNG payload at its pixel size.
pub fn iterm(img: &RgbImage) -> String {
    let png = png_bytes(img);
    format!(
        "\x1b]1337;File=inline=1;size={};width={}px;height={}px;preserveAspectRatio=1:{}\x07",
        png.len(),
        img.width(),
        img.height(),
        base64(&png)
    )
}

/// The escape string drawing `img` at the cursor.
pub fn encode(p: Protocol, img: &RgbImage) -> String {
    match p {
        Protocol::Sixel => sixel(img),
        Protocol::Kitty => kitty(img),
        Protocol::Iterm => iterm(img),
        Protocol::None => String::new(),
    }
}

// ---------------------------------------------------------------------------
// the system viewer and the file manager
// ---------------------------------------------------------------------------

/// Open `path` with its default application. Detached; the result says
/// only whether the opener could be started.
pub fn open(path: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // explorer opens a file with its associated program; the quotes go
        // in raw so a path with spaces stays one argument
        Command::new("explorer.exe")
            .raw_arg(format!("\"{}\"", path.display()))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map(|_| ())
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map(|_| ())
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        Command::new("xdg-open")
            .arg(path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map(|_| ())
    }
}

/// Show `path` in the file manager: selected in Explorer (`/select,`) and
/// Finder (`-R`); its folder opened elsewhere.
pub fn reveal(path: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // `/select,"C:\a b\c.jpg"` — explorer parses its own command line,
        // so the switch and the quoted path go in raw
        Command::new("explorer.exe")
            .raw_arg(format!("/select,\"{}\"", path.display()))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map(|_| ())
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg("-R")
            .arg(path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map(|_| ())
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let dir = path.parent().unwrap_or(path);
        Command::new("xdg-open")
            .arg(dir)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map(|_| ())
    }
}

/// The name of the file manager, for the status line.
pub fn file_manager() -> &'static str {
    if cfg!(windows) {
        "Explorer"
    } else if cfg!(target_os = "macos") {
        "Finder"
    } else {
        "the file manager"
    }
}

/// Write `s` to the terminal unbuffered-in-effect (one write, one flush).
pub fn emit(s: &str) -> io::Result<()> {
    let mut out = io::stdout().lock();
    out.write_all(s.as_bytes())?;
    out.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_from_the_environment() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |k: &str| {
                pairs
                    .iter()
                    .find(|(n, _)| *n == k)
                    .map(|(_, v)| v.to_string())
                    .unwrap_or_default()
            }
        };
        assert_eq!(detect_from(&env(&[])), Protocol::None);
        assert_eq!(detect_from(&env(&[("WT_SESSION", "x")])), Protocol::Sixel);
        assert_eq!(
            detect_from(&env(&[("KITTY_WINDOW_ID", "1")])),
            Protocol::Kitty
        );
        assert_eq!(
            detect_from(&env(&[("TERM_PROGRAM", "WezTerm")])),
            Protocol::Iterm
        );
        assert_eq!(detect_from(&env(&[("TERM", "foot")])), Protocol::Sixel);
        // the override wins
        assert_eq!(
            detect_from(&env(&[("WT_SESSION", "x"), ("DEDUPE_GRAPHICS", "none")])),
            Protocol::None
        );
    }

    #[test]
    fn base64_matches_rfc4648_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn sixel_frames_bands_and_runs() {
        // 8×7, left half black, right half white: two bands (6 + 1 rows)
        let mut img = RgbImage::new(8, 7);
        for y in 0..7 {
            for x in 4..8 {
                img.put_pixel(x, y, image::Rgb([255, 255, 255]));
            }
        }
        let s = sixel(&img);
        assert!(s.starts_with("\x1bPq\"1;1;8;7"), "{s:?}");
        assert!(s.ends_with("-\x1b\\"), "{s:?}");
        assert_eq!(s.matches('-').count(), 2, "one '-' per band: {s:?}");
        // black (#0) and white (#215) registers defined, nothing else
        assert!(
            s.contains("#0;2;0;0;0") && s.contains("#215;2;100;100;100"),
            "{s:?}"
        );
        assert_eq!(s.matches(";2;").count(), 2, "{s:?}");
        // first band: black on the left four columns (all six bits: '~'),
        // empty on the right ('?'), run-length coded
        assert!(s.contains("#0!4~!4?"), "{s:?}");
        assert!(s.contains("#215!4?!4~"), "{s:?}");
        // second band: one row, bit 0 only ('@')
        assert!(s.contains("#0!4@!4?"), "{s:?}");
    }

    #[test]
    fn fit_pixels_keeps_aspect_and_never_enlarges() {
        let img = DynamicImage::ImageRgb8(RgbImage::new(800, 400));
        let t = fit_pixels(&img, 200, 200);
        assert_eq!((t.width(), t.height()), (200, 100));
        let small = DynamicImage::ImageRgb8(RgbImage::new(50, 40));
        let t = fit_pixels(&small, 200, 200);
        assert_eq!((t.width(), t.height()), (50, 40));
    }
}
