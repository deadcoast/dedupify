//! Near-duplicate image pass: representatives → signatures → matched pairs
//! → connected components → ordered groups.
//!
//! Exact duplicates are resolved before this runs, so each exact group is
//! represented once (by its keeper); the near pass never re-reports a
//! byte-identical copy as "similar".

use std::sync::atomic::{AtomicU64, Ordering};

use rayon::prelude::*;

use crate::exact::{Progress, Stage};
use crate::hamming::{self, Match, MatchProgress};
use crate::pdq::{distance as pdq_distance, PDQ_MAX, PDQ_QUALITY_MIN};
use crate::perceptual::{self, Signature, Variant, DHASH_MAX, PHASH_MAX, QUALITY_MIN};
use crate::walk::Candidate;

/// One image entering the near pass.
#[derive(Debug, Clone)]
pub struct Rep {
    pub cand: Candidate,
    /// Index of the exact group this file is the keeper of, if any.
    pub exact_group: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct Hashed {
    pub rep: Rep,
    pub sig: Signature,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Relation {
    pub variant: Variant,
    pub d: u32,
    pub p: u32,
    /// PDQ distance to the keeper on the same orientation (0–256).
    pub q: u32,
    /// PDQ quality of the lower-quality side; below `PDQ_QUALITY_MIN` the
    /// PDQ distance carries no weight.
    pub q_quality: u8,
}

impl Relation {
    /// The confirmation `purge` requires before binning a near duplicate:
    /// both 64-bit hashes within threshold **and** PDQ within threshold on
    /// a hash PDQ itself rates reliable.
    pub fn confirmed(&self) -> bool {
        self.d <= DHASH_MAX
            && self.p <= PHASH_MAX
            && self.q <= PDQ_MAX
            && self.q_quality >= PDQ_QUALITY_MIN
    }

    /// Why `confirmed()` is false, for the `kept` line.
    pub fn unconfirmed_reason(&self) -> String {
        if self.q_quality < PDQ_QUALITY_MIN {
            format!("PDQ quality {} below {}", self.q_quality, PDQ_QUALITY_MIN)
        } else if self.q > PDQ_MAX {
            format!("PDQ q {} over {}", self.q, PDQ_MAX)
        } else {
            format!("d {} p {} over threshold to keeper", self.d, self.p)
        }
    }
}

#[derive(Debug, Clone)]
pub struct NearMember {
    pub hashed: Hashed,
    /// Distances to the keeper; `None` for the keeper itself.
    pub relation: Option<Relation>,
}

#[derive(Debug, Clone)]
pub struct NearGroup {
    /// Keeper first.
    pub members: Vec<NearMember>,
}

impl NearGroup {
    pub fn reclaimable(&self) -> u64 {
        self.members
            .iter()
            .skip(1)
            .map(|m| m.hashed.rep.cand.size)
            .sum()
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct NearStats {
    pub decoded: u64,
    pub undecodable: u64,
    pub low_quality: u64,
    pub pairs: u64,
    pub grouped: u64,
}

// ---------------------------------------------------------------------------
// union–find
// ---------------------------------------------------------------------------

pub struct UnionFind {
    parent: Vec<usize>,
    rank: Vec<u8>,
}

impl UnionFind {
    pub fn new(n: usize) -> UnionFind {
        UnionFind {
            parent: (0..n).collect(),
            rank: vec![0; n],
        }
    }

    pub fn find(&mut self, mut x: usize) -> usize {
        while self.parent[x] != x {
            self.parent[x] = self.parent[self.parent[x]]; // path halving
            x = self.parent[x];
        }
        x
    }

    pub fn union(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra == rb {
            return;
        }
        match self.rank[ra].cmp(&self.rank[rb]) {
            std::cmp::Ordering::Less => self.parent[ra] = rb,
            std::cmp::Ordering::Greater => self.parent[rb] = ra,
            std::cmp::Ordering::Equal => {
                self.parent[rb] = ra;
                self.rank[ra] += 1;
            }
        }
    }
}

// ---------------------------------------------------------------------------

struct MatchTicker<'a> {
    inner: &'a dyn Progress,
}
impl MatchProgress for MatchTicker<'_> {
    fn tick(&self, done: u64, total: u64) {
        self.inner.tick(Stage::Match, done, total);
    }
}

/// Decode and hash every representative in parallel.
pub fn hash_all(reps: Vec<Rep>, progress: &dyn Progress) -> (Vec<Hashed>, NearStats) {
    let total = reps.len() as u64;
    let done = AtomicU64::new(0);
    progress.tick(Stage::Decode, 0, total);
    let results: Vec<(Rep, Result<Signature, perceptual::DecodeError>)> = reps
        .into_par_iter()
        .map(|rep| {
            let sig = perceptual::signature(&rep.cand.path);
            let d = done.fetch_add(1, Ordering::Relaxed) + 1;
            progress.tick(Stage::Decode, d, total);
            (rep, sig)
        })
        .collect();
    let mut stats = NearStats::default();
    let mut out = Vec::with_capacity(results.len());
    for (rep, sig) in results {
        match sig {
            Ok(sig) => {
                stats.decoded += 1;
                if sig.quality < QUALITY_MIN {
                    stats.low_quality += 1;
                } else {
                    out.push(Hashed { rep, sig });
                }
            }
            Err(_) => stats.undecodable += 1,
        }
    }
    (out, stats)
}

/// Keeper order: most pixels (a downscaled copy is the derivative), then
/// oldest (the original predates its exports), then largest file (at equal
/// age and size a lossless encoding outweighs a lossy one), then path.
pub fn keeper_order(a: &Hashed, b: &Hashed) -> std::cmp::Ordering {
    b.sig
        .pixels()
        .cmp(&a.sig.pixels())
        .then_with(|| a.rep.cand.mtime.cmp(&b.rep.cand.mtime))
        .then_with(|| b.rep.cand.size.cmp(&a.rep.cand.size))
        .then_with(|| a.rep.cand.path.cmp(&b.rep.cand.path))
}

pub const NEAR_POLICY_LABEL: &str = "keep: most pixels";

/// Best relation of `m` to the keeper `k`, searching both directions.
pub fn relation(k: &Signature, m: &Signature) -> Relation {
    let mut best: Option<(Variant, u32, u32, u32)> = None;
    for (i, v) in perceptual::VARIANT_ORDER.iter().enumerate() {
        let d1 = perceptual::hamming(k.dhash[0], m.dhash[i]);
        let p1 = perceptual::hamming(k.phash[0], m.phash[i]);
        let q1 = pdq_distance(&k.pdq[0], &m.pdq[i]);
        let d2 = perceptual::hamming(m.dhash[0], k.dhash[i]);
        let p2 = perceptual::hamming(m.phash[0], k.phash[i]);
        let q2 = pdq_distance(&m.pdq[0], &k.pdq[i]);
        for (d, p, q) in [(d1, p1, q1), (d2, p2, q2)] {
            if best.is_none_or(|(_, bd, bp, _)| d + p < bd + bp) {
                best = Some((*v, d, p, q));
            }
        }
    }
    let (variant, d, p, q) = best.unwrap();
    Relation {
        variant,
        d,
        p,
        q,
        q_quality: k.pdq_quality.min(m.pdq_quality),
    }
}

/// Match, cluster, order. Returns groups largest-reclaim first.
pub fn group(
    hashed: Vec<Hashed>,
    stats: &mut NearStats,
    progress: &dyn Progress,
) -> Vec<NearGroup> {
    let sigs: Vec<Signature> = hashed.iter().map(|h| h.sig.clone()).collect();
    let n = sigs.len();
    progress.tick(Stage::Match, 0, n as u64);
    let pairs: Vec<Match> = hamming::match_pairs(
        &sigs,
        DHASH_MAX,
        PHASH_MAX,
        &MatchTicker { inner: progress },
    );
    stats.pairs = pairs.len() as u64;

    let mut uf = UnionFind::new(n);
    for m in &pairs {
        uf.union(m.a, m.b);
    }
    let mut buckets: std::collections::HashMap<usize, Vec<usize>> =
        std::collections::HashMap::new();
    for i in 0..n {
        let r = uf.find(i);
        buckets.entry(r).or_default().push(i);
    }

    let mut hashed: Vec<Option<Hashed>> = hashed.into_iter().map(Some).collect();
    let mut groups: Vec<NearGroup> = Vec::new();
    for (_, mut idxs) in buckets {
        if idxs.len() < 2 {
            continue;
        }
        let mut members: Vec<Hashed> = idxs.drain(..).map(|i| hashed[i].take().unwrap()).collect();
        members.sort_by(keeper_order);
        let keeper_sig = members[0].sig.clone();
        let members: Vec<NearMember> = members
            .into_iter()
            .enumerate()
            .map(|(i, h)| {
                let relation = if i == 0 {
                    None
                } else {
                    Some(relation(&keeper_sig, &h.sig))
                };
                NearMember {
                    hashed: h,
                    relation,
                }
            })
            .collect();
        stats.grouped += members.len() as u64;
        groups.push(NearGroup { members });
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn union_find_components() {
        let mut uf = UnionFind::new(6);
        uf.union(0, 1);
        uf.union(1, 2);
        uf.union(4, 5);
        assert_eq!(uf.find(0), uf.find(2));
        assert_ne!(uf.find(0), uf.find(3));
        assert_eq!(uf.find(4), uf.find(5));
        assert_ne!(uf.find(4), uf.find(2));
    }
}
