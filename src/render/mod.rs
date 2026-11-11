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

mod groups;
mod hints;
mod stages;
mod totals;

pub use stages::LiveProgress;

pub const BAR_CELLS: usize = 30;

const REDRAW_EVERY: Duration = Duration::from_millis(40);

/// Cells a stage line spends before its text: margin, label, space, bar, gap.
const STAGE_LEAD: usize = 1 + 6 + 1 + BAR_CELLS + 2;

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
}

impl Renderer {
    pub fn new(theme: Theme) -> Renderer {
        Renderer { theme, sink: None }
    }

    /// Run `f` against a renderer `width` cells wide whose lines are
    /// collected instead of printed.
    pub fn capture(&self, width: usize, f: impl FnOnce(&Renderer)) -> Vec<String> {
        let mut theme = self.theme;
        theme.width = width;
        let r = Renderer {
            theme,
            sink: Some(std::cell::RefCell::new(Vec::new())),
        };
        f(&r);
        r.sink.map(|s| s.into_inner()).unwrap_or_default()
    }

    pub fn line(&self, s: &str) {
        if let Some(sink) = &self.sink {
            sink.borrow_mut().push(s.to_string());
            return;
        }
        let mut out = io::stdout().lock();
        let _ = writeln!(out, "{s}");
    }

    pub fn blank(&self) {
        self.line("");
    }

    pub fn rule(&self) {
        self.line(&self.theme.rule());
    }

    pub fn header(&self, inv: &Invocation) {
        let t = &self.theme;
        // everything but the root first, so the root gets what is left
        let lead = format!(
            "{} {}",
            t.paint_bold(Role::Plate, "dedupe"),
            t.paint_bold(Role::Keep, inv.command.word())
        );
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
        let used =
            1 + visible_width(&lead) + 2 + tail.iter().map(|p| visible_width(p) + 2).sum::<usize>();
        let root = inv.root.to_string_lossy();
        let root = shorten_middle(
            &root,
            t.width.saturating_sub(used + 1).max(24),
            t.glyphs.ellipsis,
        );
        let mut parts = vec![lead, t.paint(Role::Info, &root)];
        parts.extend(tail);
        self.line(&format!(" {}", parts.join("  ")));
        self.rule();
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
