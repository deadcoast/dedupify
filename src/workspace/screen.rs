//! The workspace's screen, composed from its state and the listing lines
//! the shell captured, so a test can check what a person sees without a
//! pseudo-terminal (split from `state.rs`, Phase 16, SC-12).

use super::state::*;
use crate::theme::{Role as Paint, Theme};
use crate::util::{fmt_bytes, fmt_count, width};

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

impl State {
    /// The full screen, exactly `total_height` lines: the title and a rule,
    /// the visible rows (marks and cursor before the captured listing
    /// lines, one per row), a rule, the status line, the keys.
    pub fn screen(
        &self,
        t: &Theme,
        title: &str,
        lines: &[String],
        total_height: usize,
    ) -> Vec<String> {
        let g = t.glyphs;
        let mut out: Vec<String> = Vec::with_capacity(total_height);
        out.push(title.to_string());
        out.push(t.rule());
        let body = total_height.saturating_sub(6);
        for i in self.top..(self.top + body).min(self.rows.len()) {
            let row = self.rows[i];
            let cur = if i == self.cursor { g.pointer } else { " " };
            let (mark, role) = match self.mark(row) {
                Mark::None => (" ", Paint::Dupe),
                Mark::On => (g.sel_on, Paint::Keep),
                Mark::Off => (g.sel_off, Paint::Dupe),
                Mark::Part => (g.sel_part, Paint::Keep),
                Mark::Lock => (g.sel_lock, Paint::Dupe),
            };
            let text = lines.get(i).map(String::as_str).unwrap_or("");
            let text = match row {
                Row::Member(_, _) => text.strip_prefix("   ").unwrap_or(text),
                Row::Head(_) => text.strip_prefix(' ').unwrap_or(text),
            };
            out.push(format!(
                "{}{} {}",
                t.paint_bold(Paint::Keep, cur),
                t.paint_bold(role, mark),
                text
            ));
        }
        while out.len() < 2 + body {
            out.push(String::new());
        }
        out.push(t.rule());

        // status: what is chosen, and where the cursor is or what a key said
        let (n, bytes, hand) = self.stats();
        let mut left = format!(
            " {} selected  {}",
            t.paint_bold(Paint::Info, &fmt_count(n)),
            t.paint(Paint::Plate, &fmt_bytes(bytes))
        );
        let mut left_plain = format!(" {} selected  {}", fmt_count(n), fmt_bytes(bytes));
        if hand > 0 {
            left.push_str(&format!(
                "  {}",
                t.paint(Paint::Warn, &format!("{hand} by hand"))
            ));
            left_plain.push_str(&format!("  {hand} by hand"));
        }
        let right_plain = match &self.message {
            Some(m) => m.clone(),
            None => match self.cursor_group() {
                Some(gi) => format!("group {}/{}", gi + 1, self.layout.groups.len()),
                None => String::new(),
            },
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
        nav.extend(pairs(&[("v", "view"), ("o", "reveal"), ("g", "go to")]));
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

    #[test]
    fn the_screen_is_exactly_the_terminal_and_fits_its_width() {
        let st = State::new(layout(), 20 - 6);
        let lines: Vec<String> = st
            .rows
            .iter()
            .map(|r| match r {
                Row::Head(g) => format!(" GROUP {:03}", g + 1),
                Row::Member(g, m) => {
                    format!(
                        "   line {} {}",
                        g,
                        st.layout.groups[*g].members[*m].path.display()
                    )
                }
            })
            .collect();
        for w in [60usize, 80, 120] {
            let t = theme(w);
            let screen = st.screen(&t, " dedupe find  root", &lines, 20);
            assert_eq!(screen.len(), 20, "{screen:#?}");
            for l in &screen {
                assert!(width(l) <= w, "{w}: {l:?}");
            }
            // the cursor row: pointer, mark, the member line
            assert!(
                screen.iter().any(|l| l.starts_with("▸■ line 0 a (1).jpg")),
                "{screen:#?}"
            );
            assert!(
                screen.iter().any(|l| l.starts_with(" ◧ GROUP 002")),
                "{screen:#?}"
            );
            assert!(
                screen.iter().any(|l| l.starts_with(" · line 1 b_next.jpg")),
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
