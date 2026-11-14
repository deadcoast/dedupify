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

use super::state::*;
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

/// Eighths of a cell, for the groups' bars.
const EIGHTHS: [&str; 8] = ["", "▏", "▎", "▍", "▌", "▋", "▊", "▉"];

/// `▸ m milk 12 → _milked  p purge …` fitted to `width`: parts whose text
/// would overflow keep their first two words, then their first, then the
/// trailing parts are dropped. Returns the painted line.
pub fn fit_parts(t: &Theme, lead: &str, parts: &[(String, String)], width_cells: usize) -> String {
    let render = |take: usize, count: usize| -> (String, String) {
        let mut plain = format!(" {lead}");
        let mut painted = format!(" {}", t.paint_bold(Paint::Keep, lead));
        for (key, text) in parts.iter().take(count) {
            let text: String = if take == 0 {
                text.clone()
            } else {
                text.split_whitespace()
                    .take(take)
                    .collect::<Vec<_>>()
                    .join(" ")
            };
            let sep = if plain.trim().is_empty() { " " } else { "  " };
            plain.push_str(&format!("{sep}{key} {text}"));
            painted.push_str(&format!(
                "{sep}{} {}",
                t.paint_bold(Paint::Keep, key),
                t.paint(Paint::Plate, &text)
            ));
        }
        (plain, painted)
    };
    // whole texts, then two words, then one; then the trailing parts go
    // (the bars list the most used keys first)
    for take in [0usize, 2, 1] {
        let c = render(take, parts.len());
        if width(&c.0) <= width_cells {
            return c.1;
        }
    }
    for count in (1..parts.len()).rev() {
        let c = render(1, count);
        if width(&c.0) <= width_cells {
            return c.1;
        }
    }
    render(1, 1).1
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
        let bar = if room >= 70 {
            (room / 10).clamp(6, 16)
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
            let cells = if t.unicode {
                format!("{}{}", "█".repeat(eighths / 8), EIGHTHS[eighths % 8])
            } else {
                "#".repeat(eighths.div_ceil(8))
            };
            let w = width(&cells);
            out.push_str("  ");
            out.push_str(&t.paint_rgb(hue, &cells));
            out.push_str(&" ".repeat(bar - w.min(bar)));
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

    /// The gauge: what is chosen, of all the groups shown can free, in
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
    /// line, the body, the gauge, the status line, the keys. `lines` are
    /// the full listing's lines by row (a group's files when it unfolds).
    pub fn screen(
        &self,
        t: &Theme,
        title: &str,
        lines: &HashMap<Row, String>,
        total_height: usize,
        frame: &Frame,
    ) -> Vec<String> {
        let g = t.glyphs;
        let max = t.width.saturating_sub(1);
        let mut out: Vec<String> = Vec::with_capacity(total_height);
        out.push(title.to_string());
        out.push(String::new());
        let body = total_height.saturating_sub(6);
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
                self.cursor_at.set(Some(out.len()));
                out.push(lit(t, &text, kind.map_or(PLATE, Hue::rgb), max));
            } else {
                out.push(text);
            }
        }
        if self.rows.is_empty() && !self.filter.is_empty() {
            out.push(format!(
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
        while out.len() < 2 + body {
            out.push(String::new());
        }
        out.push(self.gauge_line(t, frame.gauge));

        // status: what is chosen, of what there is; where the cursor is
        // or what a key said
        let (n, bytes, hand) = self.stats();
        let all: u64 = self
            .view
            .iter()
            .map(|&g| self.layout.groups[g].reclaim)
            .sum();
        let mut left = format!(
            " {} {}  {} {} {}",
            t.paint_bold(Paint::Info, &fmt_count(n)),
            t.paint_rgb(DIM, "selected"),
            t.paint(Paint::Plate, &fmt_bytes(bytes)),
            t.paint_rgb(DIM, "of"),
            t.paint_rgb(DIM, &fmt_bytes(all.max(bytes)))
        );
        let mut left_plain = format!(
            " {} selected  {} of {}",
            fmt_count(n),
            fmt_bytes(bytes),
            fmt_bytes(all.max(bytes))
        );
        if hand > 0 {
            left.push_str(&format!(
                "  {}",
                t.paint(Paint::Warn, &format!("{hand} by hand"))
            ));
            left_plain.push_str(&format!("  {hand} by hand"));
        }
        // the rule, the groups shown and their order, the cursor's group
        // (Phase 16): the least telling go first on a narrow line
        let right_plain = match &self.message {
            Some(m) => m.clone(),
            None => {
                let mut parts: Vec<String> = Vec::new();
                if let Some(p) = &self.prefer {
                    parts.push(format!("keeping in {}", if p.is_empty() { "." } else { p }));
                }
                if self.view.len() < self.layout.groups.len() {
                    parts.push(format!(
                        "{} of {} groups",
                        fmt_count(self.view.len() as u64),
                        fmt_count(self.layout.groups.len() as u64)
                    ));
                }
                if self.order != Order::Listing {
                    parts.push(self.order.words().to_string());
                }
                if let Some(gi) = self.cursor_group() {
                    parts.push(format!("group {}/{}", gi + 1, self.layout.groups.len()));
                }
                let room = t.width.saturating_sub(width(&left_plain) + 3);
                crate::util::join_fit(&parts, room)
            }
        };
        let right = if self.message.is_some() {
            t.paint(Paint::Warn, &right_plain)
        } else {
            t.paint(Paint::Dupe, &right_plain)
        };
        let gap = t
            .width
            .saturating_sub(width(&left_plain) + width(&right_plain) + 1);
        if gap >= 2 {
            out.push(format!("{left}{}{right}", " ".repeat(gap)));
        } else {
            out.push(left);
        }

        // keys
        let pairs = |v: &[(&str, &str)]| -> Vec<(String, String)> {
            v.iter()
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .collect()
        };
        let here = match self.rows.get(self.cursor) {
            Some(Row::Head(g)) | Some(Row::Member(g, _)) => Some(*g),
            None => None,
        };
        let mut nav = pairs(&[
            (&format!("{}{}", g.up, g.down), "move"),
            ("space", "select"),
            ("a", "all"),
            ("k", "keeper"),
        ]);
        if here.is_some_and(|g| !self.shots(g).is_empty()) {
            nav.push(("c".into(), "other shots".into()));
        }
        nav.extend(pairs(&[
            ("v", "view"),
            ("o", "reveal"),
            ("g", "go to"),
            ("/", "filter"),
            ("s", "sort"),
            ("f", "keep this folder"),
        ]));
        out.push(fit_parts(t, " ", &nav, t.width));
        let bar = match &self.mode {
            Mode::Browse => {
                let mut v: Vec<(String, String)> = Vec::new();
                if n > 0 {
                    v.push((
                        "m".into(),
                        format!("milk {} {} {}", fmt_count(n), g.arrow, crate::MILKED_DIR),
                    ));
                    v.push((
                        "p".into(),
                        format!("purge {} {} Recycle Bin", fmt_count(n), g.arrow),
                    ));
                    v.push(("n".into(), "preview".into()));
                }
                if self.changes > 0 {
                    v.push(("u".into(), "undo".into()));
                }
                v.push(("q".into(), "quit".into()));
                fit_parts(t, g.pointer, &v, t.width)
            }
            Mode::Confirm(act) => {
                let (verb, dest) = match act {
                    Act::Milk => ("milk", crate::MILKED_DIR),
                    Act::Purge => ("purge", "Recycle Bin"),
                };
                let mut what = format!("{} {} {dest}, {}", fmt_count(n), g.arrow, fmt_bytes(bytes));
                if hand > 0 {
                    what.push_str(&format!(", {hand} by hand"));
                }
                let shots = self.chosen_shots();
                if shots > 0 {
                    what.push_str(&format!(
                        ", {shots} other shot{}",
                        if shots == 1 { "" } else { "s" }
                    ));
                }
                fit_parts(
                    t,
                    g.pointer,
                    &[
                        (verb.to_string(), what),
                        ("enter".into(), "go".into()),
                        ("esc".into(), "back".into()),
                    ],
                    t.width,
                )
            }
            Mode::Filter { typed, .. } => fit_parts(
                t,
                g.pointer,
                &[
                    (
                        "/".into(),
                        format!(
                            "{typed}_  {} of {}",
                            fmt_count(self.view.len() as u64),
                            fmt_count(self.layout.groups.len() as u64)
                        ),
                    ),
                    ("enter".into(), "keep".into()),
                    ("esc".into(), "back".into()),
                ],
                t.width,
            ),
            Mode::GoTo(typed) => fit_parts(
                t,
                g.pointer,
                &[
                    (
                        "go to group".into(),
                        format!("{typed}_  (1-{})", self.layout.groups.len()),
                    ),
                    ("enter".into(), "go".into()),
                    ("esc".into(), "back".into()),
                ],
                t.width,
            ),
        };
        out.push(bar);
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
        for (h, all_open) in [(20usize, true), (12, false)] {
            let st = State::new(layout(), h - 6);
            for w in [40usize, 60, 80, 120, 200] {
                let t = theme(w);
                let screen = st.screen(&t, " dedupe find  root", &listed(&st), h, &Frame::STILL);
                assert_eq!(screen.len(), h, "{screen:#?}");
                for l in &screen {
                    assert!(width(l) <= w, "{w}: {l:?}");
                }
                // the cursor starts on a (1).jpg: its section open, its
                // group unfolded on a tree, the pointer on the file
                assert!(screen[2].starts_with(" COPIES"), "{screen:#?}");
                assert!(
                    screen
                        .iter()
                        .any(|l| l.starts_with("▸■ └ line 0 a (1).jpg")),
                    "{screen:#?}"
                );
                // room for every group: the other kind open too; else
                // folded to its one line
                assert!(
                    screen.iter().any(|l| l.starts_with(" SIMILAR ")),
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
                assert!(
                    screen[screen.len() - 3].contains("2 selected"),
                    "{screen:#?}"
                );
                assert!(screen.last().unwrap().contains("m milk"), "{screen:#?}");
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
        let screen = st.screen(&t, "", &listed(&st), 30, &Frame::STILL);
        let row = screen.iter().find(|l| l.starts_with("▸")).unwrap();
        // the group's line: its keeper, three beside it, what it frees,
        // what its files are
        assert!(
            row.contains("b.png") && row.contains("+3") && row.contains("410 B"),
            "{row}"
        );
        assert!(row.contains("near copy 3"), "{row}");
    }
}
