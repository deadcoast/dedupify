//! Acting: `milk` moves the planned files into `_milked`, `purge` sends
//! them to the Recycle Bin, each under a journal; the footers say what
//! was taken and why.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::cli::{Command, Invocation};
use crate::interrupt;
use crate::journal::{Entry, Op, Writer};
use crate::render::fold::Hue;
use crate::render::Renderer;
use crate::theme::Role;
use crate::util::{fmt_count, rel_display, under_root, unix_secs};
use crate::MILKED_DIR;

use super::{Found, Item, EXIT_FAILED, EXIT_INTERRUPTED, EXIT_OK};

/// `dst` with ` (2)`, ` (3)` … inserted before the extension until free.
fn unique_path(dst: &Path) -> PathBuf {
    if !dst.exists() {
        return dst.to_path_buf();
    }
    let stem = dst
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = dst.extension().map(|e| format!(".{}", e.to_string_lossy()));
    let parent = dst.parent().map(Path::to_path_buf).unwrap_or_default();
    let mut n = 2;
    loop {
        let name = format!("{stem} ({n}){}", ext.clone().unwrap_or_default());
        let p = parent.join(name);
        if !p.exists() {
            return p;
        }
        n += 1;
    }
}

/// Close the journal of a change. `None` when nothing was recorded or the
/// journal could not be written (said once, in the danger colour).
fn close_journal(w: Writer, r: &Renderer, where_are_they: &str) -> Option<String> {
    match w.finish() {
        Ok(id) => id,
        Err(e) => {
            r.note(
                Role::Danger,
                &format!(
                    "journal not written: {e}   ({where_are_they}; dedupe --undo unavailable)"
                ),
            );
            None
        }
    }
}

/// After an interrupted change: what stopped, what the journal covers.
fn interrupted_note(r: &Renderer, done: u64, verb: &str, journaled: bool) {
    r.note(
        Role::Danger,
        &format!(
            "interrupted after {} {verb}{}",
            fmt_count(done),
            if journaled {
                "; the journal covers them, dedupe --undo reverses them"
            } else {
                ""
            }
        ),
    );
}

/// What an action did: its exit code, its journal when it wrote one, and
/// every file it could not take with the reason (SC-07), so the workspace
/// can offer a retry or a fallback by kind.
pub(crate) struct Acted {
    pub(crate) code: i32,
    pub(crate) journal: Option<String>,
    pub(crate) failed: Vec<(PathBuf, crate::fsops::ActError)>,
}

/// Move every actionable item of `f` into `_milked` (or say what would
/// move). A move across volumes is a verified copy (`fsops`).
pub(crate) fn act_milk(
    f: &Found,
    list: &[Item<'_>],
    inv: &Invocation,
    r: &Renderer,
    dry_run: bool,
) -> Acted {
    let milked = inv.root.join(MILKED_DIR);
    let milked_display = under_root(inv.root_display.as_deref(), MILKED_DIR);
    let mut w = Writer::new(&inv.root, &inv.command_line_as(Command::Milk));
    let mut moved: u64 = 0;
    let mut bytes: u64 = 0;
    let mut failed: u64 = 0;
    let mut stopped = false;

    if !dry_run {
        interrupt::install();
        interrupt::changing(true);
    }
    let mut kept: Vec<(String, String, bool)> = Vec::new();
    let mut by_hand: u64 = 0;
    let mut failures: Vec<(PathBuf, crate::fsops::ActError)> = Vec::new();
    let mut copied_n: u64 = 0;
    r.act_open(
        Hue::Milk,
        list.iter().filter(|it| it.taken()).count() as u64,
        dry_run,
    );
    for it in list {
        if !dry_run && interrupt::requested() {
            stopped = true;
            break;
        }
        let rel = rel_display(it.path, &inv.root);
        // a near duplicate nothing confirmed is shown, not moved — the
        // same evidence rule as purge, reversible or not (RESEARCH §2.7) —
        // unless a person chose it in the workspace
        if !it.taken() {
            kept.push((rel, it.near_reason.clone(), it.pair));
            continue;
        }
        let rel_path = it
            .path
            .strip_prefix(&inv.root)
            .map(Path::to_path_buf)
            .unwrap_or_else(|_| PathBuf::from(it.path.file_name().unwrap_or_default()));
        let planned = milked.join(&rel_path);
        let dst = unique_path(&planned);
        // the destination mirrors the source path under _milked; a renamed
        // collision and a near relation are the two things worth a word
        let mut detail = String::new();
        if dst != planned {
            detail = format!(
                "as {}",
                dst.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            );
        }
        if let Some(n) = &it.near {
            if !detail.is_empty() {
                detail.push_str("  ");
            }
            detail.push_str(n);
        }
        if it.by_hand && !it.actionable() {
            detail.push_str("  by hand");
        }
        let verb_dry = if it.near.is_some() { "would~" } else { "would" };
        let verb = if it.near.is_some() { "moved~" } else { "moved" };
        if dry_run {
            r.action(verb_dry, Role::Warn, &rel, &detail);
            moved += 1;
            bytes += it.size;
            if it.by_hand && !it.actionable() {
                by_hand += 1;
            }
            continue;
        }
        match crate::fsops::move_file(it.path, &dst) {
            Ok(copied) => {
                let mut detail = detail;
                if copied {
                    copied_n += 1;
                    if !detail.is_empty() {
                        detail.push_str("  ");
                    }
                    detail.push_str("copied across volumes");
                }
                r.action(verb, Role::Ok, &rel, &detail);
                moved += 1;
                bytes += it.size;
                if it.by_hand && !it.actionable() {
                    by_hand += 1;
                }
                w.record(Entry {
                    op: Op::Move {
                        from: it.path.to_path_buf(),
                        to: dst.clone(),
                        copied,
                    },
                    size: it.size,
                    hash: it.hash.clone(),
                });
            }
            Err(e) => {
                r.action("failed", Role::Danger, &rel, &e.words());
                failed += 1;
                failures.push((it.path.to_path_buf(), e));
            }
        }
    }
    interrupt::changing(false);
    for (path, why, _) in &kept {
        r.action("kept~", Role::Warn, path, why);
    }

    let journal_id = if dry_run {
        None
    } else {
        close_journal(w, r, "moves done")
    };
    let verb = if dry_run { "would move" } else { "moved" };
    r.action_totals(
        verb,
        moved,
        bytes,
        &format!("into {milked_display}"),
        Some(&format!("into {MILKED_DIR}")),
        journal_id.as_deref(),
    );
    if (!f.near.is_empty() || !f.video.is_empty()) && !r.folding() {
        r.note_fit(
            Role::Dupe,
            tilde_legend(by_hand),
            "(images: dHash, pHash and PDQ or the geometric fit agree; videos: TMK or sampled frames)",
        );
    }
    let kept_pairs = kept.iter().filter(|k| k.2).count() as u64;
    let kept_unconfirmed = kept.len() as u64 - kept_pairs;
    if kept_unconfirmed > 0 {
        r.note(
            Role::Warn,
            &format!(
                "{} {}: similar on two hashes, confirmed by nothing else; shown, left in place",
                fmt_count(kept_unconfirmed),
                kept_word(r)
            ),
        );
    }
    pair_note(r, kept_pairs);
    by_hand_note(r, by_hand);
    if copied_n > 0 {
        r.note(
            Role::Dupe,
            &format!(
                "{} copied across volumes: each copy read back and compared before its original was deleted",
                fmt_count(copied_n)
            ),
        );
    }
    if failed > 0 && !r.folding() {
        r.note(
            Role::Danger,
            &format!(
                "{} could not be moved and stay where they are",
                fmt_count(failed)
            ),
        );
    }
    if stopped {
        interrupted_note(r, moved, "moves", journal_id.is_some());
    }
    if journal_id.is_some() {
        r.undo_hint(inv);
    }
    let code = if stopped {
        EXIT_INTERRUPTED
    } else if failed > 0 {
        EXIT_FAILED
    } else {
        EXIT_OK
    };
    Acted {
        code,
        journal: journal_id,
        failed: failures,
    }
}

/// Bin every verified item of `f` (or say what would be binned).
pub(crate) fn act_purge(
    f: &Found,
    list: &[Item<'_>],
    verified: &[bool],
    inv: &Invocation,
    r: &Renderer,
    dry_run: bool,
) -> Acted {
    // exceptions are printed after the action lines, before the footer
    let mut to_bin: Vec<usize> = Vec::new();
    let mut kept: Vec<(&str, String, String)> = Vec::new();
    let mut kept_mismatch: u64 = 0;
    let mut kept_unconfirmed: u64 = 0;
    let mut kept_pairs: u64 = 0;
    for (i, it) in list.iter().enumerate() {
        if verified[i] {
            to_bin.push(i);
        } else if it.near.is_some() {
            if it.pair {
                kept_pairs += 1;
            } else {
                kept_unconfirmed += 1;
            }
            kept.push((
                "kept~",
                rel_display(it.path, &inv.root),
                it.near_reason.clone(),
            ));
        } else {
            kept_mismatch += 1;
            kept.push((
                "kept",
                rel_display(it.path, &inv.root),
                "bytes differ from keeper".to_string(),
            ));
        }
    }

    let mut w = Writer::new(&inv.root, &inv.command_line_as(Command::Purge));
    let mut failures: Vec<(PathBuf, crate::fsops::ActError)> = Vec::new();
    let mut binned: u64 = 0;
    let mut bytes: u64 = 0;
    let mut failed: u64 = 0;
    let mut stopped = false;

    let detail_of = |it: &Item<'_>| -> String {
        let mut d = it.near.clone().unwrap_or_default();
        if it.by_hand && !it.actionable() {
            d.push_str("  by hand");
        }
        d
    };
    let by_hand = to_bin
        .iter()
        .filter(|&&i| list[i].by_hand && !list[i].actionable())
        .count() as u64;
    r.act_open(Hue::Purge, to_bin.len() as u64, dry_run);
    if dry_run {
        for &i in &to_bin {
            let it = &list[i];
            let detail = detail_of(it);
            r.action(
                if it.near.is_some() { "would~" } else { "would" },
                Role::Warn,
                &rel_display(it.path, &inv.root),
                &detail,
            );
            binned += 1;
            bytes += it.size;
        }
    } else {
        // batched bin calls (one shell operation per chunk on Windows; a
        // file the bin cannot hold refused, never deleted outright —
        // `recycle`), then confirm per file by checking it is gone; Ctrl-C
        // stops between chunks
        interrupt::install();
        interrupt::changing(true);
        // on Windows a second name for each file while the shell bins it,
        // so one the bin lets go can be put back (`recycle`); only `_milked`
        // is ever written in the tree, and this is removed after
        let guard = inv.root.join(crate::MILKED_DIR).join(".purging");
        for chunk in to_bin.chunks(500) {
            if interrupt::requested() {
                stopped = true;
                break;
            }
            let paths: Vec<&Path> = chunk.iter().map(|&i| list[i].path).collect();
            let t = unix_secs(SystemTime::now());
            let mut left: std::collections::HashMap<std::path::PathBuf, crate::fsops::ActError> =
                crate::recycle::to_bin(&paths, Some(&guard))
                    .into_iter()
                    .collect();
            for &i in chunk {
                let it = &list[i];
                let rel = rel_display(it.path, &inv.root);
                // a reason, on disk or not, is a failure: a file the bin
                // let go without keeping is not "binned" (Phase 13)
                let why = left.remove(it.path);
                if why.is_none() && !it.path.exists() {
                    let detail = detail_of(it);
                    r.action(
                        if it.near.is_some() {
                            "binned~"
                        } else {
                            "binned"
                        },
                        Role::Ok,
                        &rel,
                        &detail,
                    );
                    binned += 1;
                    bytes += it.size;
                    w.record(Entry {
                        op: Op::Trash {
                            path: it.path.to_path_buf(),
                            time_deleted: t,
                        },
                        size: it.size,
                        hash: it.hash.clone(),
                    });
                } else {
                    let e = why.unwrap_or_else(|| {
                        crate::fsops::ActError::Bin("the file is still there".into())
                    });
                    r.action("failed", Role::Danger, &rel, &e.words());
                    failed += 1;
                    failures.push((it.path.to_path_buf(), e));
                }
            }
        }
    }

    interrupt::changing(false);
    for (verb, path, why) in &kept {
        r.action(verb, Role::Warn, path, why);
    }

    let journal_id = if dry_run {
        None
    } else {
        close_journal(w, r, "files are in the Recycle Bin")
    };
    let verb = if dry_run { "would bin" } else { "binned" };
    r.action_totals(
        verb,
        binned,
        bytes,
        "to the Recycle Bin",
        None,
        journal_id.as_deref(),
    );
    if (!f.near.is_empty() || !f.video.is_empty()) && !r.folding() {
        r.note_fit(
            Role::Dupe,
            tilde_legend(by_hand),
            "(images: dHash, pHash and PDQ or the geometric fit agree; videos: TMK or sampled frames)",
        );
    }
    if kept_mismatch > 0 {
        r.note(
            Role::Warn,
            &format!(
                "{} kept: bytes differed from the keeper at verify time",
                fmt_count(kept_mismatch)
            ),
        );
    }
    if kept_unconfirmed > 0 {
        r.note(
            Role::Warn,
            &format!(
                "{} {}: similar on two hashes, confirmed by nothing else; shown, left in place",
                fmt_count(kept_unconfirmed),
                kept_word(r)
            ),
        );
    }
    pair_note(r, kept_pairs);
    by_hand_note(r, by_hand);
    if failed > 0 && !r.folding() {
        r.note(
            Role::Danger,
            &format!(
                "{} could not be binned and stay where they are",
                fmt_count(failed)
            ),
        );
    }
    if stopped {
        interrupted_note(r, binned, "binned", journal_id.is_some());
    }
    if journal_id.is_some() {
        r.undo_hint(inv);
    }
    let code = if stopped {
        EXIT_INTERRUPTED
    } else if failed > 0 {
        EXIT_FAILED
    } else {
        EXIT_OK
    };
    Acted {
        code,
        journal: journal_id,
        failed: failures,
    }
}

/// RAW + JPEG pairs left together, for the footer.
fn pair_note(r: &Renderer, n: u64) {
    if n > 0 {
        r.note(
            Role::Warn,
            &format!(
                "{} {}: the camera's JPEG beside its RAW stays with it; choose it by hand in the workspace",
                fmt_count(n),
                kept_word(r)
            ),
        );
    }
}

/// What the notes call a file left where it is: the `kept~` rows of the
/// full listing, or the fold's `left in place`.
fn kept_word(r: &Renderer) -> &'static str {
    if r.folding() {
        "left in place"
    } else {
        "kept~"
    }
}

/// The legend of the `~` rows. A row a person chose by hand carries `~`
/// too (it is not a byte copy) but nothing confirmed it — an unconfirmed
/// near duplicate, or another shot (NF-05) — so the legend says so when
/// there is one. [CLAUDE, Phase 11]
fn tilde_legend(by_hand: u64) -> &'static str {
    if by_hand > 0 {
        "~ not a byte copy; a confirmed near duplicate unless marked by hand"
    } else {
        "~ confirmed near duplicate, not a byte copy"
    }
}

/// The footer line for items a person chose although the program's own
/// evidence did not confirm them.
fn by_hand_note(r: &Renderer, n: u64) {
    if n > 0 {
        r.note(
            Role::Warn,
            &format!(
                "{} by hand: chosen in the workspace, confirmed by your look, not by a second family",
                fmt_count(n)
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn unique_path_suffixes() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("x.jpg");
        assert_eq!(unique_path(&p), p);
        fs::write(&p, b"1").unwrap();
        assert_eq!(unique_path(&p), d.path().join("x (2).jpg"));
        fs::write(d.path().join("x (2).jpg"), b"1").unwrap();
        assert_eq!(unique_path(&p), d.path().join("x (3).jpg"));
        let noext = d.path().join("noext");
        fs::write(&noext, b"1").unwrap();
        assert_eq!(unique_path(&noext), d.path().join("noext (2)"));
    }
}
