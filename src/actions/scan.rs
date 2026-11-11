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
use crate::render::{LiveProgress, Renderer};
use crate::theme::Role;
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
}
