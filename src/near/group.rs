//! Matching, clustering and ordering: the hashes' graph, the deep tier's
//! attachments, the fit where it is needed. (Split from `near.rs`, Phase
//! 14, SC-09.)

use super::*;

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
        // a single's best keeper in the band under DEEP_MIN, tried only
        // after the pairs at DEEP_MIN: an attachment the fit may still
        // drop must not take a single from a pair that lists (Phase 11)
        let mut band_best: Vec<(f32, usize, usize)> = Vec::new(); // (cosine, single, comp)
        for &s_ in &singles {
            let mut best: Option<(usize, f32)> = None;
            for (ci, c) in comps.iter().enumerate() {
                if let Some(cs) = cos_to(s_, c[0]) {
                    if cs >= crate::deep::DEEP_FIT_MIN && best.is_none_or(|(_, b)| cs > b) {
                        best = Some((ci, cs));
                    }
                }
            }
            match best {
                Some((ci, cs)) if cs >= crate::deep::DEEP_MIN => deep_attached.push((ci, s_, cs)),
                Some((ci, cs)) => {
                    band_best.push((cs, s_, ci));
                    leftover.push(s_);
                }
                None => leftover.push(s_),
            }
        }
        // leftovers pair among themselves, star-shaped around the better
        // keeper; then, in the band, a pair or a keeper, best cosine first
        enum Join {
            Pair(usize, usize),
            Keeper(usize, usize), // (single, comp)
        }
        let mut cand: Vec<(f32, Join)> = Vec::new();
        for i in 0..leftover.len() {
            for j in i + 1..leftover.len() {
                if let Some(cs) = cos_to(leftover[i], leftover[j]) {
                    if cs >= crate::deep::DEEP_FIT_MIN {
                        cand.push((cs, Join::Pair(leftover[i], leftover[j])));
                    }
                }
            }
        }
        cand.extend(
            band_best
                .into_iter()
                .map(|(cs, s_, ci)| (cs, Join::Keeper(s_, ci))),
        );
        // at DEEP_MIN pairs only (no keeper is there); under it both kinds
        cand.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        let mut placed: std::collections::HashMap<usize, usize> = std::collections::HashMap::new(); // member → comp
        for (cs, join) in cand {
            let (a, b) = match join {
                Join::Keeper(s_, ci) => {
                    if let std::collections::hash_map::Entry::Vacant(e) = placed.entry(s_) {
                        e.insert(ci);
                        deep_attached.push((ci, s_, cs));
                    }
                    continue;
                }
                Join::Pair(a, b) => (a, b),
            };
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
    // members the descriptor attached (not the hashes), by path
    let mut by_deep: std::collections::HashSet<std::path::PathBuf> = Default::default();
    for (ci, idxs) in comps.iter().enumerate() {
        let keeper = slots[idxs[0]]
            .take()
            .expect("invariant: each hashed file lands in one group");
        let mut members: Vec<NearMember> = vec![NearMember {
            hashed: keeper,
            relation: None,
        }];
        for &i in &idxs[1..] {
            let h = slots[i]
                .take()
                .expect("invariant: each hashed file lands in one group");
            let relation = Some(relation_of(&members[0].hashed, &h));
            members.push(NearMember {
                hashed: h,
                relation,
            });
        }
        for &(aci, m, _) in &deep_attached {
            if aci == ci {
                let h = slots[m]
                    .take()
                    .expect("invariant: each hashed file lands in one group");
                by_deep.insert(h.rep.cand.path.clone());
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
                    let needs = !r.kin
                        && ((r.hash_pair() && !r.pdq_ok())
                            || r.deep_only()
                            || (!r.hash_pair() && r.deep_band()));
                    needs.then_some((gi, mi))
                })
        })
        .collect();
    stats.fits = wanted.len() as u64;
    // one thumbnail per keeper, shared by its members' fits (a keeper with
    // twelve copies was decoded twelve times), small-decoded (Phase 14)
    let mut by_group: Vec<(usize, Vec<usize>)> = Vec::new();
    for (gi, mi) in wanted {
        match by_group.last_mut() {
            Some((g, ms)) if *g == gi => ms.push(mi),
            _ => by_group.push((gi, vec![mi])),
        }
    }
    let fits: Vec<((usize, usize), Option<Fit>)> = by_group
        .into_par_iter()
        .flat_map(|(gi, mis)| {
            let keeper = geometry::thumb_of(&groups[gi].members[0].hashed.rep.cand.path);
            mis.into_par_iter()
                .map(|mi| {
                    let m = &groups[gi].members[mi].hashed.rep.cand.path;
                    let f = keeper
                        .as_ref()
                        .and_then(|k| Some(geometry::fit(k, &geometry::thumb_of(m)?)));
                    ((gi, mi), f)
                })
                .collect::<Vec<_>>()
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
    // members the descriptor brought in under DEEP_MIN stay only when the
    // fit confirmed them; a group left with its keeper alone goes too
    for g in groups.iter_mut() {
        let before = g.members.len();
        let keeper = g.members.remove(0);
        g.members.retain(|m| {
            !by_deep.contains(&m.hashed.rep.cand.path)
                || m.relation.is_none_or(|r| !r.deep_band() || r.fit_ok())
        });
        g.members.insert(0, keeper);
        let dropped = before - g.members.len();
        stats.deep_pairs = stats.deep_pairs.saturating_sub(dropped as u64);
        stats.grouped = stats
            .grouped
            .saturating_sub(dropped as u64 + u64::from(g.members.len() < 2));
    }
    groups.retain(|g| g.members.len() >= 2);
    stats.kin = groups.iter().map(|g| g.kin_count() as u64).sum();
    stats.raw_pairs = groups.iter().map(|g| g.kinds().pair as u64).sum();
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
