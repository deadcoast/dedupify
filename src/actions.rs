//! The commands: find, milk, purge, undo. Each one runs the shared
//! scan → exact groups → [near groups] pipeline, then acts, then journals.

use std::collections::HashMap;
use std::fs;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime};

use rayon::prelude::*;

use crate::cli::{Command, Invocation};
use crate::exact::{self, Group, PipelineStats, Progress, Stage};
use crate::interrupt;
use crate::journal::{self, Entry, Journal, Op, Writer};
use crate::keeper;
use crate::kind::Kind;
use crate::near::{self, NearGroup, NearStats, Rep};
use crate::perceptual;
use crate::render::{LiveProgress, Renderer};
use crate::theme::Role;
use crate::util::{counted, fmt_count, rel_display, shorten_middle, under_root, unix_secs, width};
use crate::video::{self, VRep, VideoGroup, VideoStats};
use crate::walk::{self, Candidate, ScanStats};
use crate::MILKED_DIR;

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

/// What one scan found: the exact groups, the near groups, the video
/// groups and the stage statistics. `find` shows it and then hands it to
/// the workspace, which acts on it without scanning again (and keeps a
/// copy from before each change, so `u` can put the groups back).
#[derive(Clone)]
pub(crate) struct Found {
    pub(crate) groups: Vec<Group>,
    pub(crate) near: Vec<NearGroup>,
    pub(crate) scan: ScanStats,
    pub(crate) pipe: PipelineStats,
    /// Decodable images seen when the near pass did not run (quick search).
    pub(crate) images_seen: u64,
    pub(crate) video: Vec<VideoGroup>,
    /// Videos seen when the video pass did not run (quick search).
    pub(crate) videos_seen: u64,
    /// ffmpeg/ffprobe missing when the video pass was wanted.
    pub(crate) video_tools_missing: bool,
}

impl Found {
    pub(crate) fn empty(&self) -> bool {
        self.groups.is_empty() && self.near.is_empty() && self.video.is_empty()
    }

    /// Groups as numbered on screen: exact, then near, then video.
    pub(crate) fn group_count(&self) -> usize {
        self.groups.len() + self.near.len() + self.video.len()
    }

    /// Make member `idx` of on-screen group `n` (1-based) the keeper.
    /// Returns false when either number is out of range.
    pub(crate) fn set_keeper(&mut self, n: usize, idx: usize) -> bool {
        if n == 0 {
            return false;
        }
        let mut i = n - 1;
        if i < self.groups.len() {
            let g = &mut self.groups[i];
            if idx == 0 || idx >= g.members.len() {
                return false;
            }
            let m = g.members.remove(idx);
            g.members.insert(0, m);
            return true;
        }
        i -= self.groups.len();
        if i < self.near.len() {
            let g = &mut self.near[i];
            if idx == 0 || idx >= g.members.len() {
                return false;
            }
            g.set_keeper(idx);
            return true;
        }
        i -= self.near.len();
        if i < self.video.len() {
            let g = &mut self.video[i];
            if idx == 0 || idx >= g.members.len() {
                return false;
            }
            g.set_keeper(idx);
            return true;
        }
        false
    }

    /// After a change: drop every member no longer on disk, then every
    /// group left with fewer than two, and renumber the `+GROUP` links the
    /// near and video members carry to the exact groups that remain.
    pub(crate) fn prune_missing(&mut self) {
        let mut renumber: Vec<Option<usize>> = Vec::with_capacity(self.groups.len());
        let mut next = 0usize;
        for g in &mut self.groups {
            g.members.retain(|m| m.path.exists());
            if g.members.len() >= 2 {
                renumber.push(Some(next));
                next += 1;
            } else {
                renumber.push(None);
            }
        }
        self.groups.retain(|g| g.members.len() >= 2);
        let remap = |eg: &mut Option<usize>| {
            *eg = eg.and_then(|i| renumber.get(i).copied().flatten());
        };
        for g in &mut self.near {
            g.members.retain(|m| m.hashed.rep.cand.path.exists());
            for m in &mut g.members {
                remap(&mut m.hashed.rep.exact_group);
            }
        }
        self.near.retain(|g| g.members.len() >= 2);
        for g in &mut self.video {
            g.members.retain(|m| m.hashed.rep.cand.path.exists());
            for m in &mut g.members {
                remap(&mut m.hashed.rep.exact_group);
            }
        }
        self.video.retain(|g| g.members.len() >= 2);
    }
}

/// One rule for depth: `find`, `milk` and `purge` all run the full suite,
/// so what `find` lists is what `milk` and `purge` act on; the kind token
/// scopes what is scanned, nothing else. `--quick-search` is the fast
/// byte-copy tier and skips the near passes. (Phase 8: before it, `find`
/// and `milk` ran the near passes only with a kind token, and a folder
/// `find . img` had just listed fifteen near duplicates in answered
/// `milk .` with "no duplicates".)
fn near_enabled(inv: &Invocation) -> bool {
    !inv.quick && inv.kind != Kind::Video
}

/// The video pass, by the same rule.
fn video_enabled(inv: &Invocation) -> bool {
    !inv.quick && inv.kind != Kind::Image
}

fn is_video(p: &Path) -> bool {
    Kind::Video.matches(p)
}

pub(crate) fn scan_and_group(inv: &Invocation, r: &Renderer) -> Found {
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

    // an empty scan has nothing to stage; the totals line says so
    let (mut groups, pipe) = if cands.is_empty() {
        (Vec::new(), PipelineStats::default())
    } else {
        exact::find_groups(cands, inv.quick, &lp)
    };
    keeper::order(&mut groups);

    let mut near_groups = Vec::new();
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
            // the deep tier is on when a model file is installed
            let deep_model =
                crate::deep::installed().and_then(|p| match crate::deep::Deep::load(&p) {
                    Ok(m) => Some(m),
                    Err(e) => {
                        r.note(
                            Role::Warn,
                            &format!(
                                "deep model at {} could not be loaded: {e}",
                                p.to_string_lossy()
                            ),
                        );
                        None
                    }
                });
            let (hashed, mut stats) = near::hash_all(reps, &lp, deep_model.as_ref());
            let e = lp.stage_elapsed(Stage::Decode);
            lp.finish_stage(Stage::Decode, lp.decode_summary(&stats, e));
            near_groups = near::group(hashed, &mut stats, &lp);
            let e = lp.stage_elapsed(Stage::Match);
            lp.finish_stage(Stage::Match, lp.match_summary(&stats, e));
        } else if !reps.is_empty() {
            // one image: nothing to compare, said once; none: no stage line
            let stats = NearStats {
                decoded: reps.len() as u64,
                ..Default::default()
            };
            lp.finish_stage(
                Stage::Decode,
                lp.decode_summary(&stats, std::time::Duration::ZERO),
            );
        }
    }

    let mut video_groups = Vec::new();
    let mut video_tools_missing = false;
    let videos_present = all.iter().any(|c| is_video(&c.path));
    if video_on && videos_present {
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
            } else if !reps.is_empty() {
                let stats = VideoStats {
                    probed: reps.len() as u64,
                    hashed: reps.len() as u64,
                    ..Default::default()
                };
                lp.finish_stage(
                    Stage::Frames,
                    lp.frames_summary(&stats, std::time::Duration::ZERO),
                );
            }
        }
    }

    Found {
        groups,
        near: near_groups,
        video: video_groups,
        videos_seen,
        video_tools_missing,
        scan,
        pipe,
        images_seen,
    }
}

pub(crate) fn report(f: &Found, inv: &Invocation, r: &Renderer) {
    r.groups(&f.groups, &inv.root);
    r.near_groups(&f.near, &inv.root, f.groups.len());
    r.video_groups(&f.video, &inv.root, f.groups.len() + f.near.len());
    r.skipped(&f.scan, &f.pipe);
    r.totals(
        &f.groups,
        &f.scan,
        !f.near.is_empty() || !f.video.is_empty(),
    );
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

// ---------------------------------------------------------------------------

/// One file to move or bin, with what justifies it.
pub(crate) struct Item<'a> {
    pub(crate) path: &'a Path,
    pub(crate) size: u64,
    pub(crate) keeper: &'a Path,
    /// `None` for exact duplicates (verifiable byte for byte); the relation
    /// text for near duplicates.
    pub(crate) near: Option<String>,
    /// For near items: does a second family confirm the relation to the
    /// keeper? `milk` and `purge` act on a near duplicate only when it
    /// does. `None` for exact items.
    pub(crate) near_confirmed: Option<bool>,
    pub(crate) near_reason: String,
    pub(crate) hash: String,
    /// Listed by the report but never acted on by the program's own rule:
    /// the learned descriptor's word alone. Only a person's choice in the
    /// workspace can include it (`by_hand`).
    pub(crate) listed: bool,
    /// Chosen in the workspace although the program's evidence did not
    /// confirm it: the person's look is the second family. [CLAUDE]
    pub(crate) by_hand: bool,
}

impl Item<'_> {
    /// Would `milk` move it / `purge` bin it on the program's own rule
    /// (before byte verification)?
    pub(crate) fn actionable(&self) -> bool {
        !self.listed && self.near_confirmed != Some(false)
    }

    /// Will the action take it: the program's rule, or a person's choice.
    pub(crate) fn taken(&self) -> bool {
        self.actionable() || self.by_hand
    }
}

/// Every member that could be acted on, keepers and other capture moments
/// excluded. The commands take `default_items` (the program's rule); the
/// workspace takes the person's selection out of this list.
pub(crate) fn items(f: &Found) -> Vec<Item<'_>> {
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
                listed: false,
                by_hand: false,
            });
        }
    }
    for g in &f.near {
        let k = &g.members[0].hashed;
        for m in &g.members[1..] {
            let rel = m.relation.unwrap();
            if rel.kin {
                // another capture moment: another photograph, never moved
                // or binned, by the program or by hand (RESEARCH §2.8)
                continue;
            }
            // the words first, the evidence after, as on the listing
            let txt = format!("{}  {}", rel.words(k, &m.hashed), rel.numbers());
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
                // the descriptor's word alone: listed, acted on only by hand
                // (RESEARCH §2.6)
                listed: rel.listed_only(),
                by_hand: false,
            });
        }
    }
    for g in &f.video {
        let k = &g.members[0].hashed;
        for m in &g.members[1..] {
            let rel = m.relation.unwrap();
            let txt = format!("{}  {}", rel.words(&k.sig, &m.hashed.sig), rel.numbers());
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
                listed: false,
                by_hand: false,
            });
        }
    }
    v
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

/// The items the commands act on without a person in the loop: everything
/// but the descriptor-only listings (shown on the report, never acted on).
/// Unconfirmed near duplicates stay in the list so the action prints them
/// as `kept~` with their reason.
pub(crate) fn default_items(f: &Found) -> Vec<Item<'_>> {
    items(f).into_iter().filter(|it| !it.listed).collect()
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

fn milk(inv: &Invocation, r: &Renderer) -> i32 {
    r.header(inv);
    let f = scan_and_group(inv, r);
    report(&f, inv, r);
    if f.empty() {
        return EXIT_OK;
    }
    r.blank();
    let list = default_items(&f);
    act_milk(&f, &list, inv, r, inv.dry_run).0
}

/// Move every actionable item of `f` into `_milked` (or say what would
/// move). Returns the exit code and the journal id when one was written.
pub(crate) fn act_milk(
    f: &Found,
    list: &[Item<'_>],
    inv: &Invocation,
    r: &Renderer,
    dry_run: bool,
) -> (i32, Option<String>) {
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
    let mut kept: Vec<(String, String)> = Vec::new();
    let mut by_hand: u64 = 0;
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
            kept.push((rel, it.near_reason.clone()));
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
                if it.by_hand && !it.actionable() {
                    by_hand += 1;
                }
                w.record(Entry {
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
    interrupt::changing(false);
    for (path, why) in &kept {
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
    if !f.near.is_empty() || !f.video.is_empty() {
        r.note_fit(
            Role::Dupe,
            "~ confirmed near duplicate, not a byte copy",
            "(images: dHash, pHash and PDQ or the geometric fit agree; videos: TMK or sampled frames)",
        );
    }
    if !kept.is_empty() {
        r.note(
            Role::Warn,
            &format!(
                "{} kept~: similar on two hashes, confirmed by nothing else; shown, left in place",
                fmt_count(kept.len() as u64)
            ),
        );
    }
    by_hand_note(r, by_hand);
    if failed > 0 {
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
    (code, journal_id)
}

// ---------------------------------------------------------------------------

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
    act_purge(&f, &list, &verified, inv, r, inv.dry_run).0
}

/// VERIFY — exact items byte for byte against the keeper; near items are
/// justified by two agreeing families and cannot be byte-verified, so
/// their flag is their confirmation. `--quick-search` skips the full
/// hash, never this: purge bins nothing it has not read against its keeper.
pub(crate) fn verify_items(list: &[Item<'_>], pipe: &PipelineStats, r: &Renderer) -> Vec<bool> {
    let lp = LiveProgress::new(r.theme);
    let exact_idx: Vec<usize> = list
        .iter()
        .enumerate()
        .filter(|(_, it)| it.near.is_none())
        .map(|(i, _)| i)
        .collect();
    let total = exact_idx.len() as u64;
    // a near item is verified by its evidence or by a person's choice
    let mut verified: Vec<bool> = list
        .iter()
        .map(|it| it.near.is_some() && it.taken())
        .collect();
    if total > 0 {
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
        let mut p = *pipe;
        p.confirmed = ok_n;
        lp.done(Stage::Verify, &p);
    }
    verified
}

/// Bin every verified item of `f` (or say what would be binned).
pub(crate) fn act_purge(
    f: &Found,
    list: &[Item<'_>],
    verified: &[bool],
    inv: &Invocation,
    r: &Renderer,
    dry_run: bool,
) -> (i32, Option<String>) {
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

    let mut w = Writer::new(&inv.root, &inv.command_line_as(Command::Purge));
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
        // batched bin calls (one shell operation per chunk on Windows), then
        // confirm per file by checking it is gone; Ctrl-C stops between chunks
        interrupt::install();
        interrupt::changing(true);
        for chunk in to_bin.chunks(500) {
            if interrupt::requested() {
                stopped = true;
                break;
            }
            let paths: Vec<&Path> = chunk.iter().map(|&i| list[i].path).collect();
            let t = unix_secs(SystemTime::now());
            let res = trash::delete_all(&paths);
            for &i in chunk {
                let it = &list[i];
                let rel = rel_display(it.path, &inv.root);
                if !it.path.exists() {
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
    if !f.near.is_empty() || !f.video.is_empty() {
        r.note_fit(
            Role::Dupe,
            "~ confirmed near duplicate, not a byte copy",
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
                "{} kept~: similar on two hashes, confirmed by nothing else; shown, left in place",
                fmt_count(kept_unconfirmed)
            ),
        );
    }
    by_hand_note(r, by_hand);
    if failed > 0 {
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
    (code, journal_id)
}

// ---------------------------------------------------------------------------

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
        if let Op::Move { from, to } = &e.op {
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
