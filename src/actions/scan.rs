//! The scan: walk the root, group exact copies, then near images and
//! videos, into one `Found` that `find` reports and every action reads.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::cli::Invocation;
use crate::exact::{self, Group, PipelineStats, Stage};
use crate::keeper;
use crate::kind::Kind;
use crate::near::{self, NearGroup, NearStats, Rep};
use crate::render::fold::{self, Hue, Mark, Yield};
use crate::render::{LiveProgress, Renderer};
use crate::theme::{Role, Theme};
use crate::util::{fmt_bytes, fmt_count};
use crate::video::{self, VRep, VideoGroup, VideoStats};
use crate::walk::{self, Candidate, ScanStats};

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
    /// Files whose ffmpeg sent nothing for `tools::stall()` and was stopped:
    /// left out of this run, named under the totals (Phase 13, CORE-11).
    pub(crate) stalled: Vec<PathBuf>,
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
        let Some(g) = n.checked_sub(1).and_then(|i| self.view_mut(i)) else {
            return false;
        };
        if idx == 0 || idx >= g.len() {
            return false;
        }
        g.set_keeper(idx);
        true
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
    let filter = walk::Filter {
        skip: &inv.skip,
        min: inv.min,
    };
    let (cands, scan) = walk::scan(&inv.root, inv.recurse, inv.kind, filter, &mut |s| {
        lp.scan(s);
    });
    lp.finish(Stage::Scan, r.scan_text(&scan, Some(t0.elapsed())));
    let t = &r.theme;
    lp.settle(
        Hue::Files,
        if scan.files == 0 {
            Mark::Empty
        } else {
            Mark::Done
        },
        &files_segments(t, &scan),
    );

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
            .filter(|c| crate::decode::decodable(&c.path))
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
    if scan.files > 0 {
        let (mark, segs) = copies_segments(t, &groups, &pipe);
        lp.settle(Hue::Copies, mark, &segs);
    }

    let mut stalled: Vec<PathBuf> = Vec::new();
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
            .filter(|c| crate::decode::decodable(&c.path))
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
            stalled.extend(stats.stalled.iter().cloned());
            let e = lp.stage_elapsed(Stage::Decode);
            lp.finish_stage(Stage::Decode, lp.decode_summary(&stats, e));
            near_groups = near::group(hashed, &mut stats, &lp);
            let e = lp.stage_elapsed(Stage::Match);
            lp.finish_stage(Stage::Match, lp.match_summary(&stats, e));
            let (mark, segs) = similar_segments(t, &near_groups, &stats);
            lp.settle(Hue::Similar, mark, &segs);
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
            let (mark, segs) = similar_segments(t, &[], &stats);
            lp.settle(Hue::Similar, mark, &segs);
        }
    }

    let mut video_groups = Vec::new();
    let mut video_tools_missing = false;
    let videos_present = all.iter().any(|c| is_video(&c.path));
    if video_on && videos_present {
        let (ff, fp) = video::tools_present();
        if !(ff && fp) {
            video_tools_missing = true;
            lp.settle(
                Hue::Video,
                Mark::Warn,
                &[
                    fold::warn(t, "skipped: ffmpeg and ffprobe not found"),
                    fold::dim(t, "dedupe doctor"),
                ],
            );
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
                stalled.extend(stats.stalled.iter().cloned());
                let e = lp.stage_elapsed(Stage::Frames);
                lp.finish_stage(Stage::Frames, lp.frames_summary(&stats, e));
                let hashed: Vec<video::VHashed> = hashed
                    .into_iter()
                    .map(|(rep, sig)| video::VHashed { rep, sig })
                    .collect();
                video_groups = video::group(hashed, &mut stats, &lp);
                let e = lp.stage_elapsed(Stage::Match);
                lp.finish_stage(Stage::Match, lp.video_match_summary(&stats, e));
                let (mark, segs) = video_segments(t, &video_groups, &stats);
                lp.settle(Hue::Video, mark, &segs);
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
                let (mark, segs) = video_segments(t, &[], &stats);
                lp.settle(Hue::Video, mark, &segs);
            }
        }
    }

    Found {
        groups,
        near: near_groups,
        video: video_groups,
        videos_seen,
        video_tools_missing,
        stalled,
        scan,
        pipe,
        images_seen,
    }
}

pub(crate) fn report(f: &Found, inv: &Invocation, r: &Renderer) {
    if r.folding() {
        // the sections said what each pass found; the yield says what it
        // comes to
        r.blank();
        fold::reveal(&r.theme, &yield_of(f));
        if !f.stalled.is_empty() {
            r.blank();
            r.note(Role::Warn, &stalled_note(&f.stalled, &inv.root));
        }
        r.blank();
        return;
    }
    r.listing(&f.groups, &f.near, &f.video, &inv.root);
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
            "video pass skipped: ffmpeg and ffprobe not found   dedupe doctor",
        );
    }
    if !f.stalled.is_empty() {
        r.note(Role::Warn, &stalled_note(&f.stalled, &inv.root));
    }
}

/// The files a stalled ffmpeg was stopped on, by name: the first three,
/// then how many more.
pub(crate) fn stalled_note(paths: &[PathBuf], root: &Path) -> String {
    let mut sorted: Vec<&PathBuf> = paths.iter().collect();
    sorted.sort();
    let mut names: Vec<String> = sorted
        .iter()
        .take(3)
        .map(|p| crate::util::rel_display(p, root))
        .collect();
    if paths.len() > 3 {
        names.push(format!("and {} more", paths.len() - 3));
    }
    format!(
        "timed out: {}   ffmpeg sent nothing for {} s and was stopped; left out of this run",
        names.join(", "),
        crate::tools::stall().as_secs()
    )
}

// ---------------------------------------------------------------------------
// the fold's sections (docs/CLI.md §3a)
// ---------------------------------------------------------------------------

/// What the walk found and what it left out.
fn files_segments(t: &Theme, s: &ScanStats) -> Vec<String> {
    let mut v = vec![
        fold::count(t, s.files, "file", "files"),
        fold::figure(t, &fmt_bytes(s.bytes)),
    ];
    if s.errors > 0 {
        v.push(fold::warn(
            t,
            &format!("{} not listed", fmt_count(s.errors)),
        ));
    }
    for (n, what) in [
        (s.skipped, "skipped".to_string()),
        (s.small, format!("under {}", fmt_bytes(s.min))),
        (s.online, "online only, not read".to_string()),
        (s.system, "bin and system folders left out".to_string()),
        (s.empty, "empty".to_string()),
    ] {
        if n > 0 {
            v.push(fold::dim(t, &format!("{} {what}", fmt_count(n))));
        }
    }
    v
}

/// The byte-for-byte pass: groups, the copies beyond their keepers, what
/// they free.
fn copies_segments(t: &Theme, groups: &[exact::Group], p: &PipelineStats) -> (Mark, Vec<String>) {
    let mut v = Vec::new();
    let mut mark = Mark::Done;
    if groups.is_empty() {
        mark = Mark::Empty;
        v.push(fold::dim(t, "no exact copies"));
    } else {
        let copies: u64 = groups.iter().map(|g| g.members.len() as u64 - 1).sum();
        v.push(fold::count(t, groups.len() as u64, "group", "groups"));
        v.push(fold::count(t, copies, "copy", "copies"));
        v.push(fold::figure(
            t,
            &fmt_bytes(groups.iter().map(exact::Group::reclaimable).sum()),
        ));
        if groups.iter().any(|g| g.tier == exact::Tier::Sample) {
            mark = Mark::Warn;
            v.push(fold::warn(t, "quick: size and sample, not full content"));
        }
    }
    if p.unreadable > 0 {
        mark = Mark::Warn;
        v.push(fold::warn(
            t,
            &format!("{} unreadable", fmt_count(p.unreadable)),
        ));
    }
    if p.hardlinks_collapsed > 0 {
        v.push(fold::dim(
            t,
            &format!("{} hard links collapsed", fmt_count(p.hardlinks_collapsed)),
        ));
    }
    (mark, v)
}

/// The image pass: groups, similar members, what they free, what could
/// not be read, then what the members are.
fn similar_segments(t: &Theme, groups: &[NearGroup], n: &NearStats) -> (Mark, Vec<String>) {
    let mut v = Vec::new();
    let mut warned = Vec::new();
    let stalled = n.stalled.len() as u64;
    for (k, what) in [
        (stalled, "timed out"),
        (n.need_ffmpeg, "need ffmpeg"),
        (n.ffmpeg_failed, "ffmpeg could not decode"),
    ] {
        if k > 0 {
            warned.push(fold::warn(t, &format!("{} {what}", fmt_count(k))));
        }
    }
    let unreadable = n
        .undecodable
        .saturating_sub(n.need_ffmpeg + n.ffmpeg_failed + n.no_preview + stalled);
    if groups.is_empty() {
        v.push(format!(
            "{} {}",
            fold::dim(
                t,
                if n.decoded < 2 {
                    "nothing to compare in"
                } else {
                    "none among"
                }
            ),
            fold::count(t, n.decoded, "image", "images")
        ));
        let mark = if warned.is_empty() {
            Mark::Empty
        } else {
            Mark::Warn
        };
        v.extend(warned);
        if unreadable + n.no_preview > 0 {
            v.push(fold::dim(
                t,
                &format!("{} not decoded", fmt_count(unreadable + n.no_preview)),
            ));
        }
        return (mark, v);
    }
    // the members the actions take: what the yield counts, so the
    // sections add up to it; the rest are named in the tally
    let similar: u64 = groups
        .iter()
        .flat_map(|g| g.members[1..].iter())
        .filter(|m| m.relation.is_some_and(|r| r.confirmed()))
        .count() as u64;
    v.push(fold::count(t, groups.len() as u64, "group", "groups"));
    v.push(fold::count(t, similar, "similar", "similar"));
    v.push(fold::figure(
        t,
        &fmt_bytes(groups.iter().map(NearGroup::reclaimable).sum()),
    ));
    let mark = if warned.is_empty() {
        Mark::Done
    } else {
        Mark::Warn
    };
    v.extend(warned);
    let words = groups.iter().flat_map(|g| {
        let k = &g.members[0].hashed;
        g.members[1..]
            .iter()
            .filter_map(move |m| m.relation.map(|r| r.words(k, &m.hashed)))
    });
    v.extend(fold::tally_segments(t, fold::tally(words)));
    if n.deep {
        v.push(fold::dim(t, "with the deep model"));
    }
    if unreadable + n.no_preview > 0 {
        v.push(fold::dim(
            t,
            &format!("{} not decoded", fmt_count(unreadable + n.no_preview)),
        ));
    }
    (mark, v)
}

/// The video pass, as the image pass.
fn video_segments(t: &Theme, groups: &[VideoGroup], s: &VideoStats) -> (Mark, Vec<String>) {
    let mut v = Vec::new();
    let stalled = s.stalled.len() as u64;
    let warned: Vec<String> = if stalled > 0 {
        vec![fold::warn(t, &format!("{} timed out", fmt_count(stalled)))]
    } else {
        Vec::new()
    };
    if groups.is_empty() {
        v.push(format!(
            "{} {}",
            fold::dim(
                t,
                if s.hashed < 2 {
                    "nothing to compare in"
                } else {
                    "none among"
                }
            ),
            fold::count(t, s.hashed, "video", "videos")
        ));
        let mark = if warned.is_empty() {
            Mark::Empty
        } else {
            Mark::Warn
        };
        v.extend(warned);
        return (mark, v);
    }
    let similar: u64 = groups
        .iter()
        .flat_map(|g| g.members[1..].iter())
        .filter(|m| m.relation.is_some_and(|r| r.confirmed()))
        .count() as u64;
    v.push(fold::count(t, groups.len() as u64, "group", "groups"));
    v.push(fold::count(t, similar, "similar", "similar"));
    v.push(fold::figure(
        t,
        &fmt_bytes(groups.iter().map(VideoGroup::reclaimable).sum()),
    ));
    let mark = if warned.is_empty() {
        Mark::Done
    } else {
        Mark::Warn
    };
    v.extend(warned);
    let words = groups.iter().flat_map(|g| {
        let k = &g.members[0].hashed;
        g.members[1..]
            .iter()
            .filter_map(move |m| m.relation.map(|r| r.words(&k.sig, &m.hashed.sig)))
    });
    v.extend(fold::tally_segments(t, fold::tally(words)));
    if s.unreadable > stalled {
        v.push(fold::dim(
            t,
            &format!("{} unreadable", fmt_count(s.unreadable - stalled)),
        ));
    }
    (mark, v)
}

/// What the run can give back, for the yield: each kind's bytes, the
/// files they are, and every group with its keeper's name.
fn yield_of(f: &Found) -> Yield {
    let name = |p: &Path| {
        p.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    let mut groups: Vec<(Hue, u64, String)> = Vec::new();
    let mut files = 0u64;
    for g in &f.groups {
        groups.push((Hue::Copies, g.reclaimable(), name(&g.members[0].path)));
        files += g.members.len() as u64 - 1;
    }
    for g in &f.near {
        groups.push((
            Hue::Similar,
            g.reclaimable(),
            name(&g.members[0].hashed.rep.cand.path),
        ));
        files += g.members[1..]
            .iter()
            .filter(|m| m.relation.is_some_and(|r| r.confirmed()))
            .count() as u64;
    }
    for g in &f.video {
        groups.push((
            Hue::Video,
            g.reclaimable(),
            name(&g.members[0].hashed.rep.cand.path),
        ));
        files += g.members[1..]
            .iter()
            .filter(|m| m.relation.is_some_and(|r| r.confirmed()))
            .count() as u64;
    }
    let part = |h: Hue| groups.iter().filter(|g| g.0 == h).map(|g| g.1).sum::<u64>();
    Yield {
        scanned: f.scan.bytes,
        files_scanned: f.scan.files,
        parts: vec![
            (Hue::Copies, part(Hue::Copies)),
            (Hue::Similar, part(Hue::Similar)),
            (Hue::Video, part(Hue::Video)),
        ],
        files,
        groups,
    }
}
