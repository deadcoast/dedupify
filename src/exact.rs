//! Exact-duplicate pipeline (docs/RESEARCH.md §1.3):
//!
//! size → sample (first + last 64 KiB) → full BLAKE3 → [verify bytes]
//!
//! Every stage drops singletons before the next stage reads anything, so
//! bytes read stay close to the bytes that are actually duplicated.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use rayon::prelude::*;

use crate::walk::Candidate;

/// Bytes hashed from each end of a file at the sample stage.
pub const WINDOW: u64 = 64 * 1024;
/// Files at or above this size hash with BLAKE3's internal rayon split
/// (the paper notes threading overhead dominates well below this).
pub const RAYON_THRESHOLD: u64 = 1 << 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Scan,
    Size,
    Sample,
    Hash,
    Verify,
    Decode,
    Match,
    Frames,
}

impl Stage {
    pub fn label(self) -> &'static str {
        match self {
            Stage::Scan => "SCAN",
            Stage::Size => "SIZE",
            Stage::Sample => "SAMPLE",
            Stage::Hash => "HASH",
            Stage::Verify => "VERIFY",
            Stage::Decode => "DECODE",
            Stage::Match => "MATCH",
            Stage::Frames => "FRAMES",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    /// Matched on size + 128 KiB sample only (`--quick-search`).
    Sample,
    /// Matched on full-content BLAKE3.
    Full,
}

#[derive(Debug, Clone)]
pub struct Group {
    pub size: u64,
    pub hash: [u8; 32],
    pub tier: Tier,
    /// Discovery order here; `keeper::order` puts the keeper first.
    pub members: Vec<Candidate>,
}

impl Group {
    /// Bytes freed if every member but one is removed.
    pub fn reclaimable(&self) -> u64 {
        self.size * (self.members.len() as u64 - 1)
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct PipelineStats {
    pub size_sharing: u64,
    pub sample_sharing: u64,
    pub confirmed: u64,
    pub bytes_sampled: u64,
    pub bytes_hashed: u64,
    pub unreadable: u64,
    pub hardlinks_collapsed: u64,
}

/// Progress sink; called from rayon workers, so it must be `Sync`.
pub trait Progress: Sync {
    fn tick(&self, stage: Stage, done: u64, total: u64);
    /// A stage has completed; `stats` holds everything known so far.
    fn done(&self, _stage: Stage, _stats: &PipelineStats) {}
}

pub struct NoProgress;
impl Progress for NoProgress {
    fn tick(&self, _: Stage, _: u64, _: u64) {}
}

/// Run the pipeline. `quick` stops after the sample stage.
pub fn find_groups(
    cands: Vec<Candidate>,
    quick: bool,
    progress: &dyn Progress,
) -> (Vec<Group>, PipelineStats) {
    let mut stats = PipelineStats::default();

    // ---- SIZE -----------------------------------------------------------
    let total = cands.len() as u64;
    progress.tick(Stage::Size, 0, total);
    let mut by_size: HashMap<u64, Vec<Candidate>> = HashMap::new();
    for c in cands {
        by_size.entry(c.size).or_default().push(c);
    }
    by_size.retain(|_, v| v.len() >= 2);

    // hard links: two names for one file share a size, so the check is
    // only needed (and only paid for) inside size groups
    for v in by_size.values_mut() {
        let before = v.len();
        collapse_hardlinks(v);
        stats.hardlinks_collapsed += (before - v.len()) as u64;
    }
    by_size.retain(|_, v| v.len() >= 2);

    let mut pool: Vec<Candidate> = by_size.into_values().flatten().collect();
    pool.sort_by(|a, b| a.path.cmp(&b.path));
    stats.size_sharing = pool.len() as u64;
    progress.tick(Stage::Size, total, total);
    progress.done(Stage::Size, &stats);

    if pool.is_empty() {
        // nothing shares a size: the SIZE line already says so, and the
        // sample and hash stages have nothing to report
        return (Vec::new(), stats);
    }

    // ---- SAMPLE ---------------------------------------------------------
    let n = pool.len() as u64;
    let done = AtomicU64::new(0);
    progress.tick(Stage::Sample, 0, n);
    let sampled: Vec<(Candidate, io::Result<[u8; 32]>)> = pool
        .into_par_iter()
        .map(|c| {
            let h = sample_hash(&c.path, c.size);
            let d = done.fetch_add(1, Ordering::Relaxed) + 1;
            progress.tick(Stage::Sample, d, n);
            (c, h)
        })
        .collect();

    let mut by_sample: HashMap<(u64, [u8; 32]), Vec<Candidate>> = HashMap::new();
    for (c, h) in sampled {
        match h {
            Ok(h) => {
                stats.bytes_sampled += sample_bytes(c.size);
                by_sample.entry((c.size, h)).or_default().push(c);
            }
            Err(_) => stats.unreadable += 1,
        }
    }
    by_sample.retain(|_, v| v.len() >= 2);
    stats.sample_sharing = by_sample.values().map(|v| v.len() as u64).sum();
    progress.done(Stage::Sample, &stats);

    // files no larger than the two windows were fully covered by the
    // sample: their sample hash is their full hash
    let mut groups: Vec<Group> = Vec::new();
    let mut to_hash: Vec<Candidate> = Vec::new();
    let mut sample_of: HashMap<std::path::PathBuf, [u8; 32]> = HashMap::new();
    for ((size, h), members) in by_sample {
        if quick {
            groups.push(Group {
                size,
                hash: h,
                tier: Tier::Sample,
                members,
            });
        } else if size <= 2 * WINDOW {
            stats.confirmed += members.len() as u64;
            groups.push(Group {
                size,
                hash: h,
                tier: Tier::Full,
                members,
            });
        } else {
            for m in &members {
                sample_of.insert(m.path.clone(), h);
            }
            to_hash.extend(members);
        }
    }

    if quick {
        finish(&mut groups);
        return (groups, stats);
    }

    // ---- HASH -----------------------------------------------------------
    let n = to_hash.len() as u64;
    if n > 0 {
        let done = AtomicU64::new(0);
        let bytes = AtomicU64::new(0);
        progress.tick(Stage::Hash, 0, n);
        to_hash.sort_by_key(|c| std::cmp::Reverse(c.size)); // big files first: better pool balance
        let hashed: Vec<(Candidate, io::Result<[u8; 32]>)> = to_hash
            .into_par_iter()
            .map(|c| {
                let h = full_hash(&c.path, c.size);
                if h.is_ok() {
                    bytes.fetch_add(c.size, Ordering::Relaxed);
                }
                let d = done.fetch_add(1, Ordering::Relaxed) + 1;
                progress.tick(Stage::Hash, d, n);
                (c, h)
            })
            .collect();
        stats.bytes_hashed = bytes.load(Ordering::Relaxed);

        let mut by_hash: HashMap<(u64, [u8; 32]), Vec<Candidate>> = HashMap::new();
        for (c, h) in hashed {
            match h {
                Ok(h) => by_hash.entry((c.size, h)).or_default().push(c),
                Err(_) => stats.unreadable += 1,
            }
        }
        by_hash.retain(|_, v| v.len() >= 2);
        for ((size, h), members) in by_hash {
            stats.confirmed += members.len() as u64;
            groups.push(Group {
                size,
                hash: h,
                tier: Tier::Full,
                members,
            });
        }
    }
    progress.done(Stage::Hash, &stats);

    finish(&mut groups);
    (groups, stats)
}

/// Deterministic order: largest reclaimable first, then by first path.
fn finish(groups: &mut [Group]) {
    for g in groups.iter_mut() {
        g.members.sort_by(|a, b| a.path.cmp(&b.path));
    }
    groups.sort_by(|a, b| {
        b.reclaimable()
            .cmp(&a.reclaimable())
            .then_with(|| a.members[0].path.cmp(&b.members[0].path))
    });
}

fn collapse_hardlinks(v: &mut Vec<Candidate>) {
    let mut seen: HashSet<same_file::Handle> = HashSet::new();
    v.retain(|c| match same_file::Handle::from_path(&c.path) {
        Ok(h) => seen.insert(h),
        Err(_) => true, // unreadable now; the sample stage will count it
    });
}

pub fn sample_bytes(size: u64) -> u64 {
    if size <= 2 * WINDOW {
        size
    } else {
        2 * WINDOW
    }
}

/// BLAKE3 over the first and last `WINDOW` bytes (whole file when it is
/// no larger than both windows together).
pub fn sample_hash(path: &Path, size: u64) -> io::Result<[u8; 32]> {
    let mut f = File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    if size <= 2 * WINDOW {
        let mut buf = Vec::with_capacity(size as usize);
        f.read_to_end(&mut buf)?;
        hasher.update(&buf);
    } else {
        let mut buf = vec![0u8; WINDOW as usize];
        f.read_exact(&mut buf)?;
        hasher.update(&buf);
        f.seek(SeekFrom::End(-(WINDOW as i64)))?;
        f.read_exact(&mut buf)?;
        hasher.update(&buf);
    }
    Ok(*hasher.finalize().as_bytes())
}

/// Full-content BLAKE3 with a buffered sequential read.
pub fn full_hash(path: &Path, size: u64) -> io::Result<[u8; 32]> {
    let mut f = File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    let cap: usize = if size >= 32 * RAYON_THRESHOLD {
        4 << 20
    } else {
        1 << 20
    };
    let mut buf = vec![0u8; cap];
    let parallel = size >= RAYON_THRESHOLD;
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        if parallel {
            hasher.update_rayon(&buf[..n]);
        } else {
            hasher.update(&buf[..n]);
        }
    }
    Ok(*hasher.finalize().as_bytes())
}

/// Byte-for-byte comparison, 1 MiB at a time. Used by `purge` before the
/// Recycle Bin call so a hash match is never the sole basis for removal.
pub fn verify_identical(a: &Path, b: &Path) -> io::Result<bool> {
    let mut fa = File::open(a)?;
    let mut fb = File::open(b)?;
    if fa.metadata()?.len() != fb.metadata()?.len() {
        return Ok(false);
    }
    let mut ba = vec![0u8; 1 << 20];
    let mut bb = vec![0u8; 1 << 20];
    loop {
        let na = read_fill(&mut fa, &mut ba)?;
        let nb = read_fill(&mut fb, &mut bb)?;
        if na != nb || ba[..na] != bb[..nb] {
            return Ok(false);
        }
        if na == 0 {
            return Ok(true);
        }
    }
}

fn read_fill(f: &mut File, buf: &mut [u8]) -> io::Result<usize> {
    let mut total = 0;
    while total < buf.len() {
        let n = f.read(&mut buf[total..])?;
        if n == 0 {
            break;
        }
        total += n;
    }
    Ok(total)
}

pub fn hex(h: &[u8; 32]) -> String {
    h.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kind::Kind;
    use crate::walk::scan;
    use std::fs;
    use std::path::PathBuf;

    fn mk(dir: &Path, rel: &str, bytes: &[u8]) -> PathBuf {
        let p = dir.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, bytes).unwrap();
        p
    }

    fn big(seed: u8, len: usize) -> Vec<u8> {
        (0..len)
            .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
            .collect()
    }

    #[test]
    fn small_files_confirm_at_sample_stage() {
        let d = tempfile::tempdir().unwrap();
        mk(d.path(), "a.bin", b"same bytes");
        mk(d.path(), "b.bin", b"same bytes");
        mk(d.path(), "c.bin", b"same byteS");
        mk(d.path(), "d.bin", b"other");
        let (c, _) = scan(
            d.path(),
            true,
            Kind::All,
            crate::walk::Filter::default(),
            &mut |_| {},
        );
        let (g, s) = find_groups(c, false, &NoProgress);
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].members.len(), 2);
        assert_eq!(g[0].tier, Tier::Full);
        assert_eq!(s.size_sharing, 3); // a, b, c share a size
        assert_eq!(s.sample_sharing, 2);
        assert_eq!(s.confirmed, 2);
        assert_eq!(s.bytes_hashed, 0); // sample covered the whole file
    }

    #[test]
    fn large_files_need_full_hash_and_sample_prefilters() {
        let d = tempfile::tempdir().unwrap();
        let len = (2 * WINDOW + 4096) as usize;
        let base = big(1, len);
        let mut middle_diff = base.clone();
        middle_diff[len / 2] ^= 0xff; // same head, same tail, different body
        let mut head_diff = base.clone();
        head_diff[10] ^= 0xff;
        mk(d.path(), "x1.bin", &base);
        mk(d.path(), "sub/x2.bin", &base);
        mk(d.path(), "mid.bin", &middle_diff);
        mk(d.path(), "head.bin", &head_diff);
        let (c, _) = scan(
            d.path(),
            true,
            Kind::All,
            crate::walk::Filter::default(),
            &mut |_| {},
        );
        let (g, s) = find_groups(c, false, &NoProgress);
        assert_eq!(s.size_sharing, 4);
        assert_eq!(
            s.sample_sharing, 3,
            "head.bin must fall out at the sample stage"
        );
        assert_eq!(g.len(), 1, "mid.bin must fall out at the full-hash stage");
        assert_eq!(g[0].members.len(), 2);
        assert_eq!(g[0].reclaimable(), len as u64);
        assert_eq!(s.bytes_hashed, 3 * len as u64);
    }

    #[test]
    fn quick_reports_sample_tier_and_never_full_hashes() {
        let d = tempfile::tempdir().unwrap();
        let len = (2 * WINDOW + 4096) as usize;
        let base = big(7, len);
        let mut middle_diff = base.clone();
        middle_diff[len / 2] ^= 0xff;
        mk(d.path(), "x1.bin", &base);
        mk(d.path(), "mid.bin", &middle_diff);
        let (c, _) = scan(
            d.path(),
            true,
            Kind::All,
            crate::walk::Filter::default(),
            &mut |_| {},
        );
        let (g, s) = find_groups(c, true, &NoProgress);
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].tier, Tier::Sample);
        assert_eq!(s.bytes_hashed, 0);
    }

    #[test]
    fn hardlinks_collapse() {
        let d = tempfile::tempdir().unwrap();
        let a = mk(d.path(), "a.bin", b"linked content");
        let b = d.path().join("b.bin");
        if fs::hard_link(&a, &b).is_err() {
            return; // filesystem without hard links; nothing to test here
        }
        mk(d.path(), "c.bin", b"linked content");
        let (c, _) = scan(
            d.path(),
            true,
            Kind::All,
            crate::walk::Filter::default(),
            &mut |_| {},
        );
        let (g, s) = find_groups(c, false, &NoProgress);
        assert_eq!(s.hardlinks_collapsed, 1);
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].members.len(), 2); // one of {a,b} plus c
    }

    #[test]
    fn verify_bytes() {
        let d = tempfile::tempdir().unwrap();
        let len = (3 << 20) + 17;
        let base = big(3, len);
        let mut diff = base.clone();
        diff[len - 1] ^= 1;
        let a = mk(d.path(), "a", &base);
        let b = mk(d.path(), "b", &base);
        let c = mk(d.path(), "c", &diff);
        assert!(verify_identical(&a, &b).unwrap());
        assert!(!verify_identical(&a, &c).unwrap());
    }

    #[test]
    fn hashes_are_stable_and_distinct() {
        let d = tempfile::tempdir().unwrap();
        let a = mk(d.path(), "a", b"hello");
        let h1 = full_hash(&a, 5).unwrap();
        let h2 = sample_hash(&a, 5).unwrap();
        assert_eq!(h1, h2); // whole file covered by the sample
        assert_eq!(hex(&h1), blake3::hash(b"hello").to_hex().as_str());
    }
}
