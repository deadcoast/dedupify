//! Near-duplicate image pass: representatives → signatures → matched pairs
//! → connected components → ordered groups.
//!
//! Exact duplicates are resolved before this runs, so each exact group is
//! represented once (by its keeper); the near pass never re-reports a
//! byte-identical copy as "similar".

use std::sync::atomic::{AtomicU64, Ordering};

use rayon::prelude::*;

mod bars;
mod group;
mod relation;

pub use bars::*;
pub use group::*;
pub use relation::*;

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
    /// A camera RAW, hashed from its embedded preview: the keeper of its
    /// group when it is in one, never acted on as a member.
    pub raw: bool,
    /// Flat bars around the picture (left, top, right, bottom as shares of
    /// the width and height), found the way the video tier finds them:
    /// a letterboxed or padded copy carries its original's picture inside
    /// more pixels, and the keeper rule counts the picture. [CLAUDE]
    pub bars: Option<[f32; 4]>,
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
    /// Make member `idx` the keeper: it moves to the front and every other
    /// member's relation is recomputed against it, the geometric fit
    /// included where the cheaper evidence leaves the question open (the
    /// same rule as `group`). The workspace's `k` key. [CLAUDE]
    pub fn set_keeper(&mut self, idx: usize) {
        if idx == 0 || idx >= self.members.len() {
            return;
        }
        let chosen = self.members.remove(idx);
        self.members.insert(0, chosen);
        self.members[0].relation = None;
        let keeper = self.members[0].hashed.clone();
        let mut thumb: Option<Option<geometry::Thumb>> = None;
        for m in self.members.iter_mut().skip(1) {
            let mut r = relation_of(&keeper, &m.hashed);
            let needs = !r.kin && ((r.hash_pair() && !r.pdq_ok()) || r.deep_only());
            if needs {
                let k = thumb.get_or_insert_with(|| geometry::thumb_of(&keeper.rep.cand.path));
                r.fit = k.as_ref().and_then(|k| {
                    Some(geometry::fit(
                        k,
                        &geometry::thumb_of(&m.hashed.rep.cand.path)?,
                    ))
                });
            }
            m.relation = Some(r);
        }
    }

    /// Bytes `milk`/`purge` act on: confirmed members only.
    pub fn reclaimable(&self) -> u64 {
        self.members
            .iter()
            .skip(1)
            .filter(|m| m.relation.is_some_and(|r| r.confirmed()))
            .map(|m| m.hashed.rep.cand.size)
            .sum()
    }

    /// Every member but the keeper, by what its line says: `kin` first,
    /// then a RAW and its camera JPEG (`pair`), a RAW (`raw`), the
    /// descriptor's word alone (`deep`), and the rest (`similar`).
    pub fn kinds(&self) -> Kinds {
        let mut k = Kinds::default();
        for m in self.members.iter().skip(1) {
            let Some(r) = m.relation else { continue };
            if r.kin {
                k.kin += 1;
            } else if r.pair {
                k.pair += 1;
            } else if r.raw {
                k.raw += 1;
            } else if r.deep_only() && !r.fit_ok() {
                k.deep += 1;
            } else {
                k.similar += 1;
            }
        }
        k
    }

    /// Members shown with the `deep` verb (learned descriptor alone).
    pub fn deep_only_count(&self) -> usize {
        self.kinds().deep
    }

    /// Members shown with the `kin` verb (different capture moment).
    pub fn kin_count(&self) -> usize {
        self.kinds().kin
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

/// Member counts of one group, by kind (`NearGroup::kinds`).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Kinds {
    pub similar: usize,
    pub deep: usize,
    pub kin: usize,
    pub pair: usize,
    pub raw: usize,
}

#[derive(Debug, Default, Clone)]
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
    /// Decoded through a route other than the `image` crate.
    pub heif: u64,
    pub avif: u64,
    pub raw: u64,
    pub ffmpeg: u64,
    /// Not decoded: ffmpeg needed and absent; ffmpeg failed; a RAW with no
    /// usable preview. The rest of `undecodable` is unreadable files.
    pub need_ffmpeg: u64,
    pub ffmpeg_failed: u64,
    pub no_preview: u64,
    /// Members of near groups that are a RAW's own camera JPEG.
    pub raw_pairs: u64,
    /// Files whose ffmpeg sent nothing for `tools::stall()` and was
    /// stopped (Phase 13, CORE-11); counted in `undecodable` too.
    pub stalled: Vec<std::path::PathBuf>,
}

// ---------------------------------------------------------------------------
// union–find
// ---------------------------------------------------------------------------

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
        (
            Signature,
            Option<crate::deep::Embedding>,
            Option<Capture>,
            crate::decode::Route,
            Option<[f32; 4]>,
        ),
        crate::decode::DecodeError,
    >;
    let results: Vec<(Rep, Decoded)> = reps
        .into_par_iter()
        .map(|rep| {
            // one decode feeds every hash and, when active, the embedding;
            // the capture time comes from the route when it read the
            // metadata (HEIF, RAW), else from the file's EXIF
            let r = crate::decode::load(&rep.cand.path).map(|d| {
                let sig = perceptual::signature_from_image(&d.img);
                let emb = deep.and_then(|m| match m.embed(&d.img) {
                    Ok(e) => Some(e),
                    Err(_) => {
                        deep_failed.fetch_add(1, Ordering::Relaxed);
                        None
                    }
                });
                let cap = match d.capture {
                    Some(c) => c,
                    None => capture::read(&rep.cand.path),
                };
                (sig, emb, cap, d.route, picture_bars(&d.img))
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
            Ok((sig, emb, capture, route, bars)) => {
                use crate::decode::Route;
                use crate::heif::Codec;
                stats.decoded += 1;
                match route {
                    Route::Image => {}
                    Route::Heif(Codec::Av1) => stats.avif += 1,
                    Route::Heif(_) => stats.heif += 1,
                    Route::Raw => stats.raw += 1,
                    Route::Ffmpeg => stats.ffmpeg += 1,
                }
                if sig.quality < QUALITY_MIN {
                    stats.low_quality += 1;
                } else {
                    out.push(Hashed {
                        raw: route == Route::Raw,
                        rep,
                        sig,
                        deep: emb,
                        capture,
                        bars,
                    });
                }
            }
            Err(e) => {
                use crate::decode::DecodeError as E;
                stats.undecodable += 1;
                match e {
                    E::NoFfmpeg(_) => stats.need_ffmpeg += 1,
                    E::Ffmpeg(..) => stats.ffmpeg_failed += 1,
                    E::NoPreview => stats.no_preview += 1,
                    E::Stalled(_) => stats.stalled.push(rep.cand.path.clone()),
                    _ => {}
                }
            }
        }
    }
    (out, stats)
}

/// Keeper order inside one group: the member the most other members match
/// directly on the two 64-bit hashes (the hub — an original is what its
/// derivatives all resemble, derivatives need not resemble each other),
/// then most pixels of the picture itself (a downscaled copy is the
/// derivative; a padded or letterboxed one counts the picture inside its
/// bars, `picture_pixels`), then the path that does not name or file it
/// as a copy (`keeper::derived`, CORE-07), then oldest, then largest file
/// (at equal age and size a lossless encoding outweighs a lossy one),
/// then path. Degree comes first so a padded copy cannot take the seat
/// where the pictures' own bars go unseen (bars of a texture, a border
/// with a gradient). A RAW comes before all of it (Phase 10).
pub fn keeper_order_with_degree(
    a: &Hashed,
    da: usize,
    b: &Hashed,
    db: usize,
) -> std::cmp::Ordering {
    // a RAW is the original exposure: it keeps its group (Phase 10)
    b.raw
        .cmp(&a.raw)
        .then_with(|| db.cmp(&da))
        .then_with(|| keeper_order(a, b))
}

pub fn keeper_order(a: &Hashed, b: &Hashed) -> std::cmp::Ordering {
    b.raw.cmp(&a.raw).then_with(|| {
        picture_pixels(b)
            .total_cmp(&picture_pixels(a))
            .then_with(|| {
                crate::keeper::derived(&a.rep.cand.path)
                    .cmp(&crate::keeper::derived(&b.rep.cand.path))
            })
            .then_with(|| a.rep.cand.mtime.cmp(&b.rep.cand.mtime))
            .then_with(|| b.rep.cand.size.cmp(&a.rep.cand.size))
            .then_with(|| a.rep.cand.path.cmp(&b.rep.cand.path))
    })
}

pub const NEAR_POLICY_LABEL: &str =
    "keep: shared moment, the RAW, most matched, most pixels, not a copy";
