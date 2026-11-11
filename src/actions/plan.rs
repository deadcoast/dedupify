//! Which files an action may take, and why: the program's rule
//! (`default_items`), a person's choice out of `items`, and the byte
//! check `purge` makes before it bins anything.

use std::path::Path;

use rayon::prelude::*;

use crate::exact::{self, PipelineStats, Progress, Stage};
use crate::render::{LiveProgress, Renderer};

use super::Found;

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
    /// The camera's JPEG beside its RAW: left with it unless chosen by
    /// hand, and counted apart from the unconfirmed. [CLAUDE, Phase 10]
    pub(crate) pair: bool,
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
                pair: false,
            });
        }
    }
    for g in &f.near {
        let k = &g.members[0].hashed;
        for m in &g.members[1..] {
            let rel = m
                .relation
                .expect("invariant: every member after the keeper is related to it");
            if rel.kin || rel.raw {
                // another capture moment: another photograph; or a camera
                // RAW: never moved or binned, by the program or by hand
                // (RESEARCH §2.8, §2.10)
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
                pair: rel.pair,
            });
        }
    }
    for g in &f.video {
        let k = &g.members[0].hashed;
        for m in &g.members[1..] {
            let rel = m
                .relation
                .expect("invariant: every member after the keeper is related to it");
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
                pair: false,
            });
        }
    }
    v
}

/// The other shots of the near groups — another capture moment, or another
/// frame of the same second — for a person culling a burst in the
/// workspace (`c`, NF-05). They are never in `items`, so no command and no
/// rule of the program ever takes one; only this person's choice does,
/// and each is printed `by hand`.
pub(crate) fn shot_items(f: &Found) -> Vec<Item<'_>> {
    let mut v = Vec::new();
    for g in &f.near {
        let k = &g.members[0].hashed;
        for m in &g.members[1..] {
            let Some(rel) = m.relation else { continue };
            if !rel.kin || rel.raw || rel.pair {
                continue;
            }
            v.push(Item {
                path: &m.hashed.rep.cand.path,
                size: m.hashed.rep.cand.size,
                keeper: &k.rep.cand.path,
                near: Some(format!("{}  {}", rel.words(k, &m.hashed), rel.numbers())),
                near_confirmed: Some(false),
                near_reason: rel.unconfirmed_reason(),
                hash: format!(
                    "near:{:016x}:{}",
                    m.hashed.sig.dhash[0],
                    crate::pdq::hex(&m.hashed.sig.pdq[0])
                ),
                listed: true,
                by_hand: true,
                pair: false,
            });
        }
    }
    v
}

/// The items the commands act on without a person in the loop: everything
/// but the descriptor-only listings (shown on the report, never acted on).
/// Unconfirmed near duplicates stay in the list so the action prints them
/// as `kept~` with their reason.
pub(crate) fn default_items(f: &Found) -> Vec<Item<'_>> {
    items(f).into_iter().filter(|it| !it.listed).collect()
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
