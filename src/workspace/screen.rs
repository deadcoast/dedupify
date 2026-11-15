//! The workspace's screen, composed from its state and the listing lines
//! the shell captured, so a test can check what a person sees without a
//! pseudo-terminal (split from `state.rs`, Phase 16, SC-12).
//!
//! The fold, at rest (docs/CLI.md §2a, Phase 18) [CLAUDE]: the groups are
//! drawn as the run's output is, one line each, by kind. Each kind is a
//! section; the sections the cursor is not in fold to their one line (its
//! groups, what they free, what is chosen, a skyline of its groups); in
//! the cursor's section each group is one line (its keeper, how many
//! beside it, what it frees and a bar of it against the largest, what its
//! files are); the cursor's group unfolds to its files, the full listing's
//! lines on a tree. The cursor's line is lit in its kind's colour. Under
//! the groups the gauge: what is chosen, of all there is to free, in each
//! kind's colour — the line the full output drew as a rule.

use std::collections::HashMap;
use std::time::Duration;

use super::state::*;
use super::thumbs::{Thumb, Thumbs};
use crate::render::fold::{self, BLOCKS, DIM, GROOVE, PLATE};
use crate::theme::{Rgb, Role as Paint, Theme};
use crate::util::{fmt_bytes, fmt_count, shorten_middle, strip_sgr, visible_width, width};

/// What the shell animates between keys.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    /// How far the lines that just unfolded have come in, 0 to 1.
    pub fade: f64,
    /// The lines that just unfolded: a group's files, or a section's groups.
    pub fresh: Fresh,
    /// The bytes the gauge shows while it rolls to the selection's.
    pub gauge: Option<u64>,
    /// The pointer's breath, 0 to 1.
    pub breath: f64,
    /// Seconds since the workspace opened (a coming picture's sheen).
    pub clock: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fresh {
    None,
    Group(usize),
    Section(Hue),
}

impl Frame {
    /// Everything drawn in full.
    pub const STILL: Frame = Frame {
        fade: 1.0,
        fresh: Fresh::None,
        gauge: None,
        breath: 0.0,
        clock: 0.0,
    };
}

/// One line of the body as drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Line {
    /// A kind's section (the order as listed).
    Section(Hue),
    /// The one section of an order other than as listed.
    Order,
    Group(usize),
    Member(usize, usize),
}

/// The pointer's colour, and the cursor line's ground in a kind's colour.
const AMBER: Rgb = Rgb(0xFF, 0xB0, 0x00);
const GROUND: Rgb = Rgb(0x14, 0x14, 0x14);

/// Panels from this width: the groups on the left, the comparison on the
/// right; narrower, the groups alone.
pub const PANELS_MIN: usize = 100;

/// A picture's fade from the panel's ground, once it arrives.
pub const FADE: Duration = Duration::from_millis(220);

/// The chip a key sits on: a shade above the ground.
const CHIP: Rgb = Rgb(0x2B, 0x28, 0x22);

/// A key as a keycap: the key bold in amber on its chip, a cell of chip
/// either side; `[k]` without colour. Its width is the key's and two.
pub fn cap(t: &Theme, key: &str) -> String {
    if t.color {
        let Rgb(r, g, b) = AMBER;
        let Rgb(cr, cg, cb) = CHIP;
        format!("\x1b[1;38;2;{r};{g};{b};48;2;{cr};{cg};{cb}m {key} \x1b[0m")
    } else {
        format!("[{key}]")
    }
}

/// Keys on a line: each a keycap and its words dim, two cells between;
/// the words shorten to their first, then the last keys go. A bar's
/// most used keys come first. Its width is `width_cells` at most.
pub fn fit_parts(t: &Theme, parts: &[(String, String)], width_cells: usize) -> String {
    let render = |take: usize, count: usize| -> (usize, String) {
        let mut w = 0;
        let mut painted = String::new();
        for (key, text) in parts.iter().take(count) {
            let text: String = if take == 0 {
                text.clone()
            } else {
                text.split_whitespace()
                    .take(take)
                    .collect::<Vec<_>>()
                    .join(" ")
            };
            let sep = if w == 0 { " " } else { "  " };
            w += width(sep) + width(key) + 2 + 1 + width(&text);
            painted.push_str(&format!(
                "{sep}{} {}",
                cap(t, key),
                t.paint_rgb(DIM.lerp(PLATE, 0.35), &text)
            ));
        }
        (w, painted)
    };
    for take in [0usize, 1] {
        let c = render(take, parts.len());
        if c.0 <= width_cells {
            return c.1;
        }
    }
    for count in (1..parts.len()).rev() {
        let c = render(1, count);
        if c.0 <= width_cells {
            return c.1;
        }
    }
    render(1, 1).1
}

/// Keys on a frame's edge, as many as fit `room` in order: the key lit,
/// its words in the plate's dim, the edge's rule between them. With the
/// cells it takes.
fn edge_keys(t: &Theme, edge: Rgb, items: &[(String, String)], room: usize) -> (String, usize) {
    let hz = if t.unicode { "─" } else { "-" };
    let mut out = String::new();
    let mut w = 0;
    for (key, words) in items {
        let add = width(key) + 1 + width(words) + if w == 0 { 0 } else { 3 };
        if w + add > room {
            break;
        }
        if w > 0 {
            out.push_str(&t.paint_rgb(edge, &format!(" {hz} ")));
        }
        out.push_str(&t.paint_rgb_bold(AMBER, key));
        out.push(' ');
        out.push_str(&t.paint_rgb(DIM.lerp(PLATE, 0.35), words));
        w += add;
    }
    (out, w)
}

/// `text` on the cursor's ground — its kind's colour, faint — across the
/// line; every reset inside it sets the ground again.
fn lit(t: &Theme, text: &str, hue: Rgb, max: usize) -> String {
    if !t.color {
        return text.to_string();
    }
    let pad = max.saturating_sub(visible_width(text));
    let Rgb(r, g, b) = GROUND.lerp(hue, 0.16);
    let set = format!("\x1b[48;2;{r};{g};{b}m");
    format!(
        "{set}{}{}\x1b[0m",
        text.replace("\x1b[0m", &format!("\x1b[0m{set}")),
        " ".repeat(pad)
    )
}

/// A line faded toward the ground, as under a card: every colour it sets
/// three quarters of the way to the ground, the text it leaves in the
/// terminal's colour a dim grey.
fn fade(t: &Theme, line: &str) -> String {
    if !t.color {
        return line.to_string();
    }
    let base = GROOVE.lerp(GROUND, 0.2);
    let Rgb(r, g, b) = base;
    let dim_fg = format!("\x1b[38;2;{r};{g};{b}m");
    let mut out = dim_fg.clone();
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' || chars.peek() != Some(&'[') {
            out.push(c);
            continue;
        }
        chars.next();
        let mut params = String::new();
        let mut fin = 'm';
        for d in chars.by_ref() {
            if d.is_ascii_alphabetic() {
                fin = d;
                break;
            }
            params.push(d);
        }
        if fin != 'm' {
            out.push_str(&format!("\x1b[{params}{fin}"));
            continue;
        }
        let p: Vec<&str> = params.split(';').collect();
        let mut kept: Vec<String> = Vec::new();
        let mut i = 0;
        let mut reset = false;
        while i < p.len() {
            match p[i] {
                "38" | "48" if p.get(i + 1) == Some(&"2") && i + 4 < p.len() => {
                    let n = |k: usize| p[i + k].parse::<u8>().unwrap_or(0);
                    let Rgb(r, g, b) = Rgb(n(2), n(3), n(4)).lerp(GROUND, 0.75);
                    kept.push(format!("{};2;{r};{g};{b}", p[i]));
                    i += 5;
                }
                "0" | "" => {
                    reset = true;
                    i += 1;
                }
                "1" => i += 1,
                other => {
                    kept.push(other.into());
                    i += 1;
                }
            }
        }
        // a reset comes back to the dim grey, then what the escape sets
        if reset {
            out.push_str("\x1b[0m");
            out.push_str(&dim_fg);
        }
        if !kept.is_empty() {
            out.push_str(&format!("\x1b[{}m", kept.join(";")));
        }
    }
    out.push_str("\x1b[0m");
    out
}

/// A panel's frame around `inner` lines `w` cells wide (the frame adds
/// two): its title in the top edge on the left, `tail` (`tail_w` cells)
/// in it on the right, `foot` (`foot_w` cells) in the bottom edge on the
/// right; every line padded to the frame, so nothing — a cursor's ground
/// included — runs past it. A label the edge has no room for is left off.
#[allow(clippy::too_many_arguments)]
fn framed(
    t: &Theme,
    title: &str,
    (tail, tail_w): (&str, usize),
    (foot, foot_w): (&str, usize),
    edge: Rgb,
    inner: &[String],
    w: usize,
    h: usize,
) -> Vec<String> {
    let (tl, tr, bl, br, hz, vt) = if t.unicode {
        ("╭", "╮", "╰", "╯", "─", "│")
    } else {
        ("+", "+", "+", "+", "-", "|")
    };
    let rule = |n: usize| t.paint_rgb(edge, &hz.repeat(n));
    let mut out = Vec::with_capacity(h + 2);
    let title_w = visible_width(title);
    if title_w > 0 && w > title_w + 3 {
        let mut rest = w - (title_w + 3);
        let tail_fits = tail_w > 0 && rest >= tail_w + 4;
        if tail_fits {
            rest -= tail_w + 3;
        }
        let mut top = format!(
            "{}{title}{}{}",
            t.paint_rgb(edge, &format!("{tl}{hz} ")),
            t.paint_rgb(edge, " "),
            rule(rest)
        );
        if tail_fits {
            top.push_str(&format!(" {tail} {}", rule(1)));
        }
        top.push_str(&t.paint_rgb(edge, tr));
        out.push(top);
    } else {
        out.push(t.paint_rgb(edge, &format!("{tl}{}{tr}", hz.repeat(w))));
    }
    let side = t.paint_rgb(edge, vt);
    for i in 0..h {
        let l = inner.get(i).map(String::as_str).unwrap_or("");
        let vw = visible_width(l);
        let l = if vw > w {
            crate::util::cut_sgr(l, w)
        } else {
            l.to_string()
        };
        let pad = w.saturating_sub(visible_width(&l));
        out.push(format!("{side}{l}{}{side}", " ".repeat(pad)));
    }
    if foot_w > 0 && w >= foot_w + 4 {
        out.push(format!(
            "{}{} {foot} {}",
            t.paint_rgb(edge, bl),
            rule(w - foot_w - 3),
            t.paint_rgb(edge, &format!("{hz}{br}"))
        ));
    } else {
        out.push(t.paint_rgb(edge, &format!("{bl}{}{br}", hz.repeat(w))));
    }
    out
}

/// The quadrant blocks by which of a cell's four pixels they cover:
/// top left 8, top right 4, bottom left 2, bottom right 1.
const QUADS: [&str; 16] = [
    " ", "▗", "▖", "▄", "▝", "▐", "▞", "▟", "▘", "▚", "▌", "▙", "▀", "▜", "▛", "█",
];

/// A cell of four pixels as one quadrant block in two colours: of the
/// ways to split the four into two sets, the one whose sets sit closest
/// to their own means; the block covers one set in its colour, the ground
/// is the other's.
fn quad(p: [Rgb; 4]) -> (usize, Rgb, Rgb) {
    let mean = |m: usize, on: bool| -> Rgb {
        let (mut r, mut g, mut b, mut n) = (0u32, 0u32, 0u32, 0u32);
        for (i, c) in p.iter().enumerate() {
            if ((m >> (3 - i)) & 1 == 1) == on {
                r += c.0 as u32;
                g += c.1 as u32;
                b += c.2 as u32;
                n += 1;
            }
        }
        if n == 0 {
            return Rgb(0, 0, 0);
        }
        Rgb((r / n) as u8, (g / n) as u8, (b / n) as u8)
    };
    let err = |m: usize| -> u32 {
        let (f, g) = (mean(m, true), mean(m, false));
        p.iter()
            .enumerate()
            .map(|(i, c)| {
                let k = if (m >> (3 - i)) & 1 == 1 { f } else { g };
                let d = |a: u8, b: u8| (a as i32 - b as i32).pow(2) as u32;
                d(c.0, k.0) + d(c.1, k.1) + d(c.2, k.2)
            })
            .sum()
    };
    // every split up to swapping its two sides, and no split at all
    let mut best = (15usize, err(15));
    for m in 1..8 {
        let e = err(m);
        if e < best.1 {
            best = (m, e);
        }
    }
    (best.0, mean(best.0, true), mean(best.0, false))
}

/// A picture `cw` cells by `ch` rows in quadrant blocks — four pixels a
/// cell in two of their colours, 24-bit — centred; while it is coming, a
/// sheen crosses its place; as it arrives it rises out of the ground.
fn picture(t: &Theme, thumb: &Thumb, cw: usize, ch: usize, aspect: f64, clock: f64) -> Vec<String> {
    let mut out = Vec::with_capacity(ch);
    match thumb {
        Thumb::Ready(img, at, _) => {
            let a = (at.elapsed().as_secs_f64() / FADE.as_secs_f64()).min(1.0);
            let a = 1.0 - (1.0 - a).powi(2);
            let (uw, uh) = ((img.width() / 2) as usize, (img.height() / 2) as usize);
            let x0 = cw.saturating_sub(uw) / 2;
            let y0 = ch.saturating_sub(uh) / 2;
            let px = |x: u32, y: u32| -> Rgb {
                let p = img.get_pixel(x.min(img.width() - 1), y.min(img.height() - 1));
                GROUND.lerp(Rgb(p[0], p[1], p[2]), a)
            };
            for r in 0..ch {
                if r < y0 || r >= y0 + uh {
                    out.push(String::new());
                    continue;
                }
                let mut line = " ".repeat(x0);
                let y = 2 * (r - y0) as u32;
                for c in 0..uw.min(cw) {
                    let x = 2 * c as u32;
                    let (m, f, g) = quad([px(x, y), px(x + 1, y), px(x, y + 1), px(x + 1, y + 1)]);
                    let (Rgb(r1, g1, b1), Rgb(r2, g2, b2)) = (f, g);
                    line.push_str(&format!(
                        "\x1b[38;2;{r1};{g1};{b1};48;2;{r2};{g2};{b2}m{}\x1b[0m",
                        QUADS[m]
                    ));
                }
                out.push(line);
            }
        }
        Thumb::Coming(_) => {
            // the picture's place at its proportions, a sheen running
            // across it corner to corner
            let w = ((2.0 * ch as f64 * aspect).round() as usize).clamp(4, cw);
            let h = ((w as f64 / (2.0 * aspect)).round() as usize).clamp(1, ch);
            let (x0, y0) = ((cw - w) / 2, (ch - h) / 2);
            let pos = (clock * 18.0) % (w as f64 + h as f64 + 12.0) - 6.0;
            for r in 0..ch {
                if r < y0 || r >= y0 + h {
                    out.push(String::new());
                    continue;
                }
                let mut line = " ".repeat(x0);
                for c in 0..w {
                    let d = (c as f64 + r as f64 * 1.6 - pos).abs();
                    let k = (1.0 - d / 5.0).max(0.0);
                    let col = Rgb(0x1C, 0x1C, 0x1C).lerp(Rgb(0x34, 0x34, 0x34), k);
                    line.push_str(&t.paint_rgb(col, "█"));
                }
                out.push(line);
            }
        }
        Thumb::Unreadable => {
            for r in 0..ch {
                out.push(if r == ch / 2 {
                    let s = "no picture";
                    format!(
                        "{}{}",
                        " ".repeat(cw.saturating_sub(s.len()) / 2),
                        t.paint_rgb(DIM, s)
                    )
                } else {
                    String::new()
                });
            }
        }
    }
    out
}

/// A meter: how close the file is on one piece of evidence, as a gauge
/// that is full when the two are the same — a distance empties toward
/// half again its limit, a score fills to its value — with a mark where
/// the limit falls; green within it, red past it; then the value against
/// the limit and a tick or a cross.
fn meter(t: &Theme, m: &crate::actions::Meter, bar: usize) -> String {
    let green = Rgb(0x7C, 0xD9, 0x92);
    let red = Rgb(0xFF, 0x4D, 0x4D);
    let (close, at) = if m.distance {
        let scale = m.limit * 1.5;
        (
            1.0 - (m.value / scale).clamp(0.0, 1.0),
            1.0 - m.limit / scale,
        )
    } else {
        (m.value.clamp(0.0, 1.0), m.limit)
    };
    let fill = (close * bar as f64).round() as usize;
    let mark = ((at * bar as f64).round() as usize).min(bar.saturating_sub(1));
    let colour = if !m.counts {
        DIM
    } else if m.within() {
        green
    } else {
        red
    };
    let (on, mk) = if t.unicode {
        ("━", "┃")
    } else {
        ("=", "|")
    };
    let mut cells = String::new();
    for i in 0..bar {
        let c = if i == mark {
            PLATE
        } else if i < fill {
            colour
        } else {
            Rgb(0x2A, 0x2A, 0x2A)
        };
        cells.push_str(&t.paint_rgb(c, if i == mark { mk } else { on }));
    }
    let value = if m.distance {
        format!("{}", m.value as u64)
    } else {
        format!("{:.2}", m.value)
    };
    let limit = if m.distance {
        format!("≤ {}", m.limit as u64)
    } else {
        format!("≥ {:.2}", m.limit)
    };
    let verdict = if !m.counts {
        t.paint_rgb(DIM, "flat picture: not weighed")
    } else if m.within() {
        t.paint_rgb_bold(green, if t.unicode { "✔" } else { "ok" })
    } else {
        t.paint_rgb_bold(red, if t.unicode { "✖" } else { "x" })
    };
    format!(
        "{}  {cells}  {}  {}  {verdict}",
        t.paint_rgb(DIM, &format!("{:<6}", m.name)),
        t.paint_rgb_bold(PLATE, &format!("{value:>4}")),
        t.paint_rgb(DIM, &format!("{limit:<6}"))
    )
}

impl State {
    /// The body's lines in the order drawn, and the cursor's among them.
    pub fn lines(&self) -> (Vec<Line>, Option<usize>) {
        let here = self.cursor_group();
        let at = self.rows.get(self.cursor).copied();
        let mut out = Vec::new();
        let mut cur = None;
        let mut group = |out: &mut Vec<Line>, g: usize| {
            if at == Some(Row::Head(g)) {
                cur = Some(out.len());
            }
            out.push(Line::Group(g));
            if here == Some(g) {
                for m in 0..self.layout.groups[g].members.len() {
                    if at == Some(Row::Member(g, m)) {
                        cur = Some(out.len());
                    }
                    out.push(Line::Member(g, m));
                }
            }
        };
        // every section open when every group fits; else the sections the
        // cursor is not in fold to their line
        let here_files = here.map_or(0, |g| self.layout.groups[g].members.len());
        let kinds = 1 + self
            .view
            .windows(2)
            .filter(|w| self.layout.groups[w[0]].kind != self.layout.groups[w[1]].kind)
            .count();
        let all_open = kinds + self.view.len() + here_files <= self.height;
        if self.order == Order::Listing {
            let mut i = 0;
            while i < self.view.len() {
                let kind = self.layout.groups[self.view[i]].kind;
                let j = i + self.view[i..]
                    .iter()
                    .take_while(|&&g| self.layout.groups[g].kind == kind)
                    .count();
                out.push(Line::Section(kind));
                if all_open || here.is_some_and(|h| self.view[i..j].contains(&h)) {
                    for &g in &self.view[i..j] {
                        group(&mut out, g);
                    }
                }
                i = j;
            }
        } else if !self.view.is_empty() {
            out.push(Line::Order);
            for &g in &self.view {
                group(&mut out, g);
            }
        }
        (out, cur)
    }

    /// The first body line to draw for `body` lines: the cursor in view,
    /// its group's line and section above it when they fit, its group's
    /// files below it when they fit; otherwise where it was.
    fn window(&self, lines: &[Line], cur: Option<usize>, body: usize) -> usize {
        let mut top = self.vtop.get();
        if let Some(c) = cur {
            let g_line = (0..=c)
                .rev()
                .find(|&i| matches!(lines[i], Line::Group(_)))
                .unwrap_or(c);
            let ctx = if g_line > 0 && matches!(lines[g_line - 1], Line::Section(_) | Line::Order) {
                g_line - 1
            } else {
                g_line
            };
            let end = c + lines[c + 1..]
                .iter()
                .take_while(|l| matches!(l, Line::Member(..)))
                .count();
            if c < top {
                top = if c - ctx < body { ctx } else { c };
            }
            if c >= top + body {
                top = c + 1 - body;
            }
            if ctx < top && c - ctx < body {
                top = ctx;
            }
            if end >= top + body {
                top = (end + 1 - body).min(c);
            }
        }
        top = top.min(lines.len().saturating_sub(body));
        self.vtop.set(top);
        top
    }

    /// What is chosen in groups `gs`: files, and bytes by kind.
    fn chosen_in(&self, gs: &[usize]) -> (u64, Vec<(Hue, u64)>) {
        let mut seen = std::collections::HashSet::new();
        let mut n = 0;
        let mut by: Vec<(Hue, u64)> = vec![(Hue::Copies, 0), (Hue::Similar, 0), (Hue::Video, 0)];
        for &g in gs {
            let gi = &self.layout.groups[g];
            for m in &gi.members {
                if m.role != Role::Keeper && self.selected.contains(&m.path) && seen.insert(&m.path)
                {
                    n += 1;
                    if let Some(e) = by.iter_mut().find(|e| e.0 == gi.kind) {
                        e.1 += m.size;
                    }
                }
            }
        }
        (n, by)
    }

    /// A section's line: its kind, its groups, what they free, what is
    /// chosen; open, a hairline to the edge; folded, its skyline.
    fn section_line(&self, t: &Theme, line: Line, open: bool) -> String {
        let (kind, gs): (Option<Hue>, Vec<usize>) = match line {
            Line::Section(k) => (
                Some(k),
                self.view
                    .iter()
                    .copied()
                    .filter(|&g| self.layout.groups[g].kind == k)
                    .collect(),
            ),
            _ => (None, self.view.clone()),
        };
        let hue = kind.map_or(PLATE, Hue::rgb);
        let label = match kind {
            Some(Hue::Copies) => "COPIES",
            Some(Hue::Similar) => "SIMILAR",
            Some(Hue::Video) => "VIDEO",
            _ => "GROUPS",
        };
        let max = t.width.saturating_sub(1);
        let reclaim: u64 = gs.iter().map(|&g| self.layout.groups[g].reclaim).sum();
        let (chosen, _) = self.chosen_in(&gs);
        let mut segs = vec![
            fold::count(t, gs.len() as u64, "group", "groups"),
            fold::figure(t, &fmt_bytes(reclaim)),
        ];
        if chosen > 0 {
            segs.push(fold::count(t, chosen, "chosen", "chosen"));
        }
        if kind.is_none() {
            segs.push(fold::dim(t, self.order.words()));
        }
        let lead = format!(" {}", t.paint_rgb_bold(hue, &format!("{label:<8}")));
        let room = max.saturating_sub(1 + 8 + 2);
        let text = fold::fit_segments(t, &segs, room);
        let mut out = format!("{lead}  {text}");
        let used = 1 + 8 + 2 + visible_width(&text);
        let rest = max.saturating_sub(used + 2);
        if open {
            let hair = if t.unicode { "─" } else { "-" };
            out.push_str("  ");
            out.push_str(&t.paint_rgb(GROOVE, &hair.repeat(rest)));
        } else if t.unicode && rest >= 4 {
            // the skyline: its groups tallest first, one cell each
            let mut v: Vec<u64> = gs.iter().map(|&g| self.layout.groups[g].reclaim).collect();
            v.sort_unstable_by(|a, b| b.cmp(a));
            v.truncate(rest.min(24));
            let top = v.first().copied().unwrap_or(1).max(1);
            let sky: String = v
                .iter()
                .map(|&x| {
                    if x == 0 {
                        BLOCKS[0]
                    } else {
                        BLOCKS[((7 * x / top) as usize).min(7)]
                    }
                })
                .collect();
            let w = v.len();
            out.push_str(&" ".repeat(rest + 2 - w));
            out.push_str(&t.paint_rgb(hue, &sky));
        }
        out
    }

    /// A group's line after the gutter: its keeper's name, how many files
    /// beside it, what it frees and a bar of that against the largest
    /// group shown, then what its files are.
    fn group_line(&self, t: &Theme, g: usize, room: usize, largest: u64) -> String {
        let gi = &self.layout.groups[g];
        let hue = gi.kind.rgb();
        let keeper = &gi.members[0].shown;
        let name = keeper.rsplit(['/', '\\']).next().unwrap_or(keeper);
        let n_col = (room * 28 / 100).clamp(12, 40);
        let bar = if room >= 56 {
            ((room - 40) / 3).clamp(5, 16)
        } else {
            0
        };
        let name = shorten_middle(name, n_col, t.glyphs.ellipsis);
        let mut out = String::new();
        out.push_str(&t.paint(Paint::Plate, &name));
        out.push_str(&" ".repeat(n_col - width(&name)));
        out.push_str(&t.paint_rgb(DIM, &format!("  +{:<3}", gi.members.len() - 1)));
        out.push_str(&t.paint(Paint::Plate, &format!("{:>9}", fmt_bytes(gi.reclaim))));
        let mut used = n_col + 6 + 9;
        if bar > 0 {
            let eighths = if largest > 0 {
                (gi.reclaim as f64 / largest as f64 * (bar * 8) as f64).round() as usize
            } else {
                0
            };
            // a slim rule in its track, to the half cell
            let halves = (eighths as f64 / 4.0).round() as usize;
            let (on, half, track) = if t.unicode {
                ("━", "╸", "━")
            } else {
                ("=", "-", " ")
            };
            let full = (halves / 2).min(bar);
            let mut cells = t.paint_rgb(hue, &on.repeat(full));
            let mut w = full;
            if halves % 2 == 1 && w < bar {
                cells.push_str(&t.paint_rgb(hue, half));
                w += 1;
            }
            if w == 0 && gi.reclaim > 0 {
                cells = t.paint_rgb(hue, half);
                w = 1;
            }
            cells.push_str(&t.paint_rgb(Rgb(0x26, 0x26, 0x26), &track.repeat(bar - w)));
            out.push_str("  ");
            out.push_str(&cells);
            used += 2 + bar;
        }
        // what its files are; for copies, every one the same bytes, where
        // they are
        let segs = if gi.kind == Hue::Copies {
            let folder = |p: &str| {
                p.rfind(['/', '\\'])
                    .map_or(String::new(), |i| p[..i].to_string())
            };
            let home = folder(keeper);
            // counted as they are: a folder's figures are its name
            let mut folders: Vec<(String, u64)> = Vec::new();
            for m in &gi.members[1..] {
                let f = folder(&m.shown);
                let k = if f == home {
                    "beside it".to_string()
                } else if f.is_empty() {
                    "in .".to_string()
                } else {
                    format!("in {f}")
                };
                match folders.iter_mut().find(|e| e.0 == k) {
                    Some(e) => e.1 += 1,
                    None => folders.push((k, 1)),
                }
            }
            folders.sort_by_key(|e| std::cmp::Reverse(e.1));
            fold::tally_segments(t, folders)
        } else {
            fold::tally_segments(
                t,
                fold::tally(gi.members[1..].iter().map(|m| m.words.clone())),
            )
        };
        let rest = room.saturating_sub(used + 2);
        if rest >= 6 && !segs.is_empty() {
            out.push_str("  ");
            out.push_str(&fold::fit_segments(t, &segs, rest));
        }
        out
    }

    /// A file's line in the groups panel: its role in a word, its path,
    /// its size (the rest is the comparison's).
    fn member_short(&self, t: &Theme, g: usize, m: usize, room: usize) -> String {
        let gi = &self.layout.groups[g];
        let mi = &gi.members[m];
        let (verb, paint) = match (m, gi.kind, mi.role) {
            (0, _, _) => ("keep", Paint::Keep),
            (_, _, Role::Locked(Lock::Kin)) => ("kin", Paint::Info),
            (_, _, Role::Locked(Lock::Raw)) => ("raw", Paint::Info),
            (_, Hue::Copies, _) => ("dupe", Paint::Dupe),
            _ => ("near", Paint::Dupe),
        };
        let size = fmt_bytes(mi.size);
        let path_w = room.saturating_sub(4 + 2 + 2 + size.len());
        let path = shorten_middle(&mi.shown, path_w, t.glyphs.ellipsis);
        format!(
            "{}  {}{}  {}",
            t.paint_bold(paint, &format!("{verb:<4}")),
            t.paint(if m == 0 { Paint::Plate } else { Paint::Dupe }, &path),
            " ".repeat(path_w.saturating_sub(width(&path))),
            t.paint_rgb(DIM, &size)
        )
    }

    /// The comparison: the keeper and the file under the cursor (the
    /// group's first other file when the cursor is on the group or its
    /// keeper) side by side as pictures, each with its name, folder,
    /// pixels, size and date; under them what the file is, then each
    /// piece of its evidence as a meter against its limit.
    fn comparison(
        &self,
        t: &Theme,
        w: usize,
        h: usize,
        thumbs: Option<&Thumbs>,
        clock: f64,
    ) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let Some(g) = self.cursor_group() else {
            return out;
        };
        let gi = &self.layout.groups[g];
        if gi.members.len() < 2 {
            return out;
        }
        let b = match self.rows.get(self.cursor) {
            Some(Row::Member(_, m)) if *m > 0 => *m,
            _ => 1,
        };
        let (ka, kb) = (&gi.members[0], &gi.members[b]);
        let hue = gi.kind.rgb();
        // a margin, the keeper's column, two cells, the sign, two cells,
        // the file's column, a margin
        let col = w.saturating_sub(7) / 2;
        let meters = &kb.meters;
        // under the pictures: a blank, the head and five caption lines, a
        // blank, the words, then a blank and the meters
        let text_rows = 1
            + 6
            + 1
            + 1
            + if meters.is_empty() {
                0
            } else {
                1 + meters.len()
            };
        let aspect = [ka.dims, kb.dims]
            .iter()
            .flatten()
            .map(|&(x, y)| x as f64 / y.max(1) as f64)
            .fold(None, |a: Option<f64>, b| Some(a.map_or(b, |a| a.max(b))))
            .unwrap_or(4.0 / 3.0);
        // the pictures as tall as their width lets them be, as the panel
        // lets them be
        let want = ((col as f64 / (2.0 * aspect)).ceil() as usize).max(1);
        let ch = want.min(h.saturating_sub(text_rows + 2));
        out.push(String::new());
        if ch >= 3 && t.color {
            let pic = |m: &MemberInfo| -> Vec<String> {
                let thumb = match (thumbs, &m.visual) {
                    (Some(th), Some(v)) => th.get(v, col as u32, ch as u32),
                    _ => Thumb::Unreadable,
                };
                let own = m.dims.map_or(aspect, |(x, y)| x as f64 / y.max(1) as f64);
                picture(t, &thumb, col, ch, own, clock)
            };
            let (pa, pb) = (pic(ka), pic(kb));
            let sign = if gi.kind == Hue::Copies { "=" } else { "≈" };
            for r in 0..ch {
                let a = pa.get(r).cloned().unwrap_or_default();
                let mid = if r == ch / 2 {
                    t.paint_rgb_bold(hue, sign)
                } else {
                    " ".to_string()
                };
                let pad = col.saturating_sub(visible_width(&a));
                out.push(format!(
                    " {a}{}  {mid}  {}",
                    " ".repeat(pad),
                    pb.get(r).cloned().unwrap_or_default()
                ));
            }
            out.push(String::new());
        }
        // the captions under their pictures; what differs between the two
        // is lit on the file's side, what is the same dim on both
        let folder_of = |m: &MemberInfo| -> String {
            match m.shown.rfind(['/', '\\']) {
                Some(k) => m.shown[..k].to_string(),
                None => m
                    .path
                    .parent()
                    .and_then(|p| p.file_name())
                    .map_or_else(|| ".".to_string(), |n| n.to_string_lossy().into_owned()),
            }
        };
        let name_of = |m: &MemberInfo| -> String {
            m.shown
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or(&m.shown)
                .to_string()
        };
        // pixels as the scan read them, or as the picture decoded (a
        // byte-for-byte copy is never decoded by the scan)
        let dims_of = |m: &MemberInfo| -> String {
            let decoded = match (thumbs, &m.visual) {
                (Some(th), Some(v)) => match th.get(v, col as u32, ch.max(1) as u32) {
                    Thumb::Ready(_, _, d) => Some(d),
                    _ => None,
                },
                _ => None,
            };
            m.dims
                .or(decoded)
                .map(|(x, y)| format!("{x}{}{y}", t.glyphs.times))
                .unwrap_or_default()
        };
        let verb_b = match (gi.kind, kb.role) {
            (_, Role::Locked(Lock::Kin)) => "kin",
            (_, Role::Locked(Lock::Raw)) => "raw",
            (Hue::Copies, _) => "dupe",
            _ => "near",
        };
        let rows: Vec<(String, String, bool)> = vec![
            (name_of(ka), name_of(kb), true),
            (folder_of(ka), folder_of(kb), false),
            (dims_of(ka), dims_of(kb), false),
            (fmt_bytes(ka.size), fmt_bytes(kb.size), false),
            (ka.date.clone(), kb.date.clone(), false),
        ];
        out.push(format!(
            " {}{}     {}",
            t.paint_rgb_bold(AMBER, "keep"),
            " ".repeat(col.saturating_sub(4)),
            t.paint_rgb_bold(hue, verb_b)
        ));
        for (va, vb, name) in rows {
            let differs = va != vb;
            let a = shorten_middle(&va, col, t.glyphs.ellipsis);
            let bb = shorten_middle(&vb, col, t.glyphs.ellipsis);
            let pa = if name {
                t.paint_rgb_bold(PLATE, &a)
            } else {
                t.paint_rgb(if differs { PLATE } else { DIM }, &a)
            };
            let pb = if name {
                t.paint_rgb_bold(PLATE, &bb)
            } else if differs {
                t.paint_rgb_bold(hue, &bb)
            } else {
                t.paint_rgb(DIM, &bb)
            };
            out.push(format!(
                " {pa}{}     {pb}",
                " ".repeat(col.saturating_sub(width(&a)))
            ));
        }
        out.push(String::new());
        let words = if gi.kind == Hue::Copies {
            "every byte the same".to_string()
        } else if kb.words.is_empty() {
            "the same file".to_string()
        } else {
            kb.words.clone()
        };
        out.push(format!(
            " {}",
            t.paint_rgb_bold(hue, &crate::util::fit(&words, w.saturating_sub(2)))
        ));
        if !meters.is_empty() {
            out.push(String::new());
            let bar = w
                .saturating_sub(1 + 6 + 2 + 2 + 4 + 2 + 6 + 2 + 2 + 1)
                .clamp(8, 48);
            for m in meters {
                out.push(format!(" {}", meter(t, m, bar)));
            }
        }
        out
    }

    /// The gauge: what is chosen, of all the groups shown can free, in
    /// The keys for what the comparison shows, most used first: the two
    /// side by side; this file as the keeper, its folder's copies kept;
    /// its folder.
    fn pair_keys(&self) -> Vec<(String, String)> {
        let mut v: Vec<(String, String)> = Vec::new();
        if self.target().is_some() {
            v.push(("v".into(), "view".into()));
        }
        match self.rows.get(self.cursor).copied() {
            Some(Row::Member(g, m)) => {
                if m > 0 {
                    v.push(("k".into(), "keep this".into()));
                }
                let folder = folder_of(&self.layout.groups[g].members[m].shown).to_string();
                v.push((
                    "f".into(),
                    if self.prefer.as_deref() == Some(folder.as_str()) {
                        "any folder".into()
                    } else {
                        "keep this folder".into()
                    },
                ));
                v.push(("o".into(), "reveal".into()));
            }
            Some(Row::Head(g)) => {
                if !self.shots(g).is_empty() {
                    v.push(("c".into(), "other shots".into()));
                }
                v.push(("o".into(), "reveal".into()));
            }
            None => {}
        }
        v
    }

    /// The bottom line: on the left what is chosen of what there is (or
    /// the prompt of a mode), on the right the keys that act on it as
    /// keycaps, `?` for the rest.
    fn bottom_line(&self, t: &Theme, panels: bool, list_keys: &[(String, String)]) -> String {
        let g = t.glyphs;
        let (n, bytes, hand) = self.stats();
        let all: u64 = self
            .view
            .iter()
            .map(|&g| self.layout.groups[g].reclaim)
            .sum();
        let pairs = |v: &[(&str, &str)]| -> Vec<(String, String)> {
            v.iter()
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .collect()
        };
        let lit = |s: &str| t.paint_rgb_bold(PLATE, s);
        let dim = |s: &str| t.paint_rgb(DIM, s);
        let (left, keys): (Vec<String>, Vec<(String, String)>) = match &self.mode {
            Mode::Browse => {
                let mut l = vec![String::new()];
                if hand > 0 {
                    l.push(t.paint(Paint::Warn, &format!("{hand} by hand")));
                }
                if let Some(p) = &self.prefer {
                    l.push(format!(
                        "{} {}",
                        dim("keeping in"),
                        t.paint(Paint::Plate, if p.is_empty() { "." } else { p })
                    ));
                }
                // on one screen the groups' frame carries these
                if !panels {
                    if self.view.len() < self.layout.groups.len() {
                        l.push(dim(&format!(
                            "{} of {} groups",
                            fmt_count(self.view.len() as u64),
                            fmt_count(self.layout.groups.len() as u64)
                        )));
                    }
                    if self.order != Order::Listing {
                        l.push(dim(&list_keys[0].1));
                    }
                }
                if let Some(m) = &self.message {
                    l.push(t.paint(Paint::Warn, m));
                }
                let mut k: Vec<(String, String)> = Vec::new();
                if n > 0 {
                    k.push((
                        "m".into(),
                        format!("milk {} {}", g.arrow, crate::MILKED_DIR),
                    ));
                    k.push(("p".into(), format!("purge {} Recycle Bin", g.arrow)));
                } else {
                    k.push(("space".into(), "select".into()));
                }
                k.push(("?".into(), "keys".into()));
                (l, k)
            }
            Mode::Confirm(act) => {
                let (verb, dest) = match act {
                    Act::Milk => ("milk", crate::MILKED_DIR),
                    Act::Purge => ("purge", "Recycle Bin"),
                };
                let mut what = format!(
                    "{} {} {} {}",
                    t.paint_rgb_bold(AMBER, verb),
                    lit(&fmt_count(n)),
                    dim(&format!("{} {dest} ·", g.arrow)),
                    lit(&fmt_bytes(bytes))
                );
                if hand > 0 {
                    what.push_str(&format!(
                        "  {}",
                        t.paint(Paint::Warn, &format!("{hand} by hand"))
                    ));
                }
                let shots = self.chosen_shots();
                if shots > 0 {
                    what.push_str(&format!(
                        "  {}",
                        t.paint(
                            Paint::Warn,
                            &format!("{shots} other shot{}", if shots == 1 { "" } else { "s" })
                        )
                    ));
                }
                (vec![what], pairs(&[("enter", "go"), ("esc", "back")]))
            }
            Mode::Filter { typed, .. } => (
                vec![format!(
                    "{} {}{}  {}",
                    t.paint_rgb_bold(AMBER, "/"),
                    lit(typed),
                    t.paint_rgb(AMBER, "_"),
                    dim(&format!(
                        "{} of {} groups",
                        fmt_count(self.view.len() as u64),
                        fmt_count(self.layout.groups.len() as u64)
                    ))
                )],
                pairs(&[("enter", "keep"), ("esc", "back")]),
            ),
            Mode::GoTo(typed) => (
                vec![format!(
                    "{} {}{}  {}",
                    dim("go to group"),
                    lit(typed),
                    t.paint_rgb(AMBER, "_"),
                    dim(&format!("1–{}", self.layout.groups.len()))
                )],
                pairs(&[("enter", "go"), ("esc", "back")]),
            ),
        };
        // what is chosen, as far as the line lets it say: of what there
        // is, then the bytes alone, then the count alone
        let browse = matches!(self.mode, Mode::Browse);
        let forms = [
            format!(
                "{} {}  {} {} {}",
                lit(&fmt_count(n)),
                dim("selected"),
                lit(&fmt_bytes(bytes)),
                dim("of"),
                dim(&fmt_bytes(all.max(bytes)))
            ),
            format!(
                "{} {}  {}",
                lit(&fmt_count(n)),
                dim("selected"),
                lit(&fmt_bytes(bytes))
            ),
            format!("{} {}", lit(&fmt_count(n)), dim("selected")),
        ];
        // the keys keep the room the shortest of those leaves them
        let least = if browse {
            visible_width(&forms[2]) + 3
        } else {
            24
        };
        let keys_room = t.width.saturating_sub(least + 1).max(12);
        // `?` is the way to every other key: the keys before it go first
        let mut keys = keys;
        let tightest = |k: &[(String, String)]| -> usize {
            k.iter()
                .map(|(key, words)| {
                    width(key) + 3 + width(words.split_whitespace().next().unwrap_or(""))
                })
                .sum::<usize>()
                + 2 * k.len().saturating_sub(1)
                + 1
        };
        while keys.len() > 2 && tightest(&keys) > keys_room {
            keys.remove(keys.len() - 2);
        }
        let right = fit_parts(t, &keys, keys_room);
        let right_w = visible_width(&right);
        let room = t.width.saturating_sub(right_w + 3);
        let joined = |l: &[String]| format!(" {}", l.join(&dim("  ·  ")));
        let mut l = left;
        if browse {
            let extras = l.split_off(1);
            l = forms
                .iter()
                .map(|f| {
                    let mut v = vec![f.clone()];
                    v.extend(extras.iter().cloned());
                    while v.len() > 1 && visible_width(&joined(&v)) > room {
                        v.pop();
                    }
                    v
                })
                .find(|v| visible_width(&joined(v)) <= room)
                .unwrap_or_else(|| vec![forms[2].clone()]);
        } else {
            while l.len() > 1 && visible_width(&joined(&l)) > room {
                l.pop();
            }
        }
        let mut left = joined(&l);
        if visible_width(&left) > room {
            left = crate::util::cut_sgr(&left, room);
        }
        let gap = t
            .width
            .saturating_sub(1 + visible_width(&left) + right_w)
            .max(1);
        format!("{left}{}{right}", " ".repeat(gap))
    }

    /// The key map `?` opens, on a card at most `w` × `h`: every key by
    /// what it is for, four kinds in two columns, each key a keycap set
    /// right against its words, the kind's name over the words; the words
    /// shortened forms where the card is narrow. Framed, its title KEYS, `?` or esc to close on its edge.
    fn key_card(&self, t: &Theme, w: usize, h: usize) -> Vec<String> {
        let g = t.glyphs;
        let ud = format!("{} {}", g.up, g.down);
        // (key, words, words where the card is narrow)
        type Entry<'a> = (&'a str, &'a str, &'a str);
        let sections: [(&str, Vec<Entry>); 4] = [
            (
                "MOVE",
                vec![
                    (ud.as_str(), "a line", "a line"),
                    ("PgUp PgDn", "a page", "a page"),
                    ("Home End", "the first, the last", "first, last"),
                    ("g", "go to a group by number", "go to group"),
                ],
            ),
            (
                "CHOOSE",
                vec![
                    ("space", "the file, or the group's copies", "file or group"),
                    ("a", "every copy shown, or none", "all or none"),
                    ("k", "keep this file instead", "keep this one"),
                    ("f", "keep the copies in its folder", "keep its folder"),
                    ("c", "the group's other shots too", "other shots too"),
                ],
            ),
            (
                "LOOK",
                vec![
                    ("v", "the two side by side", "side by side"),
                    ("o", "show it in its folder", "show in folder"),
                    ("/", "filter the groups by path", "filter by path"),
                    ("s", "sort the groups", "sort"),
                ],
            ),
            (
                "ACT",
                vec![
                    ("m", "milk the selection to _milked", "milk"),
                    ("p", "purge it to the Recycle Bin", "purge"),
                    ("n", "preview it: a dry run", "dry run"),
                    ("u", "undo the last change", "undo"),
                    ("q", "leave", "leave"),
                ],
            ),
        ];
        let kw = sections
            .iter()
            .flat_map(|s| s.1.iter())
            .map(|e| width(e.0) + 2)
            .max()
            .unwrap_or(3);
        let words = |long: bool| -> usize {
            sections
                .iter()
                .flat_map(|s| s.1.iter())
                .map(|e| width(if long { e.1 } else { e.2 }))
                .max()
                .unwrap_or(0)
        };
        // a column: key, two cells, words; two columns with four between;
        // the frame and a cell of room each side
        let col = |long: bool| kw + 2 + words(long);
        let span = |c: usize, two: bool| if two { 2 * c + 4 } else { c } + 4;
        let (long, two) = [(true, true), (false, true), (true, false), (false, false)]
            .into_iter()
            .find(|&(l, two)| span(col(l), two) <= w)
            .unwrap_or((false, false));
        let cw = col(long);
        let inner_w = span(cw, two) - 2;
        let block = |(name, keys): &(&str, Vec<Entry>)| -> Vec<String> {
            let mut v = vec![format!(
                "{}{}",
                " ".repeat(kw + 2),
                t.paint_rgb_bold(AMBER, name)
            )];
            // the caps right-aligned on the words' column
            for (k, l, sh) in keys {
                let pad = kw - (width(k) + 2);
                let text = crate::util::fit(if long { l } else { sh }, cw - kw - 2);
                v.push(format!(
                    "{}{}  {}",
                    " ".repeat(pad),
                    cap(t, k),
                    t.paint_rgb(DIM.lerp(PLATE, 0.55), &text)
                ));
            }
            v
        };
        let pad_to =
            |l: &str, n: usize| format!("{l}{}", " ".repeat(n.saturating_sub(visible_width(l))));
        let mut body: Vec<String> = vec![String::new()];
        let groups: Vec<Vec<&(&str, Vec<Entry>)>> = if two {
            sections.chunks(2).map(|c| c.iter().collect()).collect()
        } else {
            sections.iter().map(|s| vec![s]).collect()
        };
        for row in groups {
            let blocks: Vec<Vec<String>> = row.iter().map(|s| block(s)).collect();
            let n = blocks.iter().map(Vec::len).max().unwrap_or(0);
            for i in 0..n {
                let mut l = String::from(" ");
                for (j, b) in blocks.iter().enumerate() {
                    if j > 0 {
                        l.push_str("    ");
                    }
                    l.push_str(&pad_to(b.get(i).map_or("", String::as_str), cw));
                }
                body.push(pad_to(&l, inner_w));
            }
            body.push(String::new());
        }
        // what does not fit the height is left off, the last kinds first
        body.truncate(h.saturating_sub(2));
        let edge = PLATE.lerp(GROOVE, 0.55);
        let (foot, foot_w) = edge_keys(
            t,
            edge,
            &[("?".into(), "or esc closes".into())],
            inner_w.saturating_sub(8),
        );
        framed(
            t,
            &t.paint_rgb_bold(PLATE, "KEYS"),
            ("", 0),
            (&foot, foot_w),
            edge,
            &body,
            inner_w,
            body.len(),
        )
    }

    /// each kind's colour; `shown` rolls it.
    fn gauge_line(&self, t: &Theme, shown: Option<u64>) -> String {
        let (_, by) = self.chosen_in(&self.view);
        let chosen: u64 = by.iter().map(|e| e.1).sum();
        let all: u64 = self
            .view
            .iter()
            .map(|&g| self.layout.groups[g].reclaim)
            .sum();
        let total = all.max(chosen).max(1);
        let scale = match shown {
            Some(s) if chosen > 0 => s as f64 / chosen as f64,
            Some(_) => 0.0,
            None => 1.0,
        };
        let cells = t.width.saturating_sub(2);
        let (on, off) = match (t.unicode, t.color) {
            (true, true) => ("▄", "▄"),
            (true, false) => ("▄", "▁"),
            _ => ("#", "."),
        };
        let mut out = String::from(" ");
        let mut drawn = 0usize;
        let mut acc = 0.0;
        for (hue, bytes) in by {
            if bytes == 0 {
                continue;
            }
            acc += bytes as f64 * scale;
            let upto = ((acc / total as f64) * cells as f64).round() as usize;
            let upto = upto.max(drawn + usize::from(scale >= 1.0)).min(cells);
            out.push_str(&t.paint_rgb(hue.rgb(), &on.repeat(upto - drawn)));
            drawn = upto;
        }
        out.push_str(&t.paint_rgb(GROOVE, &off.repeat(cells - drawn)));
        out
    }

    /// The full screen, exactly `total_height` lines: the title, a blank
    /// line, the body, the gauge, the bottom line (what is chosen and the
    /// keys that act on it). `lines` are
    /// the full listing's lines by row (a group's files when it unfolds).
    pub fn screen(
        &self,
        t: &Theme,
        title: &str,
        lines: &HashMap<Row, String>,
        total_height: usize,
        frame: &Frame,
        thumbs: Option<&Thumbs>,
    ) -> Vec<String> {
        let g = t.glyphs;
        let max = t.width.saturating_sub(1);
        let mut out: Vec<String> = Vec::with_capacity(total_height);
        out.push(title.to_string());
        out.push(String::new());
        let body_rows = total_height.saturating_sub(4);
        // two panels when there is room for them: the groups, framed, and
        // the comparison; the groups' lines then fit the left frame
        let panels = t.width >= PANELS_MIN && body_rows >= 12;
        let left_w = if panels {
            (t.width * 42 / 100).clamp(48, 72)
        } else {
            max
        };
        let body = if panels { body_rows - 2 } else { body_rows };
        let max = if panels { left_w - 2 } else { max };
        let mut list: Vec<String> = Vec::with_capacity(body);
        let (drawn, cur) = self.lines();
        let top = self.window(&drawn, cur, body);
        let largest = self
            .view
            .iter()
            .map(|&g| self.layout.groups[g].reclaim)
            .max()
            .unwrap_or(0);
        self.cursor_at.set(None);
        for (i, &line) in drawn.iter().enumerate().skip(top).take(body) {
            let is_cur = cur == Some(i);
            let kind = match line {
                Line::Group(g) | Line::Member(g, _) => Some(self.layout.groups[g].kind),
                Line::Section(k) => Some(k),
                Line::Order => None,
            };
            let gutter = |row: Row| -> String {
                let (mark, role) = match self.mark(row) {
                    Mark::None => (" ", Paint::Dupe),
                    Mark::On => (g.sel_on, Paint::Keep),
                    Mark::Off => (g.sel_off, Paint::Dupe),
                    Mark::Part => (g.sel_part, Paint::Keep),
                    Mark::Lock => (g.sel_lock, Paint::Dupe),
                };
                let pointer = if is_cur {
                    t.paint_rgb_bold(AMBER.lerp(Rgb::WHITE, 0.5 * frame.breath), g.pointer)
                } else {
                    " ".to_string()
                };
                format!("{pointer}{} ", t.paint_bold(role, mark))
            };
            let mut tt = *t;
            tt.width = max + 1;
            let t = &tt;
            let text = match line {
                Line::Section(_) => {
                    let open = drawn
                        .get(i + 1)
                        .is_some_and(|l| matches!(l, Line::Group(_)));
                    self.section_line(t, line, open)
                }
                Line::Order => self.section_line(t, line, true),
                Line::Group(gi) => format!(
                    "{}{}",
                    gutter(Row::Head(gi)),
                    self.group_line(t, gi, max.saturating_sub(3), largest)
                ),
                Line::Member(gi, m) if panels => {
                    let n = self.layout.groups[gi].members.len();
                    let tree = if t.unicode {
                        if m + 1 == n {
                            "└"
                        } else {
                            "├"
                        }
                    } else if m + 1 == n {
                        "`"
                    } else {
                        "|"
                    };
                    let hue = self.layout.groups[gi].kind.rgb();
                    format!(
                        "{}{} {}",
                        gutter(Row::Member(gi, m)),
                        t.paint_rgb(hue.lerp(GROOVE, 0.35), tree),
                        self.member_short(t, gi, m, max.saturating_sub(5))
                    )
                }
                Line::Member(gi, m) => {
                    let n = self.layout.groups[gi].members.len();
                    let tree = match (t.unicode, m + 1 == n) {
                        (true, false) => "├",
                        (true, true) => "└",
                        (false, false) => "|",
                        (false, true) => "`",
                    };
                    let hue = self.layout.groups[gi].kind.rgb();
                    let listed = lines
                        .get(&Row::Member(gi, m))
                        .map(String::as_str)
                        .unwrap_or("");
                    format!(
                        "{}{} {}",
                        gutter(Row::Member(gi, m)),
                        t.paint_rgb(hue.lerp(GROOVE, 0.35), tree),
                        listed.strip_prefix("   ").unwrap_or(listed)
                    )
                }
            };
            // what just unfolded comes in from the groove
            let fresh = match (frame.fresh, line) {
                (Fresh::Group(fg), Line::Member(gi, _)) => fg == gi,
                (Fresh::Section(k), Line::Group(gi)) => self.layout.groups[gi].kind == k,
                _ => false,
            };
            let text = if fresh && frame.fade < 1.0 && t.color {
                t.paint_rgb(GROOVE.lerp(PLATE, frame.fade), &strip_sgr(&text))
            } else {
                text
            };
            if is_cur {
                self.cursor_at.set(Some(list.len()));
                list.push(lit(t, &text, kind.map_or(PLATE, Hue::rgb), max));
            } else {
                list.push(text);
            }
        }
        if self.rows.is_empty() && !self.filter.is_empty() {
            list.push(format!(
                "   {}",
                t.paint(
                    Paint::Dupe,
                    &crate::util::fit(
                        &format!("no group has a path with {:?}", self.filter),
                        t.width.saturating_sub(4)
                    )
                )
            ));
        }
        // the keys that act on a panel sit on its edge; the rest are on `?`
        let edge = PLATE.lerp(GROOVE, 0.75);
        let shown = self.view.len();
        let groups_n = self.layout.groups.len();
        let list_keys: Vec<(String, String)> = {
            let mut v = vec![("s".to_string(), self.order.words().to_string())];
            v.push((
                "/".into(),
                if self.filter.is_empty() {
                    "filter".into()
                } else {
                    self.filter.clone()
                },
            ));
            v
        };
        if panels {
            let here = self.cursor_group().map(|g| &self.layout.groups[g]);
            let hue = here.map_or(PLATE, |g| g.kind.rgb());
            let mut lt = t.paint_rgb_bold(PLATE, "GROUPS");
            lt.push_str(&t.paint_rgb(
                DIM,
                &if shown < groups_n {
                    format!(
                        "  {} of {}",
                        fmt_count(shown as u64),
                        fmt_count(groups_n as u64)
                    )
                } else {
                    format!("  {}", fmt_count(groups_n as u64))
                },
            ));
            let room = (left_w - 2).saturating_sub(visible_width(&lt) + 8);
            let (tail, tail_w) = edge_keys(t, edge, &list_keys, room);
            let left = framed(
                t,
                &lt,
                (&tail, tail_w),
                ("", 0),
                edge,
                &list,
                left_w - 2,
                body,
            );
            let right_w = t.width.saturating_sub(left_w + 2).saturating_sub(2);
            let rt = match here {
                Some(g) => t.paint_rgb_bold(
                    hue,
                    &shorten_middle(
                        g.members[0].shown.rsplit(['/', '\\']).next().unwrap_or(""),
                        right_w.saturating_sub(8).max(8),
                        t.glyphs.ellipsis,
                    ),
                ),
                None => String::new(),
            };
            let mut cmp = self.comparison(t, right_w, body, thumbs, frame.clock);
            // the comparison sits in the middle of its frame
            let lift = body.saturating_sub(cmp.len()) / 2;
            cmp.splice(0..0, std::iter::repeat_n(String::new(), lift));
            let right_edge = edge.lerp(hue, 0.45);
            let (foot, foot_w) =
                edge_keys(t, right_edge, &self.pair_keys(), right_w.saturating_sub(6));
            let right = framed(
                t,
                &rt,
                ("", 0),
                (&foot, foot_w),
                right_edge,
                &cmp,
                right_w,
                body,
            );
            // the cursor's line counts the frame's top edge
            if let Some(c) = self.cursor_at.get() {
                self.cursor_at.set(Some(c + 3));
            }
            for (l, r) in left.iter().zip(&right) {
                out.push(format!("{l} {r}"));
            }
        } else {
            if let Some(c) = self.cursor_at.get() {
                self.cursor_at.set(Some(c + 2));
            }
            out.extend(list);
            while out.len() < 2 + body {
                out.push(String::new());
            }
        }
        out.push(self.gauge_line(t, frame.gauge));
        out.push(self.bottom_line(t, panels, &list_keys));
        if self.keys_open {
            // the key map on a card over the body, the screen faded under
            // it; the cursor's line is not drawn again over it
            self.cursor_at.set(None);
            let card = self.key_card(t, t.width.saturating_sub(1), body_rows);
            let cw = card.first().map_or(0, |l| visible_width(l));
            let x = t.width.saturating_sub(1 + cw) / 2;
            let y = 2 + body_rows.saturating_sub(card.len()) / 2;
            for l in out.iter_mut().skip(2).take(body_rows) {
                *l = fade(t, l);
            }
            for (k, c) in card.iter().enumerate() {
                if let Some(l) = out.get_mut(y + k) {
                    *l = format!(
                        "{}{c}{}",
                        crate::util::slice_sgr(l, 0, x),
                        crate::util::slice_sgr(l, x + cw, usize::MAX)
                    );
                }
            }
        }
        out.truncate(total_height.max(6));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::super::state::tests::{layout, theme};
    use super::*;

    fn listed(st: &State) -> HashMap<Row, String> {
        all_rows(&st.layout)
            .into_iter()
            .map(|r| {
                let l = match r {
                    Row::Head(g) => format!(" GROUP {:03}", g + 1),
                    Row::Member(g, m) => format!(
                        "   line {} {}",
                        g,
                        st.layout.groups[g].members[m].path.display()
                    ),
                };
                (r, l)
            })
            .collect()
    }

    #[test]
    fn the_screen_is_exactly_the_terminal_and_fits_its_width() {
        for (h, all_open) in [(20usize, true), (10, false)] {
            let st = State::new(layout(), h - 4);
            for w in [40usize, 60, 80, 120, 200] {
                let t = theme(w);
                let screen = st.screen(
                    &t,
                    " dedupe find  root",
                    &listed(&st),
                    h,
                    &Frame::STILL,
                    None,
                );
                assert_eq!(screen.len(), h, "{screen:#?}");
                for l in &screen {
                    assert!(width(l) <= w, "{w}: {l:?}");
                }
                // from 100 columns (and room for them) the groups and the
                // comparison stand framed side by side
                let panels = w >= PANELS_MIN && h - 4 >= 12;
                assert_eq!(screen[2].starts_with("╭─ GROUPS"), panels, "{screen:#?}");
                // the cursor starts on a (1).jpg: its section open, its
                // group unfolded on a tree, the pointer on the file
                assert!(screen.iter().any(|l| l.contains(" COPIES")), "{screen:#?}");
                let cursor = if panels {
                    "│▸■ └ dupe  a (1).jpg"
                } else {
                    "▸■ └ line 0 a (1).jpg"
                };
                assert!(
                    screen.iter().any(|l| l.starts_with(cursor)),
                    "{cursor}\n{screen:#?}"
                );
                // room for every group: the other kind open too; else
                // folded to its one line
                assert!(
                    screen.iter().any(|l| l.contains(" SIMILAR ")),
                    "{screen:#?}"
                );
                assert_eq!(
                    screen.iter().any(|l| l.contains("b.png")),
                    all_open,
                    "{screen:#?}"
                );
                // no group's files but the cursor's
                assert!(
                    !screen.iter().any(|l| l.contains("b_next.jpg")),
                    "{screen:#?}"
                );
                // one line under the gauge: what is chosen, the keys
                // that act on it, `?` for the rest
                let bottom = screen.last().unwrap();
                assert!(bottom.contains("2 selected"), "{screen:#?}");
                assert!(bottom.contains("[?] keys"), "{w}: {bottom}");
                if w >= 60 {
                    assert!(bottom.contains("[m] milk"), "{w}: {bottom}");
                }
            }
        }
    }

    #[test]
    fn moving_into_the_next_kind_folds_the_last_and_unfolds_the_group() {
        let mut st = State::new(layout(), 30);
        // down from the last file of the copies: the first similar group
        st.handle(Key::Down);
        let (drawn, cur) = st.lines();
        assert_eq!(
            drawn,
            vec![
                Line::Section(Hue::Copies),
                Line::Group(0),
                Line::Section(Hue::Similar),
                Line::Group(1),
                Line::Member(1, 0),
                Line::Member(1, 1),
                Line::Member(1, 2),
                Line::Member(1, 3),
                Line::Group(2),
            ]
        );
        assert_eq!(cur, Some(3));
        let t = theme(100);
        let screen = st.screen(&t, "", &listed(&st), 30, &Frame::STILL, None);
        let row = screen.iter().find(|l| l.starts_with("│▸")).unwrap();
        // the group's line: its keeper, three beside it, what it frees,
        // what its files are
        assert!(
            row.contains("b.png") && row.contains("+3") && row.contains("410 B"),
            "{row}"
        );
        assert!(row.contains("near copy 3"), "{row}");
    }

    #[test]
    fn question_mark_lays_every_key_on_a_card_and_esc_takes_it_away() {
        for (w, h) in [
            (40usize, 16usize),
            (60, 20),
            (80, 24),
            (100, 30),
            (150, 40),
            (200, 50),
        ] {
            let mut st = State::new(layout(), h - 4);
            let t = theme(w);
            let plain = st.screen(
                &t,
                " dedupe find  root",
                &listed(&st),
                h,
                &Frame::STILL,
                None,
            );
            assert_eq!(st.handle(Key::Char('?')), Effect::None);
            let screen = st.screen(
                &t,
                " dedupe find  root",
                &listed(&st),
                h,
                &Frame::STILL,
                None,
            );
            assert_eq!(screen.len(), h);
            for l in &screen {
                assert!(width(l) <= w, "{w}: {l:?}");
            }
            let card: Vec<&String> = screen.iter().filter(|l| l.contains("KEYS")).collect();
            assert_eq!(card.len(), 1, "{w}: {screen:#?}");
            // every kind on the card where it has the rows for them
            if h >= 24 {
                for kind in ["MOVE", "CHOOSE", "LOOK", "ACT"] {
                    assert!(
                        screen.iter().any(|l| l.contains(kind)),
                        "{w} {kind}: {screen:#?}"
                    );
                }
                for k in ["[space]", "[k]", "[f]", "[v]", "[/]", "[n]", "[u]"] {
                    assert!(screen.iter().any(|l| l.contains(k)), "{w} {k}: {screen:#?}");
                }
            }
            // the title and the bottom line are not under it
            assert_eq!(screen[0], plain[0]);
            assert_eq!(screen.last(), plain.last());
            // the keys act under it; esc takes the card away, not the
            // workspace, and the screen is as it was
            assert_eq!(st.handle(Key::Esc), Effect::None);
            let back = st.screen(
                &t,
                " dedupe find  root",
                &listed(&st),
                h,
                &Frame::STILL,
                None,
            );
            assert_eq!(back, plain, "{w}");
            assert_eq!(st.handle(Key::Esc), Effect::Quit);
        }
    }

    #[test]
    fn the_pair_keys_follow_the_cursor() {
        let mut st = State::new(layout(), 30);
        // on a dupe: view, keep this, keep this folder, reveal
        let keys: Vec<String> = st.pair_keys().into_iter().map(|k| k.0).collect();
        assert_eq!(keys, ["v", "k", "f", "o"]);
        // on the keeper: no k
        st.handle(Key::Up);
        let keys: Vec<String> = st.pair_keys().into_iter().map(|k| k.0).collect();
        assert_eq!(keys, ["v", "f", "o"], "{:?}", st.rows.get(st.cursor));
    }
}
