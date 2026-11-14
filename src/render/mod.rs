//! Output (docs/CLI.md §3). Every line starts with one space of margin, is
//! measured in display cells and fitted to the terminal width.
//!
//! Split by screen (Phase 12, SC-03): `stages` while a scan runs, `groups`
//! for the listing, `totals` under it and after an action, `hints` for
//! what to run next. Here: the renderer itself and the lines every screen
//! uses.

use std::io::{self, Write};
use std::time::Duration;

use crate::cli::{Command, Invocation};
use crate::theme::{Role, Theme};
use crate::util::{fmt_bytes, shorten_middle, visible_width, width, wrap_words};

pub(crate) mod fold;
mod groups;
mod hints;
mod stages;
mod totals;

pub use stages::LiveProgress;
#[cfg(test)]
pub(crate) use stages::{sample_stage_texts, stage_line};

pub const BAR_CELLS: usize = 30;

const REDRAW_EVERY: Duration = Duration::from_millis(40);

/// The verb column of the `next` / `near` / `video` / `undo` lines.
const HINT_VERB: usize = 5;

/// A group row before its tail: margin, indent, verb, gap; then the gap
/// after the path. The tail starts `ROW_LEAD + path` cells in.
const ROW_LEAD: usize = 1 + 2 + 4 + 2 + 2;

/// An exact keeper row after its path: the date and one cell of margin.
const EXACT_POST: usize = 10 + 1;

pub struct Renderer {
    pub theme: Theme,
    /// When set, lines go here instead of to stdout (the workspace
    /// captures the listing to show it in its own screen).
    sink: Option<std::cell::RefCell<Vec<String>>>,
    /// The action in the fold: one live line for every file it takes.
    act: std::cell::RefCell<Option<ActFold>>,
}

/// An action folded into one line: its counts, and what it could not do.
struct ActFold {
    hue: fold::Hue,
    total: u64,
    done: u64,
    phase: &'static str,
    failed: Vec<(String, String)>,
    left: u64,
}

impl Renderer {
    pub fn new(theme: Theme) -> Renderer {
        Renderer {
            theme,
            sink: None,
            act: std::cell::RefCell::new(None),
        }
    }

    /// Run `f` against a renderer `width` cells wide whose lines are
    /// collected instead of printed — the full listing, as `--verbose`
    /// prints it (the workspace shows it in its own screen).
    pub fn capture(&self, width: usize, f: impl FnOnce(&Renderer)) -> Vec<String> {
        let mut theme = self.theme;
        theme.width = width;
        theme.fold = false;
        let r = Renderer {
            theme,
            sink: Some(std::cell::RefCell::new(Vec::new())),
            act: std::cell::RefCell::new(None),
        };
        f(&r);
        r.sink.map(|s| s.into_inner()).unwrap_or_default()
    }

    /// The fold is drawing (not the full listing, not a capture).
    pub(crate) fn folding(&self) -> bool {
        self.theme.fold && self.sink.is_none()
    }

    pub fn line(&self, s: &str) {
        if let Some(sink) = &self.sink {
            sink.borrow_mut().push(s.to_string());
            return;
        }
        let _o = fold::OUT.lock().unwrap_or_else(|e| e.into_inner());
        let mut out = io::stdout().lock();
        fold::clear_for_write(&mut out);
        let _ = writeln!(out, "{s}");
    }

    pub fn blank(&self) {
        self.line("");
    }

    pub fn rule(&self) {
        self.line(&self.theme.rule());
    }

    pub fn header(&self, inv: &Invocation) {
        // the doctor's page keeps its own frame: a rule under the header
        // and one over its verdict
        let folding = self.folding() && inv.command != Command::Doctor;
        self.line(&self.header_text(inv, folding, self.theme.width));
        if folding {
            self.blank();
        } else {
            self.rule();
        }
    }

    /// The header line `width` cells wide: the fold's (the name on its
    /// plate, the parts joined by `·`) or the full output's. The workspace
    /// draws the fold's at the top of its screen.
    pub(crate) fn header_text(&self, inv: &Invocation, folding: bool, width: usize) -> String {
        let mut theme = self.theme;
        theme.width = width;
        let t = &theme;
        // everything but the root first, so the root gets what is left;
        // the fold's header carries the name on the brand's amber plate
        let lead = if folding && t.color {
            format!(
                "\x1b[1;38;2;24;22;18;48;2;255;176;0m dedupe \x1b[0m {}",
                t.paint_bold(Role::Keep, inv.command.word().trim_start_matches('-'))
            )
        } else if folding {
            format!(
                "dedupe  {}",
                t.paint_bold(Role::Keep, inv.command.word().trim_start_matches('-'))
            )
        } else {
            format!(
                "{} {}",
                t.paint_bold(Role::Plate, "dedupe"),
                t.paint_bold(Role::Keep, inv.command.word())
            )
        };
        let mut tail: Vec<String> = Vec::new();
        if matches!(inv.command, Command::Find | Command::Milk | Command::Purge) {
            tail.push(t.paint(
                Role::Dupe,
                if inv.recurse {
                    "recursive"
                } else {
                    "this folder"
                },
            ));
            tail.push(t.paint(Role::Dupe, inv.kind.label()));
            if inv.quick {
                tail.push(t.paint(Role::Warn, "quick: size + sample only"));
            }
            if inv.min > 0 {
                tail.push(t.paint(Role::Dupe, &format!("min {}", fmt_bytes(inv.min))));
            }
            if !inv.skip.is_empty() {
                tail.push(t.paint(Role::Dupe, &format!("skip {}", inv.skip.join(", "))));
            }
        }
        if inv.dry_run && matches!(inv.command, Command::Milk | Command::Purge | Command::Undo) {
            tail.push(t.paint_bold(Role::Warn, "dry run"));
        }
        // on a narrow line the root keeps at least 16 cells: the parts
        // that say least go first (the kind, then the depth; the dry run
        // and the quick search stay), and the root takes what is left —
        // the floor of 24 ran the header past a 40-cell edge (Phase 16)
        let join = if folding { 3 } else { 2 };
        let used = |tail: &[String]| {
            1 + visible_width(&lead)
                + 2
                + tail.iter().map(|p| visible_width(p) + join).sum::<usize>()
        };
        let mut drop_order: Vec<usize> = (0..tail.len()).rev().collect();
        drop_order.sort_by_key(|&i| {
            let w = crate::util::strip_sgr(&tail[i]);
            (
                w == "dry run" || w.starts_with("quick"),
                std::cmp::Reverse(i),
            )
        });
        let mut dropped: Vec<usize> = Vec::new();
        for &i in &drop_order {
            let kept: Vec<String> = tail
                .iter()
                .enumerate()
                .filter(|(j, _)| !dropped.contains(j))
                .map(|(_, p)| p.clone())
                .collect();
            if t.width.saturating_sub(used(&kept) + 1) >= 16 {
                break;
            }
            dropped.push(i);
        }
        let tail: Vec<String> = tail
            .into_iter()
            .enumerate()
            .filter(|(j, _)| !dropped.contains(j))
            .map(|(_, p)| p)
            .collect();
        let root = inv.root.to_string_lossy();
        let room = t.width.saturating_sub(used(&tail) + 1);
        let root = shorten_middle(&root, room.max(8), t.glyphs.ellipsis);
        if folding {
            let dot = t.paint(Role::Rule, if t.unicode { " · " } else { "   " });
            let mut line = format!(" {lead}  {}", t.paint(Role::Info, &root));
            if !tail.is_empty() {
                line.push_str("   ");
                line.push_str(&tail.join(&dot));
            }
            return line;
        }
        let mut parts = vec![lead, t.paint(Role::Info, &root)];
        parts.extend(tail);
        format!(" {}", parts.join("  "))
    }

    /// A footer note. Longer than the width, it breaks at spaces and goes
    /// on indented under itself, as a totals line does.
    pub fn note(&self, role: Role, s: &str) {
        let max = self.theme.width.saturating_sub(1);
        let lines = wrap_words(s, max.saturating_sub(1), max.saturating_sub(3));
        for (i, l) in lines.iter().enumerate() {
            let lead = if i == 0 { " " } else { "   " };
            self.line(&format!("{lead}{}", self.theme.paint(role, l)));
        }
    }

    /// A note whose second part may drop to an indented line when the width
    /// is short; `detail` is shortened in the middle if even that fails.
    pub fn note_fit(&self, role: Role, lead: &str, detail: &str) {
        let t = &self.theme;
        let max = t.width.saturating_sub(1);
        let one = format!("{lead}   {detail}");
        if width(&one) < max {
            self.line(&format!(" {}", t.paint(role, &one)));
        } else {
            self.line(&format!(" {}", t.paint(role, lead)));
            self.line(&format!(
                "   {}",
                t.paint(
                    Role::Dupe,
                    &shorten_middle(detail, max.saturating_sub(3), t.glyphs.ellipsis)
                )
            ));
        }
    }

    pub fn error(&self, msg: &str) {
        let t = &self.theme;
        let mut out = io::stderr().lock();
        let _ = writeln!(
            out,
            " {}  {}",
            t.paint_bold(Role::Danger, "dedupe"),
            t.paint(Role::Plate, msg)
        );
        let _ = writeln!(out, "         {}", t.paint(Role::Dupe, "dedupe --help"));
    }
}
