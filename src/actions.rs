//! The commands: find, milk, purge, undo. Each one runs the shared
//! scan → exact groups → [near groups] pipeline, then acts, then journals.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime};

use rayon::prelude::*;

use crate::cli::{Command, Invocation};
use crate::exact::{self, Group, PipelineStats, Progress, Stage};
use crate::journal::{self, Entry, Journal, Op};
use crate::keeper;
use crate::kind::Kind;
use crate::near::{self, NearGroup, NearStats, Rep};
use crate::perceptual;
use crate::render::{LiveProgress, Renderer};
use crate::theme::Role;
use crate::util::{fmt_count, rel_display, unix_secs};
use crate::video::{self, VRep, VideoGroup, VideoStats};
use crate::walk::{self, Candidate, ScanStats};
use crate::MILKED_DIR;

pub const EXIT_OK: i32 = 0;
pub const EXIT_USER: i32 = 1;
pub const EXIT_FAILED: i32 = 2;

/// Dispatch. Returns the process exit code.
pub fn run(inv: &Invocation, r: &Renderer) -> i32 {
    match inv.command {
        Command::Find => find(inv, r),
        Command::Milk => milk(inv, r),
        Command::Purge => purge(inv, r),
        Command::Undo => undo(inv, r),
        Command::Doctor | Command::Help | Command::Menu => EXIT_OK,
    }
}

struct Found {
    groups: Vec<Group>,
    near: Vec<NearGroup>,
    scan: ScanStats,
    pipe: PipelineStats,
    near_stats: Option<NearStats>,
    /// Decodable images seen when the near pass did not run (for the hint).
    images_seen: u64,
    video: Vec<VideoGroup>,
    video_stats: Option<VideoStats>,
    /// Videos seen when the video pass did not run (for the hint).
    videos_seen: u64,
    /// ffmpeg/ffprobe missing when the video pass was wanted.
    video_tools_missing: bool,
}

impl Found {
    fn empty(&self) -> bool {
        self.groups.is_empty() && self.near.is_empty() && self.video.is_empty()
    }
}

/// The near pass runs for `img` requests and for every `purge` (the
/// comprehensive suite). Quick search is the obvious-duplicates tier and
/// skips it.
fn near_enabled(inv: &Invocation) -> bool {
    !inv.quick && (inv.kind == Kind::Image || inv.command == Command::Purge)
}

/// The video pass runs for `vid` requests and for every `purge`.
fn video_enabled(inv: &Invocation) -> bool {
    !inv.quick && (inv.kind == Kind::Video || inv.command == Command::Purge)
}

fn is_video(p: &Path) -> bool {
    Kind::Video.matches(p)
}

fn scan_and_group(inv: &Invocation, r: &Renderer) -> Found {
    let lp = LiveProgress::new(r.theme);
    let t0 = Instant::now();
    lp.begin(Stage::Scan);
    let (cands, scan) = walk::scan(&inv.root, inv.recurse, inv.kind, &mut |s| {
        lp.update_text(Stage::Scan, s.files, r.scan_text(s, None));
    });
    lp.finish(Stage::Scan, r.scan_text(&scan, Some(t0.elapsed())));

    let near_on = near_enabled(inv);
    let video_on = video_enabled(inv);
    let all: Vec<Candidate> = if near_on || video_on {
        cands.clone()
    } else {
        Vec::new()
    };
    let images_seen = if near_on {
        0
    } else {
        cands
            .iter()
            .filter(|c| perceptual::decodable(&c.path))
            .count() as u64
    };
    let videos_seen = if video_on {
        0
    } else {
        cands.iter().filter(|c| is_video(&c.path)).count() as u64
    };

    let (mut groups, pipe) = exact::find_groups(cands, inv.quick, &lp);
    keeper::order(&mut groups);

    let mut near_groups = Vec::new();
    let mut near_stats = None;
    if near_on {
        // representatives: every decodable image that is not a non-keeper
        // member of an exact group; keepers carry their group index
        let mut role: HashMap<&Path, Option<usize>> = HashMap::new();
        for (gi, g) in groups.iter().enumerate() {
            for (mi, m) in g.members.iter().enumerate() {
                role.insert(m.path.as_path(), if mi == 0 { Some(gi) } else { None });
            }
        }
        let reps: Vec<Rep> = all
            .iter()
            .filter(|c| perceptual::decodable(&c.path))
            .cloned()
            .filter_map(|c| match role.get(c.path.as_path()) {
                Some(None) => None, // exact duplicate of a keeper: already handled
                Some(Some(gi)) => Some(Rep {
                    cand: c,
                    exact_group: Some(*gi),
                }),
                None => Some(Rep {
                    cand: c,
                    exact_group: None,
                }),
            })
            .collect();
        if reps.len() >= 2 {
            let (hashed, mut stats) = near::hash_all(reps, &lp);
            let e = lp.stage_elapsed(Stage::Decode);
            lp.finish_stage(Stage::Decode, lp.decode_summary(&stats, e));
            near_groups = near::group(hashed, &mut stats, &lp);
            let e = lp.stage_elapsed(Stage::Match);
            lp.finish_stage(Stage::Match, lp.match_summary(&stats, e));
            near_stats = Some(stats);
        } else {
            let stats = NearStats {
                decoded: reps.len() as u64,
                ..Default::default()
            };
            lp.finish_stage(
                Stage::Decode,
                lp.decode_summary(&stats, std::time::Duration::ZERO),
            );
            near_stats = Some(stats);
        }
    }

    let mut video_groups = Vec::new();
    let mut video_stats = None;
    let mut video_tools_missing = false;
    if video_on {
        let (ff, fp) = video::tools_present();
        if !(ff && fp) {
            video_tools_missing = true;
        } else {
            let mut role: HashMap<&Path, Option<usize>> = HashMap::new();
            for (gi, g) in groups.iter().enumerate() {
                for (mi, m) in g.members.iter().enumerate() {
                    role.insert(m.path.as_path(), if mi == 0 { Some(gi) } else { None });
                }
            }
            let reps: Vec<(VRep, PathBuf)> = all
                .iter()
                .filter(|c| is_video(&c.path))
                .cloned()
                .filter_map(|c| match role.get(c.path.as_path()) {
                    Some(None) => None,
                    Some(Some(gi)) => Some(VRep {
                        cand: c,
                        exact_group: Some(*gi),
                    }),
                    None => Some(VRep {
                        cand: c,
                        exact_group: None,
                    }),
                })
                .map(|r| {
                    let p = r.cand.path.clone();
                    (r, p)
                })
                .collect();
            if reps.len() >= 2 {
                let (hashed, mut stats) = video::hash_all(reps, &lp);
                let e = lp.stage_elapsed(Stage::Frames);
                lp.finish_stage(Stage::Frames, lp.frames_summary(&stats, e));
                let hashed: Vec<video::VHashed> = hashed
                    .into_iter()
                    .map(|(rep, sig)| video::VHashed { rep, sig })
                    .collect();
                video_groups = video::group(hashed, &mut stats, &lp);
                let e = lp.stage_elapsed(Stage::Match);
                lp.finish_stage(Stage::Match, lp.video_match_summary(&stats, e));
                video_stats = Some(stats);
            } else {
                let stats = VideoStats {
                    probed: reps.len() as u64,
                    hashed: reps.len() as u64,
                    ..Default::default()
                };
                lp.finish_stage(
                    Stage::Frames,
                    lp.frames_summary(&stats, std::time::Duration::ZERO),
                );
                video_stats = Some(stats);
            }
        }
    }

    Found {
        groups,
        near: near_groups,
        video: video_groups,
        video_stats,
        videos_seen,
        video_tools_missing,
        scan,
        pipe,
        near_stats,
        images_seen,
    }
}

fn report(f: &Found, inv: &Invocation, r: &Renderer) {
    r.groups(&f.groups, &inv.root);
    r.near_groups(&f.near, &inv.root, f.groups.len());
    r.video_groups(&f.video, &inv.root, f.groups.len() + f.near.len());
    r.skipped(&f.scan, &f.pipe);
    r.totals(&f.groups, &f.scan);
    r.near_totals(&f.near);
    r.video_totals(&f.video);
    if f.video_tools_missing {
        r.note(
            Role::Warn,
            "video pass skipped: ffmpeg and ffprobe are not on PATH   dedupe doctor",
        );
    }
}

// ---------------------------------------------------------------------------

fn find(inv: &Invocation, r: &Renderer) -> i32 {
    r.header(inv);
    let f = scan_and_group(inv, r);
    report(&f, inv, r);
    if !f.empty() {
        r.next_after_find(inv);
    }
    if f.near_stats.is_none() && f.images_seen >= 2 {
        r.near_hint(inv, f.images_seen);
    }
    if f.video_stats.is_none() && !f.video_tools_missing && f.videos_seen >= 2 {
        r.video_hint(inv, f.videos_seen);
    }
    EXIT_OK
}

// ---------------------------------------------------------------------------

/// One file to move or bin, with what justifies it.
struct Item<'a> {
    path: &'a Path,
    size: u64,
    keeper: &'a Path,
    /// `None` for exact duplicates (verifiable byte for byte); the relation
    /// text for near duplicates.
    near: Option<String>,
    /// For near items: does PDQ confirm the relation to the keeper? `purge`
    /// bins a near duplicate only when it does. `None` for exact items.
    near_confirmed: Option<bool>,
    near_reason: String,
    hash: String,
}

fn items(f: &Found) -> Vec<Item<'_>> {
    let mut v = Vec::new();
    for g in &f.groups {
        for m in &g.members[1..] {
            v.push(Item {
                path: &m.path,
                size: m.size,
                keeper: &g.members[0].path,
                near: None,
                near_confirmed: None,
                near_reason: String::new(),
                hash: exact::hex(&g.hash),
            });
        }
    }
    for g in &f.near {
        let k = &g.members[0].hashed;
        for m in &g.members[1..] {
            let rel = m.relation.unwrap();
            let mut txt = format!("d {} p {} q {}", rel.d, rel.p, rel.q);
            if !rel.variant.label().is_empty() {
                txt.push(' ');
                txt.push_str(rel.variant.label());
            }
            v.push(Item {
                path: &m.hashed.rep.cand.path,
                size: m.hashed.rep.cand.size,
                keeper: &k.rep.cand.path,
                near: Some(txt),
                near_confirmed: Some(rel.confirmed()),
                near_reason: if rel.confirmed() {
                    String::new()
                } else {
                    rel.unconfirmed_reason()
                },
                hash: format!(
                    "near:{:016x}:{}",
                    m.hashed.sig.dhash[0],
                    crate::pdq::hex(&m.hashed.sig.pdq[0])
                ),
            });
        }
    }
    for g in &f.video {
        let k = &g.members[0].hashed;
        for m in &g.members[1..] {
            let rel = m.relation.unwrap();
            let tmk = if rel.level1 >= crate::video::LEVEL1_MIN {
                format!("tmk {:.2}", rel.level2)
            } else {
                "tmk -".to_string()
            };
            let mut txt = match rel.pos {
                Some((d, p, q)) => format!("d {d} p {p} q {q} {tmk}"),
                None => tmk,
            };
            if rel.positional_ok() && !rel.variant.label().is_empty() {
                txt.push(' ');
                txt.push_str(rel.variant.label());
            }
            v.push(Item {
                path: &m.hashed.rep.cand.path,
                size: m.hashed.rep.cand.size,
                keeper: &k.rep.cand.path,
                near: Some(txt),
                near_confirmed: Some(rel.confirmed()),
                near_reason: if rel.confirmed() {
                    String::new()
                } else {
                    rel.unconfirmed_reason()
                },
                hash: format!("video:{}f:{:.3}", m.hashed.sig.frames, rel.level2),
            });
        }
    }
    v
}

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

fn write_journal(
    j: &mut Journal,
    inv: &Invocation,
    r: &Renderer,
    where_are_they: &str,
) -> Option<String> {
    if inv.dry_run || j.entries.is_empty() {
        return None;
    }
    match journal::write(j) {
        Ok(_) => Some(j.id.clone()),
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

fn milk(inv: &Invocation, r: &Renderer) -> i32 {
    r.header(inv);
    let f = scan_and_group(inv, r);
    report(&f, inv, r);
    if f.empty() {
        return EXIT_OK;
    }
    r.blank();

    let milked = inv.root.join(MILKED_DIR);
    let mut j = Journal::new(&inv.root, &inv.command_line());
    let mut moved: u64 = 0;
    let mut bytes: u64 = 0;
    let mut failed: u64 = 0;

    for it in items(&f) {
        let rel = rel_display(it.path, &inv.root);
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
        let verb_dry = if it.near.is_some() { "would~" } else { "would" };
        let verb = if it.near.is_some() { "moved~" } else { "moved" };
        if inv.dry_run {
            r.action(verb_dry, Role::Warn, &rel, &detail);
            moved += 1;
            bytes += it.size;
            continue;
        }
        let res = dst
            .parent()
            .map(fs::create_dir_all)
            .unwrap_or(Ok(()))
            .and_then(|_| fs::rename(it.path, &dst));
        match res {
            Ok(()) => {
                r.action(verb, Role::Ok, &rel, &detail);
                moved += 1;
                bytes += it.size;
                j.entries.push(Entry {
                    op: Op::Move {
                        from: it.path.to_path_buf(),
                        to: dst.clone(),
                    },
                    size: it.size,
                    hash: it.hash.clone(),
                });
            }
            Err(e) => {
                r.action("failed", Role::Danger, &rel, &os_err(&e));
                failed += 1;
            }
        }
    }

    let journal_id = write_journal(&mut j, inv, r, "moves done");
    let verb = if inv.dry_run { "would move" } else { "moved" };
    r.action_totals(
        verb,
        moved,
        bytes,
        &format!("into {}", milked.to_string_lossy()),
        journal_id.as_deref(),
    );
    if !f.near.is_empty() || !f.video.is_empty() {
        r.note(
            Role::Dupe,
            "~ marks a near duplicate (perceptual match), not a byte copy",
        );
    }
    if failed > 0 {
        r.note(
            Role::Danger,
            &format!(
                "{} could not be moved and stay where they are",
                fmt_count(failed)
            ),
        );
    }
    if journal_id.is_some() {
        r.undo_hint(inv);
    }
    if failed > 0 {
        EXIT_FAILED
    } else {
        EXIT_OK
    }
}

// ---------------------------------------------------------------------------

fn purge(inv: &Invocation, r: &Renderer) -> i32 {
    r.header(inv);
    let f = scan_and_group(inv, r);
    let list = items(&f);

    // VERIFY — exact items byte for byte against the keeper; near items are
    // justified by two agreeing hashes and cannot be byte-verified
    let lp = LiveProgress::new(r.theme);
    let exact_idx: Vec<usize> = list
        .iter()
        .enumerate()
        .filter(|(_, it)| it.near.is_none())
        .map(|(i, _)| i)
        .collect();
    let total = exact_idx.len() as u64;
    // exact items are verified below byte for byte; near items are verified
    // by PDQ agreeing with the two 64-bit hashes (Relation::confirmed)
    let mut verified: Vec<bool> = list
        .iter()
        .map(|it| it.near_confirmed.unwrap_or(false))
        .collect();
    if total > 0 {
        if inv.quick {
            // sample-level matches, explicitly requested; no byte verify is meaningful
            for &i in &exact_idx {
                verified[i] = true;
            }
        } else {
            lp.tick(Stage::Verify, 0, total);
            let counter = std::sync::atomic::AtomicU64::new(0);
            let results: Vec<(usize, bool)> = exact_idx
                .par_iter()
                .map(|&i| {
                    let it = &list[i];
                    let ok = exact::verify_identical(it.keeper, it.path).unwrap_or(false);
                    let d = counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                    lp.tick(Stage::Verify, d, total);
                    (i, ok)
                })
                .collect();
            let mut ok_n = 0u64;
            for (i, ok) in results {
                verified[i] = ok;
                if ok {
                    ok_n += 1;
                }
            }
            let mut p = f.pipe;
            p.confirmed = ok_n;
            lp.done(Stage::Verify, &p);
        }
    }

    report(&f, inv, r);
    if f.empty() {
        return EXIT_OK;
    }
    r.blank();

    // exceptions are printed after the action lines, before the footer
    let mut to_bin: Vec<usize> = Vec::new();
    let mut kept: Vec<(&str, String, String)> = Vec::new();
    let mut kept_mismatch: u64 = 0;
    let mut kept_unconfirmed: u64 = 0;
    for (i, it) in list.iter().enumerate() {
        if verified[i] {
            to_bin.push(i);
        } else if it.near.is_some() {
            kept_unconfirmed += 1;
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

    let mut j = Journal::new(&inv.root, &inv.command_line());
    let mut binned: u64 = 0;
    let mut bytes: u64 = 0;
    let mut failed: u64 = 0;

    if inv.dry_run {
        for &i in &to_bin {
            let it = &list[i];
            let detail = it.near.clone().unwrap_or_default();
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
        // batched bin calls (one shell operation per chunk on Windows), then
        // confirm per file by checking it is gone
        for chunk in to_bin.chunks(500) {
            let paths: Vec<&Path> = chunk.iter().map(|&i| list[i].path).collect();
            let t = unix_secs(SystemTime::now());
            let res = trash::delete_all(&paths);
            for &i in chunk {
                let it = &list[i];
                let rel = rel_display(it.path, &inv.root);
                if !it.path.exists() {
                    let detail = it.near.clone().unwrap_or_default();
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
                    j.entries.push(Entry {
                        op: Op::Trash {
                            path: it.path.to_path_buf(),
                            time_deleted: t,
                        },
                        size: it.size,
                        hash: it.hash.clone(),
                    });
                } else {
                    let why = match &res {
                        Err(e) => e.to_string(),
                        Ok(()) => "still present".to_string(),
                    };
                    r.action("failed", Role::Danger, &rel, &why);
                    failed += 1;
                }
            }
        }
    }

    for (verb, path, why) in &kept {
        r.action(verb, Role::Warn, path, why);
    }

    let journal_id = write_journal(&mut j, inv, r, "files are in the Recycle Bin");
    let verb = if inv.dry_run { "would bin" } else { "binned" };
    r.action_totals(
        verb,
        binned,
        bytes,
        "to the Recycle Bin",
        journal_id.as_deref(),
    );
    if !f.near.is_empty() || !f.video.is_empty() {
        r.note(
            Role::Dupe,
            "~ near duplicate confirmed against the keeper (images: dHash+pHash+PDQ; videos: TMK or sampled frames), not a byte copy",
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
        let mut milk_cmd = inv.command_line_as(Command::Milk);
        if inv.dry_run {
            milk_cmd = milk_cmd.replace(" --dry-run", "");
        }
        r.note(
            Role::Warn,
            &format!(
                "{} kept~: similar, not confirmed for binning; {} moves them instead",
                fmt_count(kept_unconfirmed),
                milk_cmd
            ),
        );
    }
    if failed > 0 {
        r.note(
            Role::Danger,
            &format!(
                "{} could not be binned and stay where they are",
                fmt_count(failed)
            ),
        );
    }
    if journal_id.is_some() {
        r.undo_hint(inv);
    }
    if failed > 0 {
        EXIT_FAILED
    } else {
        EXIT_OK
    }
}

// ---------------------------------------------------------------------------

fn undo(inv: &Invocation, r: &Renderer) -> i32 {
    r.header(inv);
    let (jpath, mut j) = match journal::latest_pending(&inv.root) {
        Ok(Some(x)) => x,
        Ok(None) => {
            r.note(
                Role::Dupe,
                &format!("nothing to undo under {}", inv.root.to_string_lossy()),
            );
            return EXIT_OK;
        }
        Err(e) => {
            r.error(&format!("journal unreadable  {e}"));
            return EXIT_FAILED;
        }
    };
    r.note(
        Role::Plate,
        &format!(
            "journal {}   {}   {} entries",
            j.id,
            j.command,
            fmt_count(j.entries.len() as u64)
        ),
    );
    r.blank();

    let mut restored: u64 = 0;
    let mut bytes: u64 = 0;
    let mut failed: u64 = 0;

    // moves, newest first
    for e in j.entries.iter().rev() {
        if let Op::Move { from, to } = &e.op {
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
            let res = from
                .parent()
                .map(fs::create_dir_all)
                .unwrap_or(Ok(()))
                .and_then(|_| fs::rename(to, from));
            match res {
                Ok(()) => {
                    r.action("restored", Role::Ok, &rel, "");
                    restored += 1;
                    bytes += e.size;
                }
                Err(err) => {
                    r.action("failed", Role::Danger, &rel, &os_err(&err));
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

    if !inv.dry_run {
        if let Err(e) = journal::mark_consumed(&jpath, &mut j) {
            r.note(
                Role::Danger,
                &format!("journal could not be marked undone: {e}"),
            );
        }
        prune_empty_dirs(&inv.root.join(MILKED_DIR));
    }

    let verb = if inv.dry_run {
        "would restore"
    } else {
        "restored"
    };
    r.action_totals(verb, restored, bytes, "back in place", Some(&j.id));
    if failed > 0 {
        r.note(
            Role::Danger,
            &format!("{} could not be restored", fmt_count(failed)),
        );
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

pub fn os_err(e: &std::io::Error) -> String {
    let s = e.to_string();
    // strip the "(os error N)" suffix std appends
    match s.find(" (os error") {
        Some(i) => s[..i].to_string(),
        None => s,
    }
}

#[cfg(test)]
mod tests {
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
