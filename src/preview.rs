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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
#[derive(Debug, Clone, PartialEq)]
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

/// Scale `img` to fill `max_w` × `max_h` pixels at its proportions,
/// enlarged too: the workspace's sharp pictures fill their boxes as the
/// block pictures do. A fast box reduction to twice the size first, then
/// Catmull-Rom to it, so edges stay crisp. [CLAUDE, Phase 19]
pub fn fit_pixels_sharp(img: &DynamicImage, max_w: u32, max_h: u32) -> RgbImage {
    let (w, h) = (img.width().max(1), img.height().max(1));
    let s = (max_w as f64 / w as f64).min(max_h as f64 / h as f64);
    let nw = ((w as f64 * s).round() as u32).max(1);
    let nh = ((h as f64 * s).round() as u32).max(1);
    let staged = if w > 2 * nw && h > 2 * nh {
        img.thumbnail(2 * nw, 2 * nh)
    } else {
        img.clone()
    };
    staged
        .resize_exact(nw, nh, FilterType::CatmullRom)
        .to_rgb8()
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
/// run-length coded. The palette is the picture's own, 255 colours, the
/// pixels mapped to it with Floyd–Steinberg error diffusion (`quant`), so
/// gradients and skin read as the photograph and not as the 6 × 6 × 6
/// cube's posterised steps (Phase 19; the cube with ordered dithering
/// before it).
pub fn sixel(img: &RgbImage) -> String {
    let (w, h) = (img.width() as usize, img.height() as usize);
    if w == 0 || h == 0 {
        return String::new();
    }
    let pal = crate::quant::palette(img, 255);
    let mut idx = crate::quant::dither(img, &pal);
    let map: Vec<u8> = pal
        .iter()
        .flat_map(|c| c.map(|v| v.round().clamp(0.0, 255.0) as u8))
        .collect();
    let colours = pal.len();
    // only the colours the pixels use are defined, numbered as they first
    // appear (a flat picture leaves most of the 255 unused or doubled)
    let mut number = vec![u8::MAX; colours];
    let mut order: Vec<usize> = Vec::new();
    for k in idx.iter_mut() {
        let c = *k as usize;
        if number[c] == u8::MAX {
            number[c] = order.len() as u8;
            order.push(c);
        }
        *k = number[c];
    }
    let colours = order.len();
    let mut out = String::with_capacity(w * h / 2);
    out.push_str("\x1bPq");
    out.push_str(&format!("\"1;1;{w};{h}"));
    for (i, &c) in order.iter().enumerate() {
        // sixel colours are percentages
        let pc = |v: u8| (v as u32 * 100 + 127) / 255;
        out.push_str(&format!(
            "#{i};2;{};{};{}",
            pc(map[3 * c]),
            pc(map[3 * c + 1]),
            pc(map[3 * c + 2])
        ));
    }
    let mut band_y = 0;
    let mut present = vec![false; colours.max(1)];
    while band_y < h {
        let rows = (h - band_y).min(6);
        present.iter_mut().for_each(|p| *p = false);
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
        // black (#0, first seen) and white (#1) defined, nothing else
        assert!(
            s.contains("#0;2;0;0;0") && s.contains("#1;2;100;100;100"),
            "{s:?}"
        );
        assert_eq!(s.matches(";2;").count(), 2, "{s:?}");
        // first band: black on the left four columns (all six bits: '~'),
        // empty on the right ('?'), run-length coded
        assert!(s.contains("#0!4~!4?"), "{s:?}");
        assert!(s.contains("#1!4?!4~"), "{s:?}");
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

    fn unbase64(s: &str) -> Vec<u8> {
        const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = Vec::new();
        let (mut acc, mut bits) = (0u32, 0);
        for c in s.bytes().filter(|&c| c != b'=') {
            acc = (acc << 6) | T.iter().position(|&t| t == c).unwrap() as u32;
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                out.push((acc >> bits) as u8);
                acc &= (1 << bits) - 1;
            }
        }
        out
    }

    fn noise(w: u32, h: u32) -> RgbImage {
        let mut s = 0x9E3779B9u32;
        RgbImage::from_fn(w, h, |_, _| {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            image::Rgb([s as u8, (s >> 8) as u8, (s >> 16) as u8])
        })
    }

    #[test]
    fn kitty_sends_the_png_in_chunks_and_leaves_the_cursor() {
        // noise does not compress: several 4096-byte chunks
        let img = noise(60, 40);
        let s = kitty(&img);
        let cmds: Vec<&str> = s.split("\x1b\\").filter(|c| !c.is_empty()).collect();
        assert!(cmds.len() > 1, "{} commands", cmds.len());
        let mut payload = String::new();
        for (i, c) in cmds.iter().enumerate() {
            let (keys, data) = c.strip_prefix("\x1b_G").unwrap().split_once(';').unwrap();
            let last = i + 1 == cmds.len();
            if i == 0 {
                // transmit and show, PNG, cursor stays, no replies
                for k in ["f=100", "a=T", "C=1", "q=2"] {
                    assert!(keys.split(',').any(|x| x == k), "{keys}");
                }
            }
            assert!(keys.ends_with(if last { "m=0" } else { "m=1" }), "{keys}");
            assert!(data.len() <= 4096);
            payload.push_str(data);
        }
        let png = image::load_from_memory(&unbase64(&payload))
            .unwrap()
            .to_rgb8();
        assert_eq!(png, img);
    }

    #[test]
    fn iterm_sends_the_png_at_its_pixel_size() {
        let img = noise(30, 20);
        let s = iterm(&img);
        let body = s
            .strip_prefix("\x1b]1337;File=")
            .and_then(|b| b.strip_suffix('\x07'))
            .unwrap();
        let (args, data) = body.split_once(':').unwrap();
        let png = unbase64(data);
        assert_eq!(
            args,
            format!(
                "inline=1;size={};width=30px;height=20px;preserveAspectRatio=1",
                png.len()
            )
        );
        assert_eq!(image::load_from_memory(&png).unwrap().to_rgb8(), img);
    }

    #[test]
    fn sharp_pictures_fill_their_box_at_their_proportions() {
        let fit = |w, h, bw, bh| {
            let img = DynamicImage::ImageRgb8(RgbImage::new(w, h));
            fit_pixels_sharp(&img, bw, bh).dimensions()
        };
        // a large picture down, a small one up: the comparison's cells
        // are filled either way
        assert_eq!(fit(800, 400, 300, 300), (300, 150));
        assert_eq!(fit(100, 50, 300, 300), (300, 150));
        assert_eq!(fit(400, 800, 342, 266), (133, 266));
    }
}
