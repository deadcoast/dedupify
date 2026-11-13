//! Theme "MILK" (docs/CLI.md §3): palette, glyph set, capability detection.
//!
//! One struct decides everything the renderer draws with. Colour is
//! 24-bit via ANSI escapes; off when stdout is not a terminal, when
//! `NO_COLOR` is set, or when `TERM=dumb`. Glyphs fall back to ASCII when
//! stdout is not a terminal (someone is piping this), when `TERM=dumb`,
//! or when `DEDUPE_ASCII=1`; `NO_COLOR` alone keeps the glyph set, since
//! it asks for no colour, not for no Unicode.

use std::io::IsTerminal;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub const WHITE: Rgb = Rgb(0xFF, 0xFF, 0xFF);

    /// `self` moved toward `to` by `k` (0 keeps `self`, 1 is `to`).
    pub fn lerp(self, to: Rgb, k: f64) -> Rgb {
        let k = k.clamp(0.0, 1.0);
        let m = |a: u8, b: u8| (a as f64 + (b as f64 - a as f64) * k).round() as u8;
        Rgb(m(self.0, to.0), m(self.1, to.1), m(self.2, to.2))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Plate,
    Keep,
    Dupe,
    Rule,
    Ok,
    Warn,
    Danger,
    Info,
}

impl Role {
    pub fn rgb(self) -> Rgb {
        match self {
            Role::Plate => Rgb(0xE8, 0xE4, 0xD8),
            Role::Keep => Rgb(0xFF, 0xB0, 0x00),
            Role::Dupe => Rgb(0x8A, 0x8A, 0x8A),
            Role::Rule => Rgb(0x4A, 0x4A, 0x4A),
            Role::Ok => Rgb(0x7C, 0xD9, 0x92),
            Role::Warn => Rgb(0xFF, 0xB0, 0x00),
            Role::Danger => Rgb(0xFF, 0x4D, 0x4D),
            Role::Info => Rgb(0x5F, 0xB3, 0xFF),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Glyphs {
    pub rule: &'static str,    // horizontal rule
    pub bar_on: &'static str,  // progress filled
    pub bar_off: &'static str, // progress empty
    pub ellipsis: &'static str,
    pub pointer: &'static str, // menu cursor
    pub arrow: &'static str,   // "into" on the workspace bar
    pub bullet: &'static str,
    pub tl: &'static str,
    pub tr: &'static str,
    pub bl: &'static str,
    pub br: &'static str,
    pub h: &'static str,
    pub v: &'static str,
    pub up: &'static str,
    pub down: &'static str,
    pub le: &'static str,    // ≤
    pub ge: &'static str,    // ≥
    pub times: &'static str, // ×
    // workspace selection marks, one cell each
    pub sel_on: &'static str,   // acted on
    pub sel_off: &'static str,  // could be, is not
    pub sel_part: &'static str, // a group with some members chosen
    pub sel_lock: &'static str, // never (another capture moment)
}

pub const UNICODE: Glyphs = Glyphs {
    rule: "─",
    bar_on: "━",
    bar_off: "╌",
    ellipsis: "…",
    pointer: "▸",
    arrow: "→",
    bullet: "▪",
    tl: "┏",
    tr: "┓",
    bl: "┗",
    br: "┛",
    h: "━",
    v: "┃",
    up: "↑",
    down: "↓",
    le: "≤",
    ge: "≥",
    times: "×",
    sel_on: "■",
    sel_off: "□",
    sel_part: "◧",
    sel_lock: "·",
};

pub const ASCII: Glyphs = Glyphs {
    rule: "-",
    bar_on: "=",
    bar_off: "-",
    ellipsis: "...",
    pointer: ">",
    arrow: "->",
    bullet: "*",
    tl: "+",
    tr: "+",
    bl: "+",
    br: "+",
    h: "=",
    v: "|",
    up: "^",
    down: "v",
    le: "<=",
    ge: ">=",
    times: "x",
    sel_on: "x",
    sel_off: ".",
    sel_part: "~",
    sel_lock: "-",
};

#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub color: bool,
    pub unicode: bool,
    /// Terminal width in cells.
    pub width: usize,
    /// True when stdout is a terminal (progress can redraw in place).
    pub live: bool,
    pub glyphs: Glyphs,
    /// The fold (docs/CLI.md §3a): a run as one line per section and the
    /// yield. Off, the full listing (`--verbose`). Set by `main` from the
    /// command line; every theme built elsewhere is the full listing.
    pub fold: bool,
}

impl Theme {
    /// Detect from the environment. On Windows this also switches the
    /// console into virtual-terminal mode through crossterm.
    pub fn detect() -> Theme {
        let tty = std::io::stdout().is_terminal();
        let no_color = std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty());
        let dumb = std::env::var("TERM").map(|t| t == "dumb").unwrap_or(false);
        let ansi = tty && !no_color && !dumb && ansi_supported();
        let ascii_env = std::env::var("DEDUPE_ASCII")
            .map(|v| v == "1")
            .unwrap_or(false);
        let unicode = tty && !dumb && !ascii_env;
        let width = if tty {
            match crossterm::terminal::size() {
                Ok((w, _)) if w >= 40 => w as usize,
                _ => columns_env().unwrap_or(100),
            }
        } else {
            columns_env().unwrap_or(100)
        };
        Theme::custom(ansi, unicode, width.clamp(40, 200), tty)
    }

    pub fn plain() -> Theme {
        Theme::custom(false, false, 100, false)
    }

    pub fn custom(color: bool, unicode: bool, width: usize, live: bool) -> Theme {
        Theme {
            color,
            unicode,
            width,
            live,
            glyphs: if unicode { UNICODE } else { ASCII },
            fold: false,
        }
    }

    pub fn paint(&self, role: Role, s: &str) -> String {
        if !self.color {
            return s.to_string();
        }
        let Rgb(r, g, b) = role.rgb();
        format!("\x1b[38;2;{r};{g};{b}m{s}\x1b[0m")
    }

    pub fn bold(&self, s: &str) -> String {
        if !self.color {
            return s.to_string();
        }
        format!("\x1b[1m{s}\x1b[22m")
    }

    /// `s` in any 24-bit colour (the fold's hues and gradients).
    pub fn paint_rgb(&self, c: Rgb, s: &str) -> String {
        if !self.color {
            return s.to_string();
        }
        let Rgb(r, g, b) = c;
        format!("\x1b[38;2;{r};{g};{b}m{s}\x1b[0m")
    }

    pub fn paint_rgb_bold(&self, c: Rgb, s: &str) -> String {
        if !self.color {
            return s.to_string();
        }
        let Rgb(r, g, b) = c;
        format!("\x1b[1;38;2;{r};{g};{b}m{s}\x1b[0m")
    }

    pub fn paint_bold(&self, role: Role, s: &str) -> String {
        if !self.color {
            return s.to_string();
        }
        let Rgb(r, g, b) = role.rgb();
        format!("\x1b[1;38;2;{r};{g};{b}m{s}\x1b[0m")
    }

    /// A full-width horizontal rule, indented one cell like every line.
    pub fn rule(&self) -> String {
        let n = self.width.saturating_sub(2);
        self.paint(Role::Rule, &format!(" {}", self.glyphs.rule.repeat(n)))
    }
}

fn columns_env() -> Option<usize> {
    std::env::var("COLUMNS")
        .ok()?
        .parse::<usize>()
        .ok()
        .filter(|w| *w >= 40)
}

/// Windows: ask the console for virtual-terminal processing (crossterm
/// enables it as a side effect of the check). Elsewhere every terminal
/// that reaches this code speaks ANSI.
#[cfg(windows)]
fn ansi_supported() -> bool {
    crossterm::ansi_support::supports_ansi()
}

#[cfg(not(windows))]
fn ansi_supported() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_theme_paints_nothing() {
        let t = Theme::plain();
        assert_eq!(t.paint(Role::Keep, "x"), "x");
        assert_eq!(t.bold("x"), "x");
        assert_eq!(t.glyphs.rule, "-");
    }

    #[test]
    fn colour_theme_wraps_in_escapes() {
        let t = Theme::custom(true, true, 80, true);
        assert_eq!(t.paint(Role::Keep, "x"), "\x1b[38;2;255;176;0mx\x1b[0m");
        assert_eq!(t.glyphs.rule, "─");
    }
}
