//! The commands: find, milk, purge, undo. Each one runs the shared
//! scan → exact groups → [near groups] pipeline, then acts, then journals.
//!
//! Split by stage (Phase 12, SC-02): `scan` builds what was found, `plan`
//! decides what an action may take, `act` milks and purges, `undo` puts
//! it back. The commands here compose them.

use std::io::IsTerminal;

use crate::cli::{Command, Invocation};
use crate::interrupt;
use crate::render::Renderer;

mod act;
mod plan;
mod scan;
mod undo;
mod view;
#[cfg(test)]
mod widths;

pub(crate) use act::{act_milk, act_purge, Acted};
pub(crate) use plan::{default_items, items, shot_items, verify_items, Item};
pub(crate) use scan::{report, scan_and_group, Found};
pub(crate) use undo::{act_undo_which, undo};

pub const EXIT_OK: i32 = 0;

pub const EXIT_USER: i32 = 1;

pub const EXIT_FAILED: i32 = 2;

/// Ctrl-C during a change: what was done is journaled and reported.
pub const EXIT_INTERRUPTED: i32 = interrupt::EXIT_INTERRUPTED;

/// Dispatch. Returns the process exit code.
pub fn run(inv: &Invocation, r: &Renderer) -> i32 {
    if matches!(inv.command, Command::Find | Command::Milk | Command::Purge) {
        crate::recent::record(inv);
    }
    match inv.command {
        Command::Find => find(inv, r),
        Command::Milk => milk(inv, r),
        Command::Purge => purge(inv, r),
        Command::Undo => undo(inv, r),
        Command::Doctor | Command::Help | Command::Menu => EXIT_OK,
    }
}

fn find(inv: &Invocation, r: &Renderer) -> i32 {
    r.header(inv);
    let mut f = scan_and_group(inv, r);
    report(&f, inv, r);
    if inv.quick && (f.images_seen >= 2 || f.videos_seen >= 2) {
        r.full_hint(inv, f.images_seen, f.videos_seen);
    }
    if f.empty() {
        return EXIT_OK;
    }
    if r.theme.live && std::io::stdin().is_terminal() {
        // the result is the workspace: act on what is on screen
        crate::workspace::run(&mut f, inv, r)
    } else {
        r.next_after_find(inv);
        EXIT_OK
    }
}

fn milk(inv: &Invocation, r: &Renderer) -> i32 {
    r.header(inv);
    let f = scan_and_group(inv, r);
    report(&f, inv, r);
    if f.empty() {
        return EXIT_OK;
    }
    r.blank();
    let list = default_items(&f);
    act_milk(&f, &list, inv, r, inv.dry_run).code
}

fn purge(inv: &Invocation, r: &Renderer) -> i32 {
    r.header(inv);
    let f = scan_and_group(inv, r);
    let list = default_items(&f);
    let verified = verify_items(&list, &f.pipe, r);
    report(&f, inv, r);
    if f.empty() {
        return EXIT_OK;
    }
    r.blank();
    act_purge(&f, &list, &verified, inv, r, inv.dry_run).code
}

pub fn os_err(e: &std::io::Error) -> String {
    let s = e.to_string();
    // strip the "(os error N)" suffix std appends
    match s.find(" (os error") {
        Some(i) => s[..i].to_string(),
        None => s,
    }
}
