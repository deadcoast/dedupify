//! Near-duplicate image pass: representatives → signatures → matched pairs
//! → connected components → ordered groups.
//!
//! Exact duplicates are resolved before this runs, so each exact group is
//! represented once (by its keeper); the near pass never re-reports a
//! byte-identical copy as "similar".

use std::sync::atomic::{AtomicU64, Ordering};

use rayon::prelude::*;

use crate::capture::{self, Capture};
use crate::exact::{Progress, Stage};
use crate::geometry::{self, Fit, FIT_MIN};
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
    /// SSCD embedding when the deep tier is active.
    pub deep: Option<crate::deep::Embedding>,
    /// EXIF capture time, when the file states one.
    pub capture: Option<Capture>,
}

/// How a member relates to its keeper: the evidence, and what it allows.
///
/// Acting (moving or binning) needs two independent families to agree:
/// the two 64-bit hashes **and** either PDQ (frequency domain, 256 bits)
/// or the geometric fit (spatial correlation); or, without the hashes,
/// the learned descriptor **and** the fit. A member whose capture time
/// differs from the keeper's is a different photograph whatever the
/// pictures say: `kin`, listed, never acted on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Relation {
    pub variant: Variant,
    pub d: u32,
    pub p: u32,
    /// PDQ distance to the keeper on the same orientation (0–256).
    pub q: u32,
    /// PDQ quality of the lower-quality side; below `PDQ_QUALITY_MIN` the
    /// PDQ distance carries no weight.
    pub q_quality: u8,
    /// SSCD cosine to the keeper when the deep tier is active.
    pub deep: Option<f32>,
    /// Geometric fit to the keeper, computed when PDQ did not confirm or
    /// the member came in through the deep tier.
    pub fit: Option<Fit>,
    /// Both files state a capture time and they differ.
    pub kin: bool,
}

impl Relation {
    /// The two 64-bit hashes agree.
    pub fn hash_pair(&self) -> bool {
        self.d <= DHASH_MAX && self.p <= PHASH_MAX
    }

    /// PDQ within threshold on a hash PDQ itself rates reliable.
    pub fn pdq_ok(&self) -> bool {
        self.q <= PDQ_MAX && self.q_quality >= PDQ_QUALITY_MIN
    }

    pub fn fit_ok(&self) -> bool {
        self.fit.is_some_and(|f| f.ncc >= FIT_MIN)
    }

    /// Admitted by the learned descriptor.
    pub fn deep_in(&self) -> bool {
        self.deep.is_some_and(|c| c >= crate::deep::DEEP_MIN)
    }

    /// Grouped by the deep tier alone: the two 64-bit hashes disagree.
    pub fn deep_only(&self) -> bool {
        !self.hash_pair() && self.deep_in()
    }

    /// What `milk` and `purge` require before touching the file.
    pub fn confirmed(&self) -> bool {
        if self.kin {
            return false;
        }
        if self.hash_pair() {
            self.pdq_ok() || self.fit_ok()
        } else {
            self.deep_in() && self.fit_ok()
        }
    }

    /// Shown but never acted on, whatever else is measured: a different
    /// photograph (`kin`), or the learned descriptor's word alone.
    pub fn listed_only(&self) -> bool {
        self.kin || (self.deep_only() && !self.fit_ok())
    }

    /// The verb on the member's line.
    pub fn verb(&self) -> &'static str {
        if self.kin {
            "kin"
        } else if self.deep_only() && !self.fit_ok() {
            "deep"
        } else {
            "near"
        }
    }

    /// Why `confirmed()` is false, for the `kept~` line.
    pub fn unconfirmed_reason(&self) -> String {
        if self.kin {
            return "taken at a different moment".to_string();
        }
        let fit_txt = match self.fit {
            Some(f) => format!("fit {:.2} under {FIT_MIN:.2}", f.ncc.max(0.0)),
            None => "no fit".to_string(),
        };
        if self.deep_only() {
            return format!("deep {:.2} only; {fit_txt}", self.deep.unwrap_or(0.0));
        }
        if self.q_quality < PDQ_QUALITY_MIN {
            format!(
                "PDQ quality {} below {}; {fit_txt}",
                self.q_quality, PDQ_QUALITY_MIN
            )
        } else if self.q > PDQ_MAX {
            format!("PDQ q {} over {}; {fit_txt}", self.q, PDQ_MAX)
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
    /// Bytes `milk`/`purge` act on: confirmed members only.
    pub fn reclaimable(&self) -> u64 {
        self.members
            .iter()
            .skip(1)
            .filter(|m| m.relation.is_some_and(|r| r.confirmed()))
            .map(|m| m.hashed.rep.cand.size)
            .sum()
    }

    /// Members shown with the `deep` verb (learned descriptor alone).
    pub fn deep_only_count(&self) -> usize {
        self.members
            .iter()
            .filter(|m| m.relation.is_some_and(|r| r.deep_only() && !r.fit_ok()))
            .count()
    }

    /// Members shown with the `kin` verb (different capture moment).
    pub fn kin_count(&self) -> usize {
        self.members
            .iter()
            .filter(|m| m.relation.is_some_and(|r| r.kin))
            .count()
    }

    /// Members the hashes grouped that nothing confirmed: `kept~`.
    pub fn unconfirmed_count(&self) -> usize {
        self.members
            .iter()
            .skip(1)
            .filter(|m| {
                m.relation
                    .is_some_and(|r| !r.confirmed() && !r.listed_only())
            })
            .count()
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct NearStats {
    pub decoded: u64,
    pub undecodable: u64,
    pub low_quality: u64,
    pub pairs: u64,
    pub grouped: u64,
    /// Deep tier active for this run.
    pub deep: bool,
    pub deep_failed: u64,
    /// Pairs the deep tier added that the 64-bit hashes had not.
    pub deep_pairs: u64,
    /// Geometric fits computed (members PDQ did not confirm, deep members).
    pub fits: u64,
    /// Of those, fits at or above `FIT_MIN`.
    pub fit_confirmed: u64,
    /// Members set aside as `kin` by their capture time.
    pub kin: u64,
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
pub fn hash_all(
    reps: Vec<Rep>,
    progress: &dyn Progress,
    deep: Option<&crate::deep::Deep>,
) -> (Vec<Hashed>, NearStats) {
    let total = reps.len() as u64;
    let done = AtomicU64::new(0);
    let deep_failed = AtomicU64::new(0);
    progress.tick(Stage::Decode, 0, total);
    type Decoded = Result<
        (Signature, Option<crate::deep::Embedding>, Option<Capture>),
        perceptual::DecodeError,
    >;
    let results: Vec<(Rep, Decoded)> = reps
        .into_par_iter()
        .map(|rep| {
            // one decode feeds every hash and, when active, the embedding;
            // the capture time is a header read
            let r = perceptual::load_image(&rep.cand.path).map(|img| {
                let sig = perceptual::signature_from_image(&img);
                let emb = deep.and_then(|m| match m.embed(&img) {
                    Ok(e) => Some(e),
                    Err(_) => {
                        deep_failed.fetch_add(1, Ordering::Relaxed);
                        None
                    }
                });
                (sig, emb, capture::read(&rep.cand.path))
            });
            let d = done.fetch_add(1, Ordering::Relaxed) + 1;
            progress.tick(Stage::Decode, d, total);
            (rep, r)
        })
        .collect();
    let mut stats = NearStats {
        deep: deep.is_some(),
        ..Default::default()
    };
    stats.deep_failed = deep_failed.load(Ordering::Relaxed);
    let mut out = Vec::with_capacity(results.len());
    for (rep, r) in results {
        match r {
            Ok((sig, emb, capture)) => {
                stats.decoded += 1;
                if sig.quality < QUALITY_MIN {
                    stats.low_quality += 1;
                } else {
                    out.push(Hashed {
                        rep,
                        sig,
                        deep: emb,
                        capture,
                    });
                }
            }
            Err(_) => stats.undecodable += 1,
        }
    }
    (out, stats)
}

/// Keeper order inside one group: the member the most other members match
/// directly on the two 64-bit hashes (the hub — an original is what its
/// derivatives all resemble, derivatives need not resemble each other),
/// then most pixels (a downscaled copy is the derivative), then oldest, then
/// largest file (at equal age and size a lossless encoding outweighs a lossy
/// one), then path. Degree comes first so a padded or letterboxed copy with
/// more pixels than the original cannot take the keeper seat.
pub fn keeper_order_with_degree(
    a: &Hashed,
    da: usize,
    b: &Hashed,
    db: usize,
) -> std::cmp::Ordering {
    db.cmp(&da).then_with(|| keeper_order(a, b))
}

pub fn keeper_order(a: &Hashed, b: &Hashed) -> std::cmp::Ordering {
    b.sig
        .pixels()
        .cmp(&a.sig.pixels())
        .then_with(|| a.rep.cand.mtime.cmp(&b.rep.cand.mtime))
        .then_with(|| b.rep.cand.size.cmp(&a.rep.cand.size))
        .then_with(|| a.rep.cand.path.cmp(&b.rep.cand.path))
}

pub const NEAR_POLICY_LABEL: &str = "keep: shared moment, most matched, then most pixels";

/// Best relation of `m` to the keeper `k`, searching both directions.
pub fn relation(k: &Signature, m: &Signature) -> Relation {
    relation_deep(k, m, None, false)
}

/// The relation of two hashed files: distances, deep cosine, and the
/// capture-time verdict. The fit is added later, only where it is needed.
pub fn relation_of(k: &Hashed, m: &Hashed) -> Relation {
    let cos = match (k.deep.as_ref(), m.deep.as_ref()) {
        (Some(a), Some(b)) => Some(crate::deep::cosine(a, b)),
        _ => None,
    };
    let kin = capture::different_moments(k.capture.as_ref(), m.capture.as_ref()) == Some(true);
    relation_deep(&k.sig, &m.sig, cos, kin)
}

pub fn relation_deep(k: &Signature, m: &Signature, deep: Option<f32>, kin: bool) -> Relation {
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
        deep,
        fit: None,
        kin,
    }
}

/// Match, cluster, order. Returns groups largest-reclaim first.
///
/// The two 64-bit hashes build the graph (union–find over pairs). The deep
/// tier, when active, never adds edges to that graph: a deep match attaches
/// an image to an existing group's keeper, or pairs two leftover images,
/// always by its direct cosine to the keeper. That keeps one false deep
/// score from welding two families together, and makes the `deep` number
/// printed on a line the number that admitted it.
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
    let mut degree = vec![0usize; n];
    for m in &pairs {
        uf.union(m.a, m.b);
        degree[m.a] += 1;
        degree[m.b] += 1;
    }
    let mut buckets: std::collections::HashMap<usize, Vec<usize>> =
        std::collections::HashMap::new();
    for i in 0..n {
        let r = uf.find(i);
        buckets.entry(r).or_default().push(i);
    }

    // components of two or more: keeper by degree, then the usual order
    let mut comps: Vec<Vec<usize>> = Vec::new();
    let mut singles: Vec<usize> = Vec::new();
    for (_, idxs) in buckets {
        if idxs.len() >= 2 {
            comps.push(idxs);
        } else {
            singles.push(idxs[0]);
        }
    }
    for c in comps.iter_mut() {
        // the keeper is the exposure most of the component shares: the
        // member with the most known-same capture moments first, so a
        // burst frame that strayed in is the kin, not the family
        let same: std::collections::HashMap<usize, usize> = c
            .iter()
            .map(|&i| {
                let n = c
                    .iter()
                    .filter(|&&j| {
                        j != i
                            && capture::different_moments(
                                hashed[i].capture.as_ref(),
                                hashed[j].capture.as_ref(),
                            ) == Some(false)
                    })
                    .count();
                (i, n)
            })
            .collect();
        c.sort_by(|&a, &b| {
            same[&b].cmp(&same[&a]).then_with(|| {
                keeper_order_with_degree(&hashed[a], degree[a], &hashed[b], degree[b])
            })
        });
    }

    // deep attachments: singletons join the keeper they are closest to
    let mut deep_attached: Vec<(usize, usize, f32)> = Vec::new(); // (comp index, member, cosine)
    if stats.deep {
        let cos_to = |a: usize, b: usize| -> Option<f32> {
            match (hashed[a].deep.as_ref(), hashed[b].deep.as_ref()) {
                (Some(x), Some(y)) => Some(crate::deep::cosine(x, y)),
                _ => None,
            }
        };
        let mut leftover: Vec<usize> = Vec::new();
        for &s_ in &singles {
            let mut best: Option<(usize, f32)> = None;
            for (ci, c) in comps.iter().enumerate() {
                if let Some(cs) = cos_to(s_, c[0]) {
                    if cs >= crate::deep::DEEP_MIN && best.is_none_or(|(_, b)| cs > b) {
                        best = Some((ci, cs));
                    }
                }
            }
            match best {
                Some((ci, cs)) => deep_attached.push((ci, s_, cs)),
                None => leftover.push(s_),
            }
        }
        // leftovers pair among themselves, star-shaped around the better keeper
        let mut cand: Vec<(f32, usize, usize)> = Vec::new();
        for i in 0..leftover.len() {
            for j in i + 1..leftover.len() {
                if let Some(cs) = cos_to(leftover[i], leftover[j]) {
                    if cs >= crate::deep::DEEP_MIN {
                        cand.push((cs, leftover[i], leftover[j]));
                    }
                }
            }
        }
        cand.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        let mut placed: std::collections::HashMap<usize, usize> = std::collections::HashMap::new(); // member → comp
        for (cs, a, b) in cand {
            match (placed.get(&a).copied(), placed.get(&b).copied()) {
                (None, None) => {
                    let (k, m) = if keeper_order(&hashed[a], &hashed[b]) == std::cmp::Ordering::Less
                    {
                        (a, b)
                    } else {
                        (b, a)
                    };
                    comps.push(vec![k]);
                    let ci = comps.len() - 1;
                    placed.insert(k, ci);
                    placed.insert(m, ci);
                    deep_attached.push((ci, m, cs));
                }
                (Some(ci), None) if comps[ci][0] == a => {
                    placed.insert(b, ci);
                    deep_attached.push((ci, b, cs));
                }
                (None, Some(ci)) if comps[ci][0] == b => {
                    placed.insert(a, ci);
                    deep_attached.push((ci, a, cs));
                }
                _ => {} // both placed, or the placed one is not a keeper: no chaining
            }
        }
        stats.deep_pairs = deep_attached.len() as u64;
    }

    let mut slots: Vec<Option<Hashed>> = hashed.into_iter().map(Some).collect();
    let mut groups: Vec<NearGroup> = Vec::new();
    for (ci, idxs) in comps.iter().enumerate() {
        let keeper = slots[idxs[0]].take().unwrap();
        let mut members: Vec<NearMember> = vec![NearMember {
            hashed: keeper,
            relation: None,
        }];
        for &i in &idxs[1..] {
            let h = slots[i].take().unwrap();
            let relation = Some(relation_of(&members[0].hashed, &h));
            members.push(NearMember {
                hashed: h,
                relation,
            });
        }
        for &(aci, m, _) in &deep_attached {
            if aci == ci {
                let h = slots[m].take().unwrap();
                let relation = Some(relation_of(&members[0].hashed, &h));
                members.push(NearMember {
                    hashed: h,
                    relation,
                });
            }
        }
        if members.len() < 2 {
            continue;
        }
        stats.grouped += members.len() as u64;
        groups.push(NearGroup { members });
    }

    // the geometric fit, only where the cheaper evidence left a question:
    // hash-grouped members PDQ did not confirm, and deep-admitted members.
    // Kin members are decided already. Two decodes and a search per pair,
    // in parallel across pairs.
    let wanted: Vec<(usize, usize)> = groups
        .iter()
        .enumerate()
        .flat_map(|(gi, g)| {
            g.members
                .iter()
                .enumerate()
                .skip(1)
                .filter_map(move |(mi, m)| {
                    let r = m.relation?;
                    let needs = !r.kin && ((r.hash_pair() && !r.pdq_ok()) || r.deep_only());
                    needs.then_some((gi, mi))
                })
        })
        .collect();
    stats.fits = wanted.len() as u64;
    let fits: Vec<((usize, usize), Option<Fit>)> = wanted
        .into_par_iter()
        .map(|(gi, mi)| {
            let k = &groups[gi].members[0].hashed.rep.cand.path;
            let m = &groups[gi].members[mi].hashed.rep.cand.path;
            ((gi, mi), geometry::fit_files(k, m))
        })
        .collect();
    for ((gi, mi), f) in fits {
        if let Some(r) = groups[gi].members[mi].relation.as_mut() {
            r.fit = f;
            if r.fit_ok() {
                stats.fit_confirmed += 1;
            }
        }
    }
    stats.kin = groups.iter().map(|g| g.kin_count() as u64).sum();
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
