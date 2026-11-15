//! Hamming-space search over 64-bit codes (docs/RESEARCH.md §2.3).
//!
//! Two engines behind one function: a parallel linear scan for small sets
//! and Multi-Index Hashing (Norouzi, Punjani, Fleet) above
//! `LINEAR_MAX`. Both return the same pairs; the MIH engine is tested
//! against the scan on random codes.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use rayon::prelude::*;

use crate::pdq::{distance as pdq_distance, Hash256, PDQ_MAX, PDQ_QUALITY_MIN};
use crate::perceptual::{hamming, Signature, Variant, VARIANTS, VARIANT_ORDER};

/// Above this many signatures the index is used instead of the scan.
/// Measured on this build's 2-core runtime (docs/VERIFICATION.md): the
/// scan and the index cross at roughly 4,000 signatures; the scan grows as
/// n², the index close to n.
pub const LINEAR_MAX: usize = 4_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Match {
    /// Indices into the signature slice, `a < b`.
    pub a: usize,
    pub b: usize,
    /// Which variant of `b` matched the identity of `a` (or the inverse
    /// direction; `flipped` says which).
    pub variant: Variant,
    pub flipped: bool,
    pub d: u32,
    pub p: u32,
    /// PDQ distance on the same variant (0–256).
    pub q: u32,
}

/// Progress sink for the match stage.
pub trait MatchProgress: Sync {
    fn tick(&self, done: u64, total: u64);
}
pub struct NoMatchProgress;
impl MatchProgress for NoMatchProgress {
    fn tick(&self, _: u64, _: u64) {}
}

/// How a variant comparison was admitted, for ranking: the two 64-bit
/// hashes within their thresholds first (by d + p), then PDQ alone within
/// `PDQ_MAX` on hashes both sides rate reliable (by q). Phase 19: on 120
/// resized and re-encoded copies, five had pHash 0, PDQ 0–2 and a fit of
/// 1.00, and dHash 11–13 — a gradient sign flips in a flat region when a
/// picture is resized — and the pair rule alone dropped them.
#[inline]
fn rank(d: u32, p: u32, q: u32, td: u32, tp: u32, pdq: bool) -> Option<(u8, u32)> {
    if d <= td && p <= tp {
        Some((0, d + p))
    } else if pdq && q <= PDQ_MAX {
        Some((1, q))
    } else {
        None
    }
}

/// Both sides' PDQ is one PDQ itself rates reliable.
#[inline]
fn reliable(a: &Signature, b: &Signature) -> bool {
    a.pdq_quality >= PDQ_QUALITY_MIN && b.pdq_quality >= PDQ_QUALITY_MIN
}

/// One admitted variant comparison: (variant, d, p, q, rank).
type Admitted = (usize, u32, u32, u32, (u8, u32));

/// Best admitted variant comparison of `a`'s identity against every
/// variant of `b`. `None` when none is admitted.
#[inline]
fn best_against(a: &Signature, b: &Signature, td: u32, tp: u32) -> Option<Admitted> {
    let pdq = reliable(a, b);
    let mut best: Option<Admitted> = None;
    for k in 0..VARIANTS {
        let d = hamming(a.dhash[0], b.dhash[k]);
        let p = hamming(a.phash[0], b.phash[k]);
        let hashed = d <= td && p <= tp;
        if !hashed && !pdq {
            continue;
        }
        let q = pdq_distance(&a.pdq[0], &b.pdq[k]);
        let Some(r) = rank(d, p, q, td, tp, pdq) else {
            continue;
        };
        if best.is_none_or(|b| r < b.4) {
            best = Some((k, d, p, q, r));
        }
    }
    best
}

fn pair(a: usize, b: usize, sa: &Signature, sb: &Signature, td: u32, tp: u32) -> Option<Match> {
    let fwd = best_against(sa, sb, td, tp).map(|(k, d, p, q, r)| {
        (
            Match {
                a,
                b,
                variant: VARIANT_ORDER[k],
                flipped: false,
                d,
                p,
                q,
            },
            r,
        )
    });
    let rev = best_against(sb, sa, td, tp).map(|(k, d, p, q, r)| {
        (
            Match {
                a,
                b,
                variant: VARIANT_ORDER[k],
                flipped: true,
                d,
                p,
                q,
            },
            r,
        )
    });
    match (fwd, rev) {
        (Some(f), Some(r)) => Some(if f.1 <= r.1 { f.0 } else { r.0 }),
        (f, r) => f.or(r).map(|m| m.0),
    }
}

/// All matching pairs, choosing the engine by size.
pub fn match_pairs(
    sigs: &[Signature],
    td: u32,
    tp: u32,
    progress: &dyn MatchProgress,
) -> Vec<Match> {
    if sigs.len() <= LINEAR_MAX {
        linear_pairs(sigs, td, tp, progress)
    } else {
        mih_pairs(sigs, td, tp, progress)
    }
}

/// Parallel all-pairs scan.
pub fn linear_pairs(
    sigs: &[Signature],
    td: u32,
    tp: u32,
    progress: &dyn MatchProgress,
) -> Vec<Match> {
    let n = sigs.len();
    let total = n as u64;
    let done = AtomicU64::new(0);
    let mut out: Vec<Match> = (0..n)
        .into_par_iter()
        .flat_map_iter(|i| {
            let mut v = Vec::new();
            for j in i + 1..n {
                if let Some(m) = pair(i, j, &sigs[i], &sigs[j], td, tp) {
                    v.push(m);
                }
            }
            let dn = done.fetch_add(1, Ordering::Relaxed) + 1;
            if dn.is_multiple_of(64) || dn == total {
                progress.tick(dn, total);
            }
            v.into_iter()
        })
        .collect();
    out.sort_by_key(|m| (m.a, m.b));
    out
}

// ---------------------------------------------------------------------------
// Multi-Index Hashing
// ---------------------------------------------------------------------------

const SUBSTRINGS: usize = 4; // 64 bits → 4 × 16
const SUB_BITS: u32 = 16;

#[derive(Debug, Clone, Copy)]
struct Entry {
    image: u32,
    variant: u8,
    code: u64,
}

pub struct Mih {
    entries: Vec<Entry>,
    tables: Vec<HashMap<u16, Vec<u32>>>,
}

impl Mih {
    /// Index every variant's dHash of every signature.
    pub fn build(sigs: &[Signature]) -> Mih {
        let mut entries = Vec::with_capacity(sigs.len() * VARIANTS);
        for (i, s) in sigs.iter().enumerate() {
            for (k, code) in s.dhash.iter().enumerate() {
                entries.push(Entry {
                    image: i as u32,
                    variant: k as u8,
                    code: *code,
                });
            }
        }
        let mut tables: Vec<HashMap<u16, Vec<u32>>> =
            (0..SUBSTRINGS).map(|_| HashMap::new()).collect();
        for (idx, e) in entries.iter().enumerate() {
            for (s, table) in tables.iter_mut().enumerate() {
                table
                    .entry(substring(e.code, s))
                    .or_default()
                    .push(idx as u32);
            }
        }
        Mih { entries, tables }
    }

    /// Entries whose code is within Hamming distance `r` of `code`
    /// (verified exactly). Propositions 1 and 2 of the paper decide the
    /// per-substring radii: r = m·r′ + a → the first a+1 tables are searched
    /// at r′, the rest at r′−1.
    pub fn query(&self, code: u64, r: u32) -> Vec<u32> {
        let m = SUBSTRINGS as u32;
        let r_prime = r / m;
        let a = r - m * r_prime;
        let mut seen: Vec<u32> = Vec::new();
        for s in 0..SUBSTRINGS {
            let radius = if (s as u32) <= a {
                r_prime
            } else {
                r_prime.saturating_sub(1)
            };
            if (s as u32) > a && r_prime == 0 {
                continue; // radius would be −1: nothing can be found here
            }
            let q = substring(code, s);
            for_each_within(q, radius, |v| {
                if let Some(bucket) = self.tables[s].get(&v) {
                    for &idx in bucket {
                        if hamming(self.entries[idx as usize].code, code) <= r {
                            seen.push(idx);
                        }
                    }
                }
            });
        }
        seen.sort_unstable();
        seen.dedup();
        seen
    }

    fn entry(&self, idx: u32) -> Entry {
        self.entries[idx as usize]
    }
}

#[inline]
fn substring(code: u64, s: usize) -> u16 {
    ((code >> (SUB_BITS as usize * s)) & 0xFFFF) as u16
}

/// Call `f` with every 16-bit value within Hamming distance `radius` of `q`.
fn for_each_within(q: u16, radius: u32, mut f: impl FnMut(u16)) {
    f(q);
    if radius == 0 {
        return;
    }
    for i in 0..16 {
        f(q ^ (1 << i));
    }
    if radius == 1 {
        return;
    }
    for i in 0..16 {
        for j in i + 1..16 {
            f(q ^ (1 << i) ^ (1 << j));
        }
    }
    if radius == 2 {
        return;
    }
    // radius ≥ 3 is outside the thresholds this crate ships; generic fallback
    for i in 0..16 {
        for j in i + 1..16 {
            for k in j + 1..16 {
                f(q ^ (1 << i) ^ (1 << j) ^ (1 << k));
            }
        }
    }
}

/// PDQ's 256 bits as sixteen 16-bit substrings, every variant of every
/// signature PDQ rates reliable: a query within `PDQ_MAX` (31) searches
/// each table at radius 1 (31 = 16 × 1 + 15: Proposition 1, every table at
/// r′ = 1), seventeen probes a table. (Phase 19)
struct PdqIndex {
    entries: Vec<(u32, u8)>,
    codes: Vec<Hash256>,
    /// Per substring: the codes sorted by its value (`items`) and where
    /// each of the 65,536 values starts among them (`starts`), so a probe
    /// is two array reads, not a hash lookup.
    starts: Vec<Vec<u32>>,
    items: Vec<Vec<u32>>,
}

impl PdqIndex {
    fn sub(h: &Hash256, s: usize) -> u16 {
        ((h[s / 4] >> (16 * (s % 4))) & 0xFFFF) as u16
    }

    fn build(sigs: &[Signature]) -> PdqIndex {
        let mut entries = Vec::new();
        let mut codes = Vec::new();
        for (i, s) in sigs.iter().enumerate() {
            if s.pdq_quality < PDQ_QUALITY_MIN {
                continue;
            }
            for (k, h) in s.pdq.iter().enumerate() {
                entries.push((i as u32, k as u8));
                codes.push(*h);
            }
        }
        let mut starts = Vec::with_capacity(16);
        let mut items = Vec::with_capacity(16);
        for s in 0..16 {
            // counting sort of the codes by this substring
            let mut start = vec![0u32; (1 << 16) + 1];
            for h in &codes {
                start[Self::sub(h, s) as usize + 1] += 1;
            }
            for v in 0..1 << 16 {
                start[v + 1] += start[v];
            }
            let mut at = start.clone();
            let mut it = vec![0u32; codes.len()];
            for (idx, h) in codes.iter().enumerate() {
                let v = Self::sub(h, s) as usize;
                it[at[v] as usize] = idx as u32;
                at[v] += 1;
            }
            starts.push(start);
            items.push(it);
        }
        PdqIndex {
            entries,
            codes,
            starts,
            items,
        }
    }

    /// (image, variant, distance) of every code within `PDQ_MAX` of `h`.
    /// A code found under several substrings is verified each time (four
    /// XOR-popcounts, cheaper than sorting the candidates); the few that
    /// pass are deduplicated.
    fn query(&self, h: &Hash256) -> Vec<(u32, u8, u32)> {
        let mut hits: Vec<(u32, u32)> = Vec::new();
        for s in 0..16 {
            let (start, items) = (&self.starts[s], &self.items[s]);
            for_each_within(Self::sub(h, s), 1, |v| {
                let (a, b) = (start[v as usize] as usize, start[v as usize + 1] as usize);
                for &idx in &items[a..b] {
                    let q = pdq_distance(h, &self.codes[idx as usize]);
                    if q <= PDQ_MAX {
                        hits.push((idx, q));
                    }
                }
            });
        }
        hits.sort_unstable();
        hits.dedup();
        hits.into_iter()
            .map(|(idx, q)| {
                let (i, k) = self.entries[idx as usize];
                (i, k, q)
            })
            .collect()
    }
}

/// Pairs via the indexes: each identity dHash queries all variant codes,
/// pHash verifies on the same variant; each reliable identity PDQ queries
/// the PDQ index for what the 64-bit pair rule misses. Both directions
/// fall out naturally.
pub fn mih_pairs(sigs: &[Signature], td: u32, tp: u32, progress: &dyn MatchProgress) -> Vec<Match> {
    let index = Mih::build(sigs);
    let pdq_index = PdqIndex::build(sigs);
    let total = sigs.len() as u64;
    let done = AtomicU64::new(0);
    let mut out: Vec<Match> = (0..sigs.len())
        .into_par_iter()
        .flat_map_iter(|i| {
            let mut v: Vec<Match> = Vec::new();
            let si = &sigs[i];
            for idx in index.query(si.dhash[0], td) {
                let e = index.entry(idx);
                let j = e.image as usize;
                if j == i {
                    continue;
                }
                let k = e.variant as usize;
                let p = hamming(si.phash[0], sigs[j].phash[k]);
                if p > tp {
                    continue;
                }
                let d = hamming(si.dhash[0], e.code);
                let q = pdq_distance(&si.pdq[0], &sigs[j].pdq[k]);
                let (a, b, flipped) = if i < j { (i, j, false) } else { (j, i, true) };
                v.push(Match {
                    a,
                    b,
                    variant: VARIANT_ORDER[k],
                    flipped,
                    d,
                    p,
                    q,
                });
            }
            if si.pdq_quality >= PDQ_QUALITY_MIN {
                for (j, k, q) in pdq_index.query(&si.pdq[0]) {
                    let (j, k) = (j as usize, k as usize);
                    if j == i {
                        continue;
                    }
                    let d = hamming(si.dhash[0], sigs[j].dhash[k]);
                    let p = hamming(si.phash[0], sigs[j].phash[k]);
                    if d <= td && p <= tp {
                        continue; // the dHash index has it
                    }
                    let (a, b, flipped) = if i < j { (i, j, false) } else { (j, i, true) };
                    v.push(Match {
                        a,
                        b,
                        variant: VARIANT_ORDER[k],
                        flipped,
                        d,
                        p,
                        q,
                    });
                }
            }
            let dn = done.fetch_add(1, Ordering::Relaxed) + 1;
            if dn.is_multiple_of(256) || dn == total {
                progress.tick(dn, total);
            }
            v.into_iter()
        })
        .collect();
    // one match per unordered pair: keep the best admitted, by `rank`
    out.sort_by_key(|m| {
        (
            m.a,
            m.b,
            rank(m.d, m.p, m.q, td, tp, true).unwrap_or((2, 0)),
        )
    });
    out.dedup_by_key(|m| (m.a, m.b));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rng(seed: &mut u64) -> u64 {
        // xorshift64*
        *seed ^= *seed >> 12;
        *seed ^= *seed << 25;
        *seed ^= *seed >> 27;
        seed.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn flip_bits(code: u64, n: u32, seed: &mut u64) -> u64 {
        let mut c = code;
        let mut flipped = 0;
        while flipped < n {
            let bit = (rng(seed) % 64) as u32;
            if (c >> bit) & 1 == ((code >> bit) & 1) {
                c ^= 1 << bit;
                flipped += 1;
            }
        }
        c
    }

    /// Random signatures plus planted near-copies (identity and one rotated variant).
    fn corpus(n: usize, seed: u64) -> Vec<Signature> {
        let mut s = seed | 1;
        let mut out = Vec::new();
        for i in 0..n {
            let mut dh = [0u64; VARIANTS];
            let mut ph = [0u64; VARIANTS];
            for k in 0..VARIANTS {
                dh[k] = rng(&mut s);
                ph[k] = rng(&mut s);
            }
            let mut pq = [[0u64; 4]; VARIANTS];
            for word in pq.iter_mut().flatten() {
                *word = rng(&mut s);
            }
            out.push(Signature {
                dhash: dh,
                phash: ph,
                pdq: pq,
                pdq_quality: 80,
                width: 100,
                height: 100,
                quality: 50,
            });
            if i % 10 == 3 {
                // near copy: identity close to a variant of the one just pushed
                let prev = out.last().unwrap().clone();
                let k = (rng(&mut s) % VARIANTS as u64) as usize;
                let mut dh2 = [0u64; VARIANTS];
                let mut ph2 = [0u64; VARIANTS];
                for kk in 0..VARIANTS {
                    dh2[kk] = rng(&mut s);
                    ph2[kk] = rng(&mut s);
                }
                dh2[0] = flip_bits(prev.dhash[k], (rng(&mut s) % 6) as u32, &mut s);
                ph2[0] = flip_bits(prev.phash[k], (rng(&mut s) % 9) as u32, &mut s);
                let mut pq2 = [[0u64; 4]; VARIANTS];
                for word in pq2.iter_mut().flatten() {
                    *word = rng(&mut s);
                }
                // PDQ of the copy: the variant's hash with a few bits flipped per word
                for (word, src) in pq2[0].iter_mut().zip(prev.pdq[k].iter()) {
                    *word = flip_bits(*src, (rng(&mut s) % 4) as u32, &mut s);
                }
                out.push(Signature {
                    dhash: dh2,
                    phash: ph2,
                    pdq: pq2,
                    pdq_quality: 80,
                    width: 100,
                    height: 100,
                    quality: 50,
                });
            }
        }
        out
    }

    #[test]
    fn substring_radius_enumeration_counts() {
        let mut n = 0;
        for_each_within(0xBEEF, 0, |_| n += 1);
        assert_eq!(n, 1);
        n = 0;
        for_each_within(0xBEEF, 1, |_| n += 1);
        assert_eq!(n, 17);
        n = 0;
        for_each_within(0xBEEF, 2, |_| n += 1);
        assert_eq!(n, 137);
    }

    #[test]
    fn index_query_equals_brute_force() {
        let sigs = corpus(600, 99);
        let index = Mih::build(&sigs);
        let mut s = 7u64;
        for _ in 0..200 {
            let q = if rng(&mut s).is_multiple_of(2) {
                flip_bits(
                    sigs[(rng(&mut s) % sigs.len() as u64) as usize].dhash[3],
                    (rng(&mut s) % 11) as u32,
                    &mut s,
                )
            } else {
                rng(&mut s)
            };
            for r in [0u32, 3, 7, 10] {
                let got = index.query(q, r);
                let mut want: Vec<u32> = Vec::new();
                for (idx, e) in index.entries.iter().enumerate() {
                    if hamming(e.code, q) <= r {
                        want.push(idx as u32);
                    }
                }
                assert_eq!(got, want, "r={r}");
            }
        }
    }

    #[test]
    fn mih_pairs_equal_linear_pairs() {
        let sigs = corpus(1_500, 5);
        let lin = linear_pairs(&sigs, 10, 14, &NoMatchProgress);
        let mih = mih_pairs(&sigs, 10, 14, &NoMatchProgress);
        assert!(!lin.is_empty(), "corpus plants near copies");
        let lk: Vec<(usize, usize)> = lin.iter().map(|m| (m.a, m.b)).collect();
        let mk: Vec<(usize, usize)> = mih.iter().map(|m| (m.a, m.b)).collect();
        assert_eq!(lk, mk);
        for (l, m) in lin.iter().zip(mih.iter()) {
            assert_eq!(l.d + l.p, m.d + m.p, "{l:?} vs {m:?}");
        }
    }

    /// Copies the 64-bit pair rule misses — dHash and pHash random, PDQ a
    /// few bits from a variant of their original — are admitted by both
    /// engines alike, and only where both PDQs are reliable. (Phase 19)
    #[test]
    fn pdq_alone_admits_alike_in_both_engines() {
        let mut sigs = corpus(1_200, 17);
        let mut s = 0x9E37_79B9u64;
        let mut planted = Vec::new();
        for i in (0..sigs.len()).step_by(97) {
            let src = sigs[i].clone();
            let k = (rng(&mut s) % VARIANTS as u64) as usize;
            let mut c = corpus(1, rng(&mut s)).remove(0);
            for (word, w) in c.pdq[0].iter_mut().zip(src.pdq[k].iter()) {
                *word = flip_bits(*w, (rng(&mut s) % 6) as u32, &mut s);
            }
            // every third one unreliable: not admitted
            if planted.len() % 3 == 2 {
                c.pdq_quality = PDQ_QUALITY_MIN - 1;
            }
            planted.push((i, sigs.len(), c.pdq_quality >= PDQ_QUALITY_MIN));
            sigs.push(c);
        }
        let lin = linear_pairs(&sigs, 10, 14, &NoMatchProgress);
        let mih = mih_pairs(&sigs, 10, 14, &NoMatchProgress);
        let lk: Vec<(usize, usize)> = lin.iter().map(|m| (m.a, m.b)).collect();
        let mk: Vec<(usize, usize)> = mih.iter().map(|m| (m.a, m.b)).collect();
        assert_eq!(lk, mk);
        for (a, b, reliable) in planted {
            assert_eq!(lk.contains(&(a, b)), reliable, "{a} {b}");
        }
        for (l, m) in lin.iter().zip(mih.iter()) {
            assert_eq!((l.d, l.p, l.q), (m.d, m.p, m.q), "{l:?} vs {m:?}");
        }
    }

    /// The index alone at library sizes, both hash indexes in it;
    /// `cargo test --release -- --ignored mih_at_scale --nocapture`.
    #[test]
    #[ignore]
    fn mih_at_scale() {
        for n in [10_000usize, 50_000] {
            let sigs = corpus(n, 3);
            let t = std::time::Instant::now();
            let pairs = mih_pairs(&sigs, 10, 14, &NoMatchProgress);
            println!(
                "{n:>6} signatures  {:>6.2} s  {} pairs",
                t.elapsed().as_secs_f64(),
                pairs.len()
            );
        }
    }

    /// Timing of both engines at several sizes; `cargo test --release -- --ignored crossover`.
    #[test]
    #[ignore]
    fn crossover_timing() {
        for n in [2_000usize, 5_000, 10_000, 20_000, 40_000] {
            let sigs = corpus(n, 3);
            let t = std::time::Instant::now();
            let lin = linear_pairs(&sigs, 10, 14, &NoMatchProgress);
            let tl = t.elapsed();
            let t = std::time::Instant::now();
            let mih = mih_pairs(&sigs, 10, 14, &NoMatchProgress);
            let tm = t.elapsed();
            assert_eq!(lin.len(), mih.len());
            eprintln!(
                "n={:>6}  linear {:>8.3}s  mih {:>8.3}s  pairs {}",
                sigs.len(),
                tl.as_secs_f64(),
                tm.as_secs_f64(),
                lin.len()
            );
        }
    }

    #[test]
    fn planted_copies_are_found_and_random_pairs_are_not() {
        let sigs = corpus(400, 11);
        let pairs = linear_pairs(&sigs, 10, 14, &NoMatchProgress);
        // every planted pair is (i, i+1) where i % 10 == 3 in generation order; count them
        let planted = sigs.len() - 400;
        assert_eq!(pairs.len(), planted, "{pairs:?}");
        for m in &pairs {
            assert_eq!(m.b, m.a + 1);
        }
    }
}
