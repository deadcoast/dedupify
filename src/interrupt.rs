//! Ctrl-C during a change (docs/CLI.md §5).
//!
//! Outside a change, Ctrl-C ends the process as it always did (exit 130).
//! Once `milk`, `purge` or `--undo` starts moving files, the first Ctrl-C
//! only raises a flag: the loop finishes the file in hand, stops, and
//! writes the journal for what it did, so `--undo` still covers every
//! move. A second Ctrl-C ends the process at once.

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::OnceLock;

static INSTALLED: OnceLock<bool> = OnceLock::new();
static CHANGING: AtomicBool = AtomicBool::new(false);
static PRESSES: AtomicU8 = AtomicU8::new(0);

/// Exit status for an interrupted run (the shell convention, 128 + SIGINT).
pub const EXIT_INTERRUPTED: i32 = 130;

/// Install the handler once. Safe to call repeatedly.
pub fn install() -> bool {
    *INSTALLED.get_or_init(|| {
        ctrlc::set_handler(|| {
            let n = PRESSES.fetch_add(1, Ordering::SeqCst) + 1;
            if !CHANGING.load(Ordering::SeqCst) || n >= 2 {
                // nothing to protect, or the user insists; a live fold
                // line gives the cursor back first
                crate::render::fold::abandon();
                let mut err = std::io::stderr().lock();
                let _ = std::io::Write::write_all(&mut err, b"\n");
                std::process::exit(EXIT_INTERRUPTED);
            }
        })
        .is_ok()
    })
}

/// Mark the start / end of the section whose work a journal must cover.
pub fn changing(on: bool) {
    CHANGING.store(on, Ordering::SeqCst);
}

/// True once Ctrl-C was pressed during a change.
pub fn requested() -> bool {
    PRESSES.load(Ordering::SeqCst) > 0
}
