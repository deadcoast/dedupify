//! Comparing two videos and grouping them: the relation's evidence, the
//! keeper order, and the pass that welds the groups.

use std::sync::atomic::{AtomicU64, Ordering};

use rayon::prelude::*;

use crate::exact::{Progress, Stage};
use crate::pdq::{self, PDQ_MAX};
use crate::perceptual::{self, Signature, Variant, DHASH_MAX, PHASH_MAX, VARIANT_ORDER};

use super::{
    level1, level2, VideoSig, VideoStats, LEVEL1_MIN, LEVEL2_CONFIRM, LEVEL2_MIN, POSITIONS_MIN,
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VideoRelation {
    /// Positional medians over the shared positions on the modal
    /// orientation; `None` when fewer than `POSITIONS_MIN` positions exist.
    pub pos: Option<(u32, u32, u32)>,
    pub variant: Variant,
    pub level1: f32,
    pub level2: f32,
    /// Best time offset from level 2, seconds.
    pub offset: f32,
}

impl VideoRelation {
    /// Grouping rule for `find`/`milk`: positional agreement on the two
    /// 64-bit hashes, or the TMK two-phase rule.
    pub fn similar(&self) -> bool {
        self.positional_ok() || self.tmk_ok()
    }

    pub fn positional_ok(&self) -> bool {
        matches!(self.pos, Some((d, p, _)) if d <= DHASH_MAX && p <= PHASH_MAX)
    }

    pub fn tmk_ok(&self) -> bool {
        self.level1 >= LEVEL1_MIN && self.level2 >= LEVEL2_MIN
    }

    /// `purge` rule: TMK level 2 at the stricter bar, or positional
    /// agreement on all three hashes including PDQ. The positional test
    /// needs the lengths to line up, the TMK one does not.
    pub fn confirmed(&self) -> bool {
        (self.level1 >= LEVEL1_MIN && self.level2 >= LEVEL2_CONFIRM)
            || matches!(self.pos, Some((d, p, q)) if d <= DHASH_MAX && p <= PHASH_MAX && q <= PDQ_MAX)
    }

    /// What the member is, in words: `re-encoded`, `scaled 768→480`,
    /// `trimmed 0:30 of 1:20`, `pillarboxed`, `letterboxed`, `mirrored`,
    /// `similar, unconfirmed`; two words at most. [CLAUDE]
    pub fn words(&self, keeper: &VideoSig, member: &VideoSig) -> String {
        if !self.confirmed() {
            return "similar, unconfirmed".to_string();
        }
        let mut parts: Vec<String> = Vec::new();
        // bars one side has and the other has not
        match (member.bars, keeper.bars) {
            (Some(b), None) => parts.push(
                if b.width < member.meta.width {
                    "pillarboxed"
                } else {
                    "letterboxed"
                }
                .to_string(),
            ),
            (None, Some(_)) => parts.push("bars removed".to_string()),
            _ => {}
        }
        // orientation, from the sampled frames
        if self.positional_ok() {
            let o = match self.variant {
                Variant::Identity => "",
                Variant::Rotate90 => "rotated 90°",
                Variant::Rotate180 => "rotated 180°",
                Variant::Rotate270 => "rotated 270°",
                Variant::FlipH => "mirrored",
                Variant::FlipV => "flipped",
                Variant::Transpose | Variant::AntiTranspose => "rotated and mirrored",
            };
            if !o.is_empty() {
                parts.push(o.to_string());
            }
        }
        // length
        let (kd, md) = (keeper.meta.duration, member.meta.duration);
        if kd > 0.0 && (md - kd).abs() / kd > 0.02 {
            parts.push(if md < kd {
                format!(
                    "trimmed {} of {}",
                    crate::util::fmt_clock(md),
                    crate::util::fmt_clock(kd)
                )
            } else {
                format!(
                    "longer, {} of {}",
                    crate::util::fmt_clock(kd),
                    crate::util::fmt_clock(md)
                )
            });
        }
        // picture size, inside any bars
        let pic = |s: &VideoSig| s.bars.map_or(s.meta.width, |b| b.width);
        let (kw, mw) = (pic(keeper), pic(member));
        if kw != mw && (kw as f32 - mw as f32).abs() / kw.max(1) as f32 > 0.03 {
            parts.push(format!("scaled {kw}\u{2192}{mw}"));
        }
        if parts.is_empty() {
            parts.push("re-encoded".to_string());
        }
        parts.truncate(2);
        parts.join(", ")
    }

    /// The evidence, for the dim trailing column.
    pub fn numbers(&self) -> String {
        let tmk = if self.level1 >= LEVEL1_MIN {
            format!("tmk {:.2}", self.level2)
        } else {
            "tmk    -".to_string()
        };
        match self.pos {
            Some((d, p, q)) => format!("d {d:>2} p {p:>2}  q {q:>3}  {tmk}"),
            None => format!("frames     -  {tmk}"),
        }
    }

    pub fn unconfirmed_reason(&self) -> String {
        let tmk = format!("TMK {:.2} under {LEVEL2_CONFIRM:.1}", self.level2);
        match self.pos {
            Some((d, p, q)) if d <= DHASH_MAX && p <= PHASH_MAX && q > PDQ_MAX => {
                format!("frames q {q} over {PDQ_MAX}; {tmk}")
            }
            _ => tmk,
        }
    }
}

fn hamming_pair(k: &Signature, m: &Signature, vi: usize) -> (u32, u32, u32) {
    let d1 = perceptual::hamming(k.dhash[0], m.dhash[vi]);
    let p1 = perceptual::hamming(k.phash[0], m.phash[vi]);
    let q1 = pdq::distance(&k.pdq[0], &m.pdq[vi]);
    let d2 = perceptual::hamming(m.dhash[0], k.dhash[vi]);
    let p2 = perceptual::hamming(m.phash[0], k.phash[vi]);
    let q2 = pdq::distance(&m.pdq[0], &k.pdq[vi]);
    if d1 + p1 <= d2 + p2 {
        (d1, p1, q1)
    } else {
        (d2, p2, q2)
    }
}

fn median(v: &mut [u32]) -> u32 {
    v.sort_unstable();
    v[v.len() / 2]
}

pub fn relate(a: &VideoSig, b: &VideoSig) -> VideoRelation {
    let n = a.positional.len().min(b.positional.len());
    let mut variant = Variant::Identity;
    let mut pos = None;
    if n >= POSITIONS_MIN {
        // modal best orientation over positions
        let mut votes = [0u32; perceptual::VARIANTS];
        for i in 0..n {
            let mut best = (u32::MAX, 0usize);
            for vi in 0..perceptual::VARIANTS {
                let (d, p, _) = hamming_pair(&a.positional[i], &b.positional[i], vi);
                if d + p < best.0 {
                    best = (d + p, vi);
                }
            }
            votes[best.1] += 1;
        }
        let vi = votes
            .iter()
            .enumerate()
            .max_by_key(|(_, v)| **v)
            .map(|(i, _)| i)
            .unwrap_or(0);
        variant = VARIANT_ORDER[vi];
        let mut ds = Vec::with_capacity(n);
        let mut ps = Vec::with_capacity(n);
        let mut qs = Vec::with_capacity(n);
        for i in 0..n {
            let (d, p, q) = hamming_pair(&a.positional[i], &b.positional[i], vi);
            ds.push(d);
            ps.push(p);
            qs.push(q);
        }
        pos = Some((median(&mut ds), median(&mut ps), median(&mut qs)));
    }
    let l1 = level1(&a.tmk, &b.tmk);
    let (l2, offset) = if l1 >= LEVEL1_MIN {
        level2(&a.tmk, &b.tmk)
    } else {
        (0.0, 0.0)
    };
    VideoRelation {
        pos,
        variant,
        level1: l1,
        level2: l2,
        offset,
    }
}

// ---------------------------------------------------------------------------
// the pass

/// One video entering the pass (same shape as `near::Rep`).
#[derive(Debug, Clone)]
pub struct VRep {
    pub cand: crate::walk::Candidate,
    pub exact_group: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct VHashed {
    pub rep: VRep,
    pub sig: VideoSig,
}

#[derive(Debug, Clone)]
pub struct VideoMember {
    pub hashed: VHashed,
    /// Relation to the keeper; `None` for the keeper.
    pub relation: Option<VideoRelation>,
}

#[derive(Debug, Clone)]
pub struct VideoGroup {
    pub members: Vec<VideoMember>,
}

impl VideoGroup {
    /// Make member `idx` the keeper and relate the others to it. [CLAUDE]
    pub fn set_keeper(&mut self, idx: usize) {
        if idx == 0 || idx >= self.members.len() {
            return;
        }
        let chosen = self.members.remove(idx);
        self.members.insert(0, chosen);
        self.members[0].relation = None;
        let keeper = self.members[0].hashed.sig.clone();
        for m in self.members.iter_mut().skip(1) {
            m.relation = Some(relate(&keeper, &m.hashed.sig));
        }
    }

    /// Bytes `milk`/`purge` act on: confirmed members only, as for near
    /// groups. (Through Phase 8 this summed every member, so a trim the
    /// rule leaves in place was counted as reclaimable.)
    pub fn reclaimable(&self) -> u64 {
        self.members
            .iter()
            .skip(1)
            .filter(|m| m.relation.is_some_and(|r| r.confirmed()))
            .map(|m| m.hashed.rep.cand.size)
            .sum()
    }
}

pub const VIDEO_POLICY_LABEL: &str = "keep: longest, then most pixels";

/// Pixels of the picture itself: inside the bars when there are any, so a
/// padded export never outranks the picture it was padded from.
fn picture_pixels(v: &VHashed) -> u64 {
    match v.sig.bars {
        Some(b) => b.width as u64 * b.height as u64,
        None => v.sig.meta.width as u64 * v.sig.meta.height as u64,
    }
}

/// Keeper order: longest duration (a trim is the derivative), then most
/// picture pixels, then the file without bars (the padding is the
/// derivative), then highest bitrate, then the one not named or filed as
/// a copy (`keeper::derived`), then oldest, then path.
pub fn keeper_order(a: &VHashed, b: &VHashed) -> std::cmp::Ordering {
    let da = (a.sig.meta.duration * 10.0).round() as i64;
    let db = (b.sig.meta.duration * 10.0).round() as i64;
    let pa = picture_pixels(a);
    let pb = picture_pixels(b);
    let bra = if a.sig.meta.duration > 0.0 {
        a.rep.cand.size as f64 / a.sig.meta.duration
    } else {
        0.0
    };
    let brb = if b.sig.meta.duration > 0.0 {
        b.rep.cand.size as f64 / b.sig.meta.duration
    } else {
        0.0
    };
    db.cmp(&da)
        .then_with(|| pb.cmp(&pa))
        .then_with(|| a.sig.bars.is_some().cmp(&b.sig.bars.is_some()))
        .then_with(|| brb.partial_cmp(&bra).unwrap_or(std::cmp::Ordering::Equal))
        .then_with(|| {
            crate::keeper::derived(&a.rep.cand.path).cmp(&crate::keeper::derived(&b.rep.cand.path))
        })
        .then_with(|| a.rep.cand.mtime.cmp(&b.rep.cand.mtime))
        .then_with(|| a.rep.cand.path.cmp(&b.rep.cand.path))
}

/// All pairs (level 1 gates level 2), union–find, keeper order.
pub fn group(
    hashed: Vec<VHashed>,
    stats: &mut VideoStats,
    progress: &dyn Progress,
) -> Vec<VideoGroup> {
    let n = hashed.len();
    progress.tick(Stage::Match, 0, n as u64);
    let done = AtomicU64::new(0);
    let pairs: Vec<(usize, usize)> = (0..n)
        .into_par_iter()
        .flat_map_iter(|i| {
            let mut v = Vec::new();
            for j in i + 1..n {
                if relate(&hashed[i].sig, &hashed[j].sig).similar() {
                    v.push((i, j));
                }
            }
            let d = done.fetch_add(1, Ordering::Relaxed) + 1;
            progress.tick(Stage::Match, d, n as u64);
            v.into_iter()
        })
        .collect();
    stats.pairs = pairs.len() as u64;

    let mut uf = crate::near::UnionFind::new(n);
    for (a, b) in &pairs {
        uf.union(*a, *b);
    }
    let mut buckets: std::collections::HashMap<usize, Vec<usize>> =
        std::collections::HashMap::new();
    for i in 0..n {
        let r = uf.find(i);
        buckets.entry(r).or_default().push(i);
    }
    let mut slots: Vec<Option<VHashed>> = hashed.into_iter().map(Some).collect();
    let mut groups: Vec<VideoGroup> = Vec::new();
    for (_, mut idxs) in buckets {
        if idxs.len() < 2 {
            continue;
        }
        let mut members: Vec<VHashed> = idxs
            .drain(..)
            .map(|i| {
                slots[i]
                    .take()
                    .expect("invariant: each hashed video lands in one group")
            })
            .collect();
        members.sort_by(keeper_order);
        let keeper = members[0].sig.clone();
        let members: Vec<VideoMember> = members
            .into_iter()
            .enumerate()
            .map(|(i, h)| {
                let relation = if i == 0 {
                    None
                } else {
                    Some(relate(&keeper, &h.sig))
                };
                VideoMember {
                    hashed: h,
                    relation,
                }
            })
            .collect();
        stats.grouped += members.len() as u64;
        groups.push(VideoGroup { members });
    }
    groups.sort_by(|a, b| {
        b.reclaimable().cmp(&a.reclaimable()).then_with(|| {
            a.members[0]
                .hashed
                .rep
                .cand
                .path
                .cmp(&b.members[0].hashed.rep.cand.path)
        })
    });
    groups
}
