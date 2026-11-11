//! Undo: the newest journal under a folder, replayed backwards — moves
//! put back, bin entries restored from the Recycle Bin.

use std::fs;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use crate::cli::Invocation;
use crate::interrupt;
use crate::journal::{self, Entry, Journal, Op};
use crate::render::Renderer;
use crate::theme::Role;
use crate::util::{counted, fmt_count, rel_display, shorten_middle, width};
use crate::MILKED_DIR;

use super::{EXIT_FAILED, EXIT_INTERRUPTED, EXIT_OK};

pub(crate) fn undo(inv: &Invocation, r: &Renderer) -> i32 {
    r.header(inv);
    // several changes pending under this folder, and a person at the
    // keyboard: list them and let them choose (QOL-05); otherwise the
    // newest, as always
    let pending: Vec<Journal> = journal::list(&inv.root)
        .map(|v| {
            v.into_iter()
                .filter(|(_, j)| !j.consumed)
                .map(|(_, j)| j)
                .collect()
        })
        .unwrap_or_default();
    if pending.len() > 1 && r.theme.live && std::io::stdin().is_terminal() {
        let root_display = inv
            .root_display
            .clone()
            .unwrap_or_else(|| "this folder".to_string());
        let t = &r.theme;
        let lead = format!(
            "{} changes to reverse under ",
            fmt_count(pending.len() as u64)
        );
        let tail = ", newest first";
        let shown_root = shorten_middle(
            &root_display,
            t.width
                .saturating_sub(width(&lead) + tail.len() + 2)
                .max(12),
            t.glyphs.ellipsis,
        );
        r.note(Role::Plate, &format!("{lead}{shown_root}{tail}"));
        r.blank();
        let digits = pending.len().to_string().len();
        for (i, j) in pending.iter().enumerate() {
            let when = crate::util::fmt_stamp_local(&j.created_utc);
            let n_entries = counted(j.entries.len() as u64, "entry", "entries");
            // the command is the one thing that can be long: its middle goes
            let room = t
                .width
                .saturating_sub(3 + digits + 2 + width(&when) + 2 + 3 + width(&n_entries) + 1)
                .max(16);
            let cmd = shorten_middle(&j.command, room, t.glyphs.ellipsis);
            r.line(&format!(
                "   {}  {}  {}   {}",
                t.paint_bold(Role::Keep, &format!("{:>digits$}", i + 1)),
                t.paint(Role::Dupe, &when),
                t.paint(Role::Plate, &cmd),
                t.paint(Role::Dupe, &n_entries),
            ));
        }
        r.blank();
        let Some(n) = crate::workspace::pick_number(
            r,
            &format!("1-{}  enter the newest  esc leave", pending.len()),
            pending.len(),
        ) else {
            return EXIT_OK;
        };
        r.blank();
        return act_undo_which(inv, r, Some(&pending[n - 1].id));
    }
    act_undo(inv, r)
}

/// Reverse the latest pending journal under the root (the workspace's `u`
/// and the `--undo` command after its header).
pub(crate) fn act_undo(inv: &Invocation, r: &Renderer) -> i32 {
    act_undo_which(inv, r, None)
}

/// Reverse the pending journal `id` under the root, or the newest when
/// `id` is None (the workspace passes the journal its own change wrote).
pub(crate) fn act_undo_which(inv: &Invocation, r: &Renderer, id: Option<&str>) -> i32 {
    let root_display = inv
        .root_display
        .clone()
        .unwrap_or_else(|| "this folder".to_string());
    let found = match id {
        None => journal::latest_pending(&inv.root),
        Some(id) => {
            journal::list(&inv.root).map(|v| v.into_iter().find(|(_, j)| !j.consumed && j.id == id))
        }
    };
    let (jpath, mut j) = match found {
        Ok(Some(x)) => x,
        Ok(None) => {
            r.note(Role::Dupe, &format!("nothing to undo under {root_display}"));
            // a journal under another root is the likely intent
            for (root, command, n) in journal::pending_elsewhere(&inv.root).into_iter().take(3) {
                let root_s = root.to_string_lossy();
                let cmd = if root_s.contains(' ') {
                    format!("dedupe --undo \"{root_s}\"")
                } else {
                    format!("dedupe --undo {root_s}")
                };
                r.hint(
                    "undo",
                    &cmd,
                    &format!("{command}   {}", counted(n as u64, "entry", "entries")),
                );
            }
            return EXIT_OK;
        }
        Err(e) => {
            r.error(&format!("journal unreadable  {e}"));
            return EXIT_FAILED;
        }
    };
    // what the journal recorded, on one line: a long root shortens the
    // command in the middle (it describes the change; the line to copy is
    // the `undo` hint, which stays whole)
    let lead = format!("journal {}   ", j.id);
    let tail = format!("   {}", counted(j.entries.len() as u64, "entry", "entries"));
    let room = r
        .theme
        .width
        .saturating_sub(1 + width(&lead) + width(&tail))
        .max(16);
    r.note(
        Role::Plate,
        &format!(
            "{lead}{}{tail}",
            shorten_middle(&j.command, room, r.theme.glyphs.ellipsis)
        ),
    );
    r.blank();

    let mut restored: u64 = 0;
    let mut bytes: u64 = 0;
    let mut failed: u64 = 0;
    let mut stopped = false;
    // entries not restored because the run was interrupted stay in the
    // journal, which stays pending, so the next --undo continues
    let mut remaining: Vec<Entry> = Vec::new();

    if !inv.dry_run {
        interrupt::install();
        interrupt::changing(true);
    }

    // moves, newest first
    for e in j.entries.iter().rev() {
        if let Op::Move { from, to, .. } = &e.op {
            if stopped {
                remaining.push(e.clone());
                continue;
            }
            if !inv.dry_run && interrupt::requested() {
                stopped = true;
                remaining.push(e.clone());
                continue;
            }
            let rel = rel_display(from, &inv.root);
            if inv.dry_run {
                r.action("would", Role::Warn, &rel, "restore");
                restored += 1;
                bytes += e.size;
                continue;
            }
            if !to.exists() {
                r.action(
                    "missing",
                    Role::Danger,
                    &rel_display(to, &inv.root),
                    "not in _milked any more",
                );
                failed += 1;
                continue;
            }
            if from.exists() {
                r.action("exists", Role::Warn, &rel, "left in _milked");
                failed += 1;
                continue;
            }
            match crate::fsops::move_file(to, from) {
                Ok(copied) => {
                    let detail = if copied {
                        "copied back across volumes"
                    } else {
                        ""
                    };
                    r.action("restored", Role::Ok, &rel, detail);
                    restored += 1;
                    bytes += e.size;
                }
                Err(err) => {
                    r.action("failed", Role::Danger, &rel, &err.words());
                    failed += 1;
                }
            }
        }
    }

    // bin restores, one listing of the bin for all of them
    let binned: Vec<(&PathBuf, i64, u64)> = j
        .entries
        .iter()
        .filter_map(|e| match &e.op {
            Op::Trash { path, time_deleted } => Some((path, *time_deleted, e.size)),
            _ => None,
        })
        .collect();
    if !binned.is_empty() {
        if inv.dry_run {
            for (p, _, sz) in &binned {
                r.action(
                    "would",
                    Role::Warn,
                    &rel_display(p, &inv.root),
                    "restore from Recycle Bin",
                );
                restored += 1;
                bytes += sz;
            }
        } else if stopped || interrupt::requested() {
            // the bin restore is one call; it is not started once Ctrl-C arrived
            stopped = true;
            remaining.extend(
                j.entries
                    .iter()
                    .filter(|e| matches!(e.op, Op::Trash { .. }))
                    .cloned(),
            );
        } else {
            for (p, ok, why, sz) in restore_from_bin(&binned) {
                let rel = rel_display(&p, &inv.root);
                if ok {
                    r.action("restored", Role::Ok, &rel, "from Recycle Bin");
                    restored += 1;
                    bytes += sz;
                } else {
                    r.action("failed", Role::Danger, &rel, &why);
                    failed += 1;
                }
            }
        }
    }
    interrupt::changing(false);

    if !inv.dry_run {
        let res = if stopped {
            journal::keep_remaining(&jpath, &j, remaining.clone())
        } else {
            journal::mark_consumed(&jpath, &mut j)
        };
        if let Err(e) = res {
            r.note(Role::Danger, &format!("journal could not be updated: {e}"));
        }
        prune_empty_dirs(&inv.root.join(MILKED_DIR));
    }

    let verb = if inv.dry_run {
        "would restore"
    } else {
        "restored"
    };
    r.action_totals(verb, restored, bytes, "back in place", None, Some(&j.id));
    if failed > 0 {
        r.note(
            Role::Danger,
            &format!("{} could not be restored", fmt_count(failed)),
        );
    }
    if stopped {
        r.note(
            Role::Danger,
            &format!(
                "interrupted; {} still to restore, the journal keeps them: dedupe --undo continues",
                fmt_count(remaining.len() as u64)
            ),
        );
        return EXIT_INTERRUPTED;
    }
    if failed > 0 {
        EXIT_FAILED
    } else {
        EXIT_OK
    }
}

/// Restore journaled bin entries. Returns (path, ok, reason, size) per entry.
#[cfg(any(windows, all(unix, not(target_os = "macos"))))]
fn restore_from_bin(entries: &[(&PathBuf, i64, u64)]) -> Vec<(PathBuf, bool, String, u64)> {
    use trash::os_limited;
    let mut out = Vec::new();
    let items = match os_limited::list() {
        Ok(v) => v,
        Err(e) => {
            return entries
                .iter()
                .map(|(p, _, sz)| ((*p).clone(), false, format!("bin unreadable: {e}"), *sz))
                .collect();
        }
    };
    let mut to_restore = Vec::new();
    let mut idx_of: Vec<Option<usize>> = Vec::new();
    for (p, t, _) in entries {
        let found = items.iter().position(|it| {
            let orig = it.original_parent.join(&it.name);
            &orig == *p && it.time_deleted >= *t - 2
        });
        idx_of.push(found);
        if let Some(i) = found {
            to_restore.push(items[i].clone());
        }
    }
    let restore_result = if to_restore.is_empty() {
        Ok(())
    } else {
        os_limited::restore_all(to_restore)
    };
    for (k, (p, _, sz)) in entries.iter().enumerate() {
        match idx_of[k] {
            None => out.push((
                (*p).clone(),
                false,
                "not in the Recycle Bin".to_string(),
                *sz,
            )),
            Some(_) => match &restore_result {
                Ok(()) => out.push((
                    (*p).clone(),
                    p.exists(),
                    if p.exists() {
                        String::new()
                    } else {
                        "restore reported ok but file absent".into()
                    },
                    *sz,
                )),
                Err(e) => out.push(((*p).clone(), p.exists(), e.to_string(), *sz)),
            },
        }
    }
    out
}

#[cfg(not(any(windows, all(unix, not(target_os = "macos")))))]
fn restore_from_bin(entries: &[(&PathBuf, i64, u64)]) -> Vec<(PathBuf, bool, String, u64)> {
    entries
        .iter()
        .map(|(p, _, sz)| {
            (
                (*p).clone(),
                false,
                "restoring from the bin is not supported on this OS; use Finder".to_string(),
                *sz,
            )
        })
        .collect()
}

/// Remove empty directories bottom-up under `dir`, including `dir` itself.
fn prune_empty_dirs(dir: &Path) {
    if !dir.is_dir() {
        return;
    }
    if let Ok(rd) = fs::read_dir(dir) {
        for e in rd.flatten() {
            if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                prune_empty_dirs(&e.path());
            }
        }
    }
    let _ = fs::remove_dir(dir); // fails harmlessly when not empty
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prune_removes_only_empty() {
        let d = tempfile::tempdir().unwrap();
        let m = d.path().join("_milked");
        fs::create_dir_all(m.join("a/b")).unwrap();
        fs::create_dir_all(m.join("c")).unwrap();
        fs::write(m.join("c/keep.txt"), b"x").unwrap();
        prune_empty_dirs(&m);
        assert!(!m.join("a").exists());
        assert!(m.join("c/keep.txt").exists());
        assert!(m.exists());
    }
}
