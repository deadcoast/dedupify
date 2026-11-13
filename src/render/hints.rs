//! The `next` lines: the command to run after a find, a quick search or
//! an action.

use crate::cli::{Command, Invocation};
use crate::theme::Role;
use crate::util::{counted, fit, under_root, width};
use crate::MILKED_DIR;

use super::{Renderer, HINT_VERB};

impl Renderer {
    /// A `verb   command   note` line: the verb in a fixed column, the
    /// command always whole (it is there to be copied), the note whole or
    /// not at all — a note cut to its first and last letters says nothing.
    pub fn hint(&self, verb: &str, command: &str, note: &str) {
        let t = &self.theme;
        let lead = 1 + HINT_VERB + 2;
        let room = t.width.saturating_sub(lead + width(command) + 3 + 1);
        let note = if width(note) <= room { note } else { "" };
        self.line(&format!(
            " {}  {}   {}",
            t.paint_bold(Role::Keep, &fit(verb, HINT_VERB)),
            t.paint(Role::Plate, command),
            t.paint(Role::Dupe, note)
        ));
    }

    /// After a quick search that saw images or videos: the full form of
    /// the same command, which matches near duplicates too.
    pub fn full_hint(&self, inv: &Invocation, images: u64, videos: u64) {
        let mut i = inv.clone();
        i.quick = false;
        let mut seen: Vec<String> = Vec::new();
        if images >= 2 {
            seen.push(counted(images, "image", "images"));
        }
        if videos >= 2 {
            seen.push(counted(videos, "video", "videos"));
        }
        self.hint(
            "full",
            &i.command_line_as(Command::Find),
            &format!(
                "{} seen; the full run matches near duplicates",
                seen.join(", ")
            ),
        );
    }

    /// The follow-up commands after `find`, in their full form (a quick
    /// search is for looking; moving and binning run on full evidence).
    pub fn next_after_find(&self, inv: &Invocation) {
        let t = &self.theme;
        let mut full = inv.clone();
        full.quick = false;
        let milk = full.command_line_as(Command::Milk);
        let purge = full.command_line_as(Command::Purge);
        // the fold adds the way to every group and file, in the same column
        let every = format!("{} -v", inv.command_line_as(Command::Find));
        let w = width(&milk)
            .max(width(&purge))
            .max(if self.folding() { width(&every) } else { 0 })
            + 2;
        let lead = 1 + HINT_VERB + 2 + w + 1;
        let room = t.width.saturating_sub(lead);
        // notes whole or not at all: the commands are the payload and a
        // note cut to its ends says nothing. The milk note names the full
        // destination when there is room, the bare folder name otherwise.
        let dest = under_root(inv.root_display.as_deref(), MILKED_DIR);
        let long_note = format!("move duplicates into {dest}");
        let short_note = format!("move duplicates into {MILKED_DIR}");
        let milk_note = if width(&long_note) <= room {
            long_note.as_str()
        } else if width(&short_note) <= room {
            short_note.as_str()
        } else {
            ""
        };
        let purge_note = "send duplicates to the Recycle Bin";
        let purge_note = if width(purge_note) <= room {
            purge_note
        } else {
            ""
        };
        self.line(&format!(
            " {}  {}{}",
            t.paint_bold(Role::Keep, &fit("next", HINT_VERB)),
            t.paint(Role::Plate, &fit(&milk, w)),
            t.paint(Role::Dupe, milk_note)
        ));
        self.line(&format!(
            " {}  {}{}",
            " ".repeat(HINT_VERB),
            t.paint(Role::Plate, &fit(&purge, w)),
            t.paint(Role::Dupe, purge_note)
        ));
        // the fold names the way to every group and file
        if self.folding() {
            let note = "every group and file";
            let note = if width(note) <= room { note } else { "" };
            self.line(&format!(
                " {}  {}{}",
                " ".repeat(HINT_VERB),
                t.paint(Role::Plate, &fit(&every, w)),
                t.paint(Role::Dupe, note)
            ));
        }
    }

    pub fn undo_hint(&self, inv: &Invocation) {
        let mut cmd = String::from("dedupe --undo");
        if let Some(r) = &inv.root_display {
            cmd.push(' ');
            cmd.push_str(&if r.contains(' ') {
                format!("\"{r}\"")
            } else {
                r.clone()
            });
        }
        self.hint("undo", &cmd, "reverses this change");
    }
}
