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
    /// A camera RAW, hashed from its embedded preview: the keeper of its
    /// group when it is in one, never acted on as a member.
    pub raw: bool,
    /// Flat bars around the picture (left, top, right, bottom as shares of
    /// the width and height), found the way the video tier finds them:
    /// a letterboxed or padded copy carries its original's picture inside
    /// more pixels, and the keeper rule counts the picture. [CLAUDE]
    pub bars: Option<[f32; 4]>,
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
    /// The member is a camera RAW: listed, never acted on, not even by
    /// hand (Phase 10). [CLAUDE]
    pub raw: bool,
    /// The keeper is a camera RAW: the member's size is not compared with
    /// it (a RAW's own pixel count is not read, only its preview's).
    pub keeper_raw: bool,
    /// A RAW and the camera's own JPEG of it (same folder, same name):
    /// kept together; the JPEG can be chosen by hand.
    pub pair: bool,
    /// Another frame of the same moment: both files state one capture
    /// time and it agrees, but their names are numbered apart
    /// (`IMG_0012`, `IMG_0013`) — a burst from a camera that records no
    /// fraction of a second. Counted as `kin`. [CLAUDE, Phase 11]
    pub frame: bool,
    /// Flat bars one side has and the other has not.
    pub boxed: Boxed,
}

/// Which of a near pair has flat bars around its picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Boxed {
    /// Neither, or both.
    #[default]
    No,
    /// The member's picture sits between bars left and right.
    Pillar,
    /// Between bars above and below.
    Letter,
    /// Inside a frame on all four sides.
    Border,
    /// The keeper has bars the member has not.
    Removed,
}

impl Boxed {
    fn of(keeper: Option<[f32; 4]>, member: Option<[f32; 4]>) -> Boxed {
        match (keeper, member) {
            (None, Some([l, t, r, b])) => {
                let (x, y) = (l > 0.0 || r < 1.0, t > 0.0 || b < 1.0);
                match (x, y) {
                    (true, false) => Boxed::Pillar,
                    (false, true) => Boxed::Letter,
                    _ => Boxed::Border,
                }
            }
            (Some(_), None) => Boxed::Removed,
            _ => Boxed::No,
        }
    }
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
        self.fit.is_some_and(|f| f.confirmed())
    }

    /// Admitted by the learned descriptor: at `DEEP_MIN`, or in the band
    /// under it when the fit confirms the pair as well.
    pub fn deep_in(&self) -> bool {
        self.deep.is_some_and(|c| c >= crate::deep::DEEP_MIN) || (self.deep_band() && self.fit_ok())
    }

    /// The descriptor's cosine is in `[DEEP_FIT_MIN, DEEP_MIN)`: such a
    /// pair is grouped only when the fit confirms it (Phase 11).
    pub fn deep_band(&self) -> bool {
        self.deep
            .is_some_and(|c| (crate::deep::DEEP_FIT_MIN..crate::deep::DEEP_MIN).contains(&c))
    }

    /// Grouped by the deep tier alone: the two 64-bit hashes disagree.
    pub fn deep_only(&self) -> bool {
        !self.hash_pair() && self.deep_in()
    }

    /// What `milk` and `purge` require before touching the file.
    pub fn confirmed(&self) -> bool {
        if self.kin || self.raw || self.pair {
            return false;
        }
        if self.hash_pair() {
            self.pdq_ok() || self.fit_ok()
        } else {
            self.deep_in() && self.fit_ok()
        }
    }

    /// Shown but never acted on by the program's rule, whatever else is
    /// measured: a different photograph (`kin`), a RAW, or the learned
    /// descriptor's word alone.
    pub fn listed_only(&self) -> bool {
        self.kin || self.raw || (self.deep_only() && !self.fit_ok())
    }

    /// The verb on the member's line.
    pub fn verb(&self) -> &'static str {
        if self.kin {
            "kin"
        } else if self.raw {
            "raw"
        } else if self.pair {
            "pair"
        } else if self.deep_only() && !self.fit_ok() {
            "deep"
        } else {
            "near"
        }
    }

    /// What the member is, in words, for its line: the relation read off
    /// the evidence rather than the evidence itself. `keeper`/`member`
    /// supply the pixel sizes and the capture time. [CLAUDE]
    ///
    /// `mirrored`, `rotated 90°`, `resized 50%`, `cropped 80%`,
    /// `letterboxed`, `re-encoded`, `same picture`, `another shot …`,
    /// `looks alike`, `similar, unconfirmed`; two words at most, joined
    /// with a comma.
    pub fn words(&self, keeper: &Hashed, member: &Hashed) -> String {
        self.words_for(
            (keeper.sig.width, keeper.sig.height),
            (member.sig.width, member.sig.height),
            keeper.capture.as_ref().map(|c| c.datetime.as_str()),
            member.capture.as_ref().map(|c| c.datetime.as_str()),
        )
    }

    /// `words` on bare inputs: pixel sizes and `YYYY-MM-DD HH:MM:SS`
    /// capture times.
    pub fn words_for(
        &self,
        keeper: (u32, u32),
        member: (u32, u32),
        keeper_taken: Option<&str>,
        member_taken: Option<&str>,
    ) -> String {
        if self.frame {
            return "another shot, same second".to_string();
        }
        if self.kin {
            // the member's own moment: the time when the day is the
            // keeper's, the day otherwise
            return match (member_taken, keeper_taken) {
                (Some(m), Some(k)) if m.get(..10) == k.get(..10) => {
                    format!("another shot, {}", m.get(11..).unwrap_or(""))
                }
                (Some(m), _) => format!("another shot, {}", m.get(..10).unwrap_or("")),
                (None, _) => "another shot".to_string(),
            };
        }
        if self.pair {
            return "RAW + JPEG".to_string();
        }
        if self.raw {
            return "the RAW".to_string();
        }
        if self.deep_only() && !self.fit_ok() {
            return "looks alike".to_string();
        }
        if !self.confirmed() {
            return "similar, unconfirmed".to_string();
        }
        let mut parts: Vec<String> = Vec::new();
        // orientation: a 64-bit finding, so only when the hashes agree
        let orientation = if self.hash_pair() {
            match self.variant {
                Variant::Identity => "",
                Variant::Rotate90 => "rotated 90°",
                Variant::Rotate180 => "rotated 180°",
                Variant::Rotate270 => "rotated 270°",
                Variant::FlipH => "mirrored",
                Variant::FlipV => "flipped",
                Variant::Transpose | Variant::AntiTranspose => "rotated and mirrored",
            }
        } else {
            ""
        };
        if !orientation.is_empty() {
            parts.push(orientation.to_string());
        }
        // geometry: what the fit found, when it was the evidence
        let mut geometric = false;
        let fit = self.fit.filter(|f| f.confirmed());
        if fit.is_some_and(|f| f.mirrored) && orientation.is_empty() {
            parts.push("mirrored".to_string());
        }
        // flat bars one side has: what was added or taken away around the
        // picture, which the fit's own placement then need not repeat
        let boxed = match self.boxed {
            Boxed::No => None,
            Boxed::Pillar => Some("pillarboxed"),
            Boxed::Letter => Some("letterboxed"),
            Boxed::Border => Some("with a border"),
            Boxed::Removed => Some("bars removed"),
        };
        if let Some(w) = boxed {
            parts.push(w.to_string());
            geometric = true;
        }
        if let Some(f) = fit {
            if f.angle != 0.0 {
                parts.push(format!("rotated {:+.0}°", f.angle));
                geometric = true;
            } else if f.scale < 0.995 && boxed.is_none() {
                if f.inner_is_second {
                    parts.push(format!("cropped {:.0}%", f.scale * 100.0));
                } else {
                    // the member holds the keeper's picture inside a
                    // frame: bars when the shape changed, a border otherwise
                    let ka = keeper.0 as f32 / keeper.1.max(1) as f32;
                    let ma = member.0 as f32 / member.1.max(1) as f32;
                    parts.push(if (ka - ma).abs() / ka > 0.05 {
                        "letterboxed".to_string()
                    } else {
                        "with a border".to_string()
                    });
                }
                geometric = true;
            }
        }
        // size: the member's pixels against the keeper's, on the member's
        // own orientation
        let (mw, mh) = match self.variant {
            Variant::Rotate90
            | Variant::Rotate270
            | Variant::Transpose
            | Variant::AntiTranspose => (member.1, member.0),
            _ => (member.0, member.1),
        };
        let (kw, kh) = (keeper.0.max(1), keeper.1.max(1));
        let rw = mw as f32 / kw as f32;
        let rh = mh as f32 / kh as f32;
        let same_size = (rw - 1.0).abs() <= 0.03 && (rh - 1.0).abs() <= 0.03;
        // against a RAW the comparison would be with its preview's size
        if !geometric && !same_size && !self.keeper_raw {
            if (rw - rh).abs() <= 0.03 {
                parts.push(format!("resized {:.0}%", rw * 100.0));
            } else {
                parts.push(format!("{}×{}", mw, mh));
            }
        }
        if parts.is_empty() && self.keeper_raw {
            parts.push("from the RAW".to_string());
        }
        if parts.is_empty() {
            parts.push(if !self.hash_pair() {
                // in through the descriptor and the fit: the picture is
                // there, the pixels were worked on
                "edited".to_string()
            } else if self.d == 0 && self.p == 0 && self.q == 0 {
                "same picture".to_string()
            } else if self.q <= 8 {
                "re-encoded".to_string()
            } else {
                "near copy".to_string()
            });
        }
        parts.truncate(2);
        parts.join(", ")
    }

    /// The evidence behind the words, for the dim trailing column:
    /// `d 0  p 0  q 4`, then `deep 0.80` and `fit 1.00` when they exist.
    pub fn numbers(&self) -> String {
        let mut s = format!("d {:>2}  p {:>2}  q {:>3}", self.d, self.p, self.q);
        if let Some(c) = self.deep {
            s.push_str(&format!("  deep {c:.2}"));
        }
        if let Some(f) = self.fit {
            s.push_str(&format!("  fit {:.2}", f.score().max(0.0)));
        }
        s
    }

    /// Why `confirmed()` is false, for the `kept~` line.
    pub fn unconfirmed_reason(&self) -> String {
        if self.frame {
            return "another frame of the same second".to_string();
        }
        if self.kin {
            return "taken at a different moment".to_string();
        }
        if self.pair {
            return "the camera's JPEG of a RAW, kept with it".to_string();
        }
        if self.raw {
            return "a RAW, never moved".to_string();
        }
        let fit_txt = match self.fit {
            Some(f) => format!("fit {:.2} under {FIT_MIN:.2}", f.score().max(0.0)),
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

/// Bars around a still picture: rows at the top and bottom, or columns at
/// the left and right, all of one colour, on one grey copy `FLAT_WIDTH`
/// wide. The left, top, right and bottom of the picture as shares of the
/// width and height. [CLAUDE, found on the owner's Windows run: a
/// letterboxed export kept its group over the original, having more
/// pixels]
///
/// An export pads with one colour, exactly, so a line is a bar when
/// `STILL_BAR_SHARE` of its pixels sit within `STILL_BAR_TOL` grey levels
/// of the outermost line's colour, and both ends carry such bands of one
/// colour, of about one depth (2 % of the side), leaving the picture a
/// quarter of the side or more (`video::BAR_MIN_SHARE`, `BAR_KEEP_MIN`).
/// The video tier's rule (detail along the band, and a picture that
/// starts at once) was tried first and missed half of the letterboxed
/// photographs whose sky meets the bar (VERIFICATION § Phase 12).
pub fn picture_bars(img: &image::DynamicImage) -> Option<[f32; 4]> {
    let w = crate::video::FLAT_WIDTH;
    if img.width() < 16 || img.height() < 16 {
        return None;
    }
    let g = if img.width() > w {
        img.thumbnail(w, u32::MAX).to_luma8()
    } else {
        img.to_luma8()
    };
    let (gw, gh) = (g.width() as usize, g.height() as usize);
    let px = g.as_raw();
    let rows: Vec<Vec<u8>> = (0..gh).map(|y| px[y * gw..(y + 1) * gw].to_vec()).collect();
    let cols: Vec<Vec<u8>> = (0..gw)
        .map(|x| (0..gh).map(|y| px[y * gw + x]).collect())
        .collect();
    let x = uniform_band(&cols);
    let y = uniform_band(&rows);
    if x.is_none() && y.is_none() {
        return None;
    }
    let (l, r) = x.map_or((0.0, 1.0), |(a, b)| {
        (a as f32 / gw as f32, 1.0 - b as f32 / gw as f32)
    });
    let (t, b) = y.map_or((0.0, 1.0), |(a, b)| {
        (a as f32 / gh as f32, 1.0 - b as f32 / gh as f32)
    });
    Some([l, t, r, b])
}

/// Grey levels a bar's pixel may stray from the bar's colour (a JPEG's
/// noise, the thumbnail's averaging at the picture's edge).
pub const STILL_BAR_TOL: i32 = 8;
/// The share of a line's pixels that must be the bar's colour.
pub const STILL_BAR_SHARE: f64 = 0.97;

/// The depth of the bands of one colour at both ends of `lines`, when
/// there are such bands and they make bars.
fn uniform_band(lines: &[Vec<u8>]) -> Option<(usize, usize)> {
    let n = lines.len();
    if n < 8 {
        return None;
    }
    let colour = |l: &Vec<u8>| {
        let mut v = l.clone();
        v.sort_unstable();
        v[v.len() / 2] as i32
    };
    let is_bar = |l: &Vec<u8>, c: i32| {
        let near = l
            .iter()
            .filter(|&&v| (v as i32 - c).abs() <= STILL_BAR_TOL)
            .count();
        near as f64 >= STILL_BAR_SHARE * l.len() as f64
    };
    let (c0, c1) = (colour(&lines[0]), colour(&lines[n - 1]));
    if (c0 - c1).abs() > STILL_BAR_TOL {
        return None;
    }
    let lead = lines.iter().take_while(|l| is_bar(l, c0)).count();
    let trail = lines.iter().rev().take_while(|l| is_bar(l, c1)).count();
    let min = ((crate::video::BAR_MIN_SHARE * n as f64).ceil() as usize).max(2);
    if lead < min || trail < min || lead + trail >= n {
        return None;
    }
    if lead.abs_diff(trail) > min {
        return None;
    }
    if ((n - lead - trail) as f64) < crate::video::BAR_KEEP_MIN * n as f64 {
        return None;
    }
    Some((lead, trail))
}

/// The picture's own pixels, for "most pixels": inside its flat bars when
/// it has any, and then a fiftieth less, so a padded copy outranks the
/// picture it was padded from only with clearly more picture (its bars
/// are found at 256 wide, a line either way). [CLAUDE]
pub fn picture_pixels(h: &Hashed) -> f64 {
    let px = h.sig.pixels() as f64;
    match h.bars {
        Some([l, t, r, b]) => px * ((r - l) * (b - t)) as f64 * 0.98,
        None => px,
    }
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
    let (kp, mp) = (&k.rep.cand.path, &m.rep.cand.path);
    let frame = capture::different_moments(k.capture.as_ref(), m.capture.as_ref()) == Some(false)
        && numbered_apart(kp, mp);
    let kin =
        frame || capture::different_moments(k.capture.as_ref(), m.capture.as_ref()) == Some(true);
    let mut r = relation_deep(&k.sig, &m.sig, cos, kin);
    r.frame = frame;
    r.raw = m.raw;
    r.keeper_raw = k.raw;
    r.pair = !kin && k.raw != m.raw && twins(kp, mp);
    r.boxed = Boxed::of(k.bars, m.bars);
    r
}

/// Two names a camera numbered apart: the same once every run of digits
/// is set aside, with at least one of those numbers different —
/// `IMG_0012` / `IMG_0013`, `DSC00012` / `DSC00013`,
/// `20230101_120000_001` / `…_002`, `payload_N000` / `payload_N001`.
/// A copy's name changes shape instead (`IMG_0012 (1)`, `IMG_0012 2`,
/// `IMG_0012-2`, `IMG_0012 - Copy`), and a format conversion keeps the
/// number (`IMG_0012.png`). Letters compare without case; numbers by
/// value. [CLAUDE, Phase 11]
pub fn numbered_apart(a: &std::path::Path, b: &std::path::Path) -> bool {
    fn runs(p: &std::path::Path) -> Option<(Vec<String>, Vec<u128>)> {
        let stem = p.file_stem()?.to_string_lossy().to_lowercase();
        let (mut words, mut nums) = (vec![String::new()], Vec::new());
        let mut digits = String::new();
        for c in stem.chars() {
            if c.is_ascii_digit() {
                digits.push(c);
                continue;
            }
            if !digits.is_empty() {
                nums.push(digits.parse().ok()?);
                digits.clear();
                words.push(String::new());
            }
            words.last_mut()?.push(c);
        }
        if !digits.is_empty() {
            nums.push(digits.parse().ok()?);
            words.push(String::new());
        }
        Some((words, nums))
    }
    match (runs(a), runs(b)) {
        (Some((wa, na)), Some((wb, nb))) => wa == wb && na.len() == nb.len() && na != nb,
        _ => false,
    }
}

/// A camera writes a RAW and its JPEG side by side under one name
/// (`IMG_0041.CR2`, `IMG_0041.JPG`): same folder, same stem, the case of
/// either ignored.
pub fn twins(a: &std::path::Path, b: &std::path::Path) -> bool {
    let stem = |p: &std::path::Path| p.file_stem().map(|s| s.to_string_lossy().to_lowercase());
    a.parent() == b.parent() && stem(a).is_some() && stem(a) == stem(b)
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
    let (variant, d, p, q) = best.expect("invariant: VARIANT_ORDER is not empty");
    Relation {
        variant,
        d,
        p,
        q,
        q_quality: k.pdq_quality.min(m.pdq_quality),
        deep,
        fit: None,
        kin,
        raw: false,
        keeper_raw: false,
        pair: false,
        frame: false,
        boxed: Boxed::No,
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

#[cfg(test)]
mod words_tests {
    use super::*;

    fn rel(d: u32, p: u32, q: u32) -> Relation {
        Relation {
            variant: Variant::Identity,
            d,
            p,
            q,
            q_quality: 100,
            deep: None,
            fit: None,
            kin: false,
            raw: false,
            keeper_raw: false,
            pair: false,
            frame: false,
            boxed: Boxed::No,
        }
    }

    #[test]
    fn plain_words_for_plain_cases() {
        let k = (1000, 750);
        assert_eq!(rel(0, 0, 0).words_for(k, k, None, None), "same picture");
        assert_eq!(rel(1, 0, 6).words_for(k, k, None, None), "re-encoded");
        assert_eq!(rel(4, 2, 22).words_for(k, k, None, None), "near copy");
        assert_eq!(
            rel(0, 0, 4).words_for(k, (500, 375), None, None),
            "resized 50%"
        );
        assert_eq!(
            rel(0, 0, 4).words_for(k, (1500, 1125), None, None),
            "resized 150%"
        );
        // an odd shape with no fit consulted: the size itself
        assert_eq!(rel(0, 0, 4).words_for(k, (800, 750), None, None), "800×750");
        let mut r = rel(0, 0, 6);
        r.variant = Variant::Rotate90;
        assert_eq!(r.words_for(k, (750, 1000), None, None), "rotated 90°");
        r.variant = Variant::FlipH;
        assert_eq!(r.words_for(k, k, None, None), "mirrored");
        r.variant = Variant::FlipH;
        assert_eq!(
            r.words_for(k, (500, 375), None, None),
            "mirrored, resized 50%"
        );
    }

    #[test]
    fn numbered_apart_tells_frames_from_copies() {
        use std::path::Path;
        let ap = |a: &str, b: &str| numbered_apart(Path::new(a), Path::new(b));
        // frames a camera numbered
        for (a, b) in [
            ("d/IMG_0012.JPG", "d/IMG_0013.JPG"),
            ("d/DSC00012.JPG", "e/dsc00013.jpg"),
            ("d/20230101_120000_001.jpg", "d/20230101_120000_002.jpg"),
            ("burst/payload_N000.jpg", "burst/payload_N003.jpg"),
            ("d/P1000123.JPG", "d/P1000124.JPG"),
        ] {
            assert!(ap(a, b), "{a} {b}");
        }
        // copies, conversions, and names with no number
        for (a, b) in [
            ("d/IMG_0012.JPG", "d/IMG_0012 (1).JPG"),
            ("d/IMG_0012.JPG", "d/IMG_0012 2.JPG"),
            ("d/IMG_0012.JPG", "d/IMG_0012-2.JPG"),
            ("d/IMG_0012.JPG", "d/IMG_0012 - Copy.JPG"),
            ("d/IMG_0012.JPG", "exports/IMG_0012.png"),
            ("d/IMG_0012.JPG", "d/img_0012.jpg"),
            ("d/IMG_0012.JPG", "d/IMG_E0012.JPG"),
            ("d/beach.jpg", "d/beach_small.jpg"),
            ("d/beach.jpg", "e/beach.jpg"),
            ("d/0012.jpg", "d/012.jpg"),
        ] {
            assert!(!ap(a, b), "{a} {b}");
        }
    }

    #[test]
    fn fit_words_and_the_frames() {
        let k = (1000, 750);
        let mut r = rel(10, 6, 32);
        r.fit = Some(Fit {
            ncc: 1.0,
            scale: 0.95,
            dx: 0,
            dy: 0,
            angle: 0.0,
            inner_is_second: true,
            ..Fit::none()
        });
        assert_eq!(r.words_for(k, (950, 712), None, None), "cropped 95%");
        r.fit = Some(Fit {
            ncc: 0.97,
            scale: 0.8,
            dx: 0,
            dy: 0,
            angle: 0.0,
            inner_is_second: false,
            ..Fit::none()
        });
        assert_eq!(r.words_for(k, (1000, 1000), None, None), "letterboxed");
        assert_eq!(r.words_for(k, (1100, 825), None, None), "with a border");
        r.fit = Some(Fit {
            ncc: 0.95,
            scale: 1.0,
            dx: 0,
            dy: 0,
            angle: 4.0,
            inner_is_second: true,
            ..Fit::none()
        });
        assert_eq!(r.words_for(k, k, None, None), "rotated +4°");
        // the fit fell short: listed, not acted on
        r.fit = Some(Fit {
            ncc: 0.8,
            scale: 1.0,
            dx: 0,
            dy: 0,
            angle: 0.0,
            inner_is_second: true,
            ..Fit::none()
        });
        assert_eq!(r.words_for(k, k, None, None), "similar, unconfirmed");
    }

    #[test]
    fn moments_and_the_descriptor() {
        let k = (1000, 750);
        let mut r = rel(1, 2, 4);
        r.kin = true;
        assert_eq!(
            r.words_for(
                k,
                k,
                Some("2021-03-04 05:06:07"),
                Some("2021-03-04 05:06:08")
            ),
            "another shot, 05:06:08"
        );
        assert_eq!(
            r.words_for(
                k,
                k,
                Some("2021-03-04 05:06:07"),
                Some("2021-03-05 09:00:00")
            ),
            "another shot, 2021-03-05"
        );
        let mut d = rel(30, 28, 120);
        d.deep = Some(0.8);
        assert_eq!(d.words_for(k, k, None, None), "looks alike");
        d.fit = Some(Fit {
            ncc: 0.96,
            scale: 1.0,
            dx: 0,
            dy: 0,
            angle: 0.0,
            inner_is_second: true,
            ..Fit::none()
        });
        assert_eq!(d.words_for(k, k, None, None), "edited");
    }
}

#[cfg(test)]
mod bars_tests {
    use super::*;
    use image::{DynamicImage, Rgb, RgbImage};

    /// A textured picture: rings and stripes, detail everywhere.
    fn picture(w: u32, h: u32) -> RgbImage {
        RgbImage::from_fn(w, h, |x, y| {
            let (fx, fy) = (x as f32 / w as f32, y as f32 / h as f32);
            let v = 128.0
                + 60.0 * (fx * 41.0).sin() * (fy * 29.0).cos()
                + 50.0 * ((fx - 0.4).hypot(fy - 0.6) * 90.0).sin();
            let c = v.clamp(0.0, 255.0) as u8;
            Rgb([c, c / 2 + 40, 255 - c])
        })
    }

    /// `pic` centred on a canvas of `w × h` in `colour`.
    fn boxed(pic: &RgbImage, w: u32, h: u32, colour: [u8; 3]) -> RgbImage {
        let mut c = RgbImage::from_pixel(w, h, Rgb(colour));
        image::imageops::overlay(
            &mut c,
            pic,
            ((w - pic.width()) / 2) as i64,
            ((h - pic.height()) / 2) as i64,
        );
        c
    }

    fn hashed(name: &str, img: &RgbImage) -> Hashed {
        let d = DynamicImage::ImageRgb8(img.clone());
        Hashed {
            rep: Rep {
                cand: crate::walk::Candidate {
                    path: std::path::PathBuf::from(name),
                    size: 1000,
                    mtime: std::time::SystemTime::UNIX_EPOCH,
                },
                exact_group: None,
            },
            sig: perceptual::signature_from_image(&d),
            deep: None,
            capture: None,
            raw: false,
            bars: picture_bars(&d),
        }
    }

    #[test]
    fn a_letterboxed_copy_never_outranks_its_picture() {
        let pic = picture(512, 512);
        assert_eq!(picture_bars(&DynamicImage::ImageRgb8(pic.clone())), None);
        // black, white and grey bars, above and below or left and right
        for (w, h, colour) in [
            (512, 640, [0, 0, 0]),
            (512, 640, [255, 255, 255]),
            (680, 512, [128, 128, 128]),
        ] {
            let b = boxed(&pic, w, h, colour);
            let [l, t, r, bt] =
                picture_bars(&DynamicImage::ImageRgb8(b.clone())).expect("bars found");
            let near = |a: f32, e: f32| (a - e).abs() < 0.02;
            assert!(
                near(l, (w - 512) as f32 / 2.0 / w as f32)
                    && near(t, (h - 512) as f32 / 2.0 / h as f32)
                    && near(r, 1.0 - (w - 512) as f32 / 2.0 / w as f32)
                    && near(bt, 1.0 - (h - 512) as f32 / 2.0 / h as f32),
                "{w}x{h}: {l} {t} {r} {bt}"
            );
            // the original keeps, whichever is older or named which way
            let (o, c) = (hashed("photos/pic.jpg", &pic), hashed("a/pic_box.png", &b));
            assert_eq!(keeper_order(&o, &c), std::cmp::Ordering::Less, "{w}x{h}");
            assert_eq!(keeper_order(&c, &o), std::cmp::Ordering::Greater, "{w}x{h}");
            // and the member is said to be what it is
            let want = if h > w { Boxed::Letter } else { Boxed::Pillar };
            assert_eq!(relation_of(&o, &c).boxed, want, "{w}x{h}");
            assert_eq!(relation_of(&c, &o).boxed, Boxed::Removed);
        }
        // more picture inside the bars than the bare file has: it keeps
        let big = image::imageops::resize(&pic, 1024, 1024, image::imageops::FilterType::Triangle);
        let up = hashed("up.png", &boxed(&big, 1024, 1280, [0, 0, 0]));
        let o = hashed("pic.jpg", &pic);
        assert_eq!(keeper_order(&up, &o), std::cmp::Ordering::Less);
        // a picture with a flat sky on one side only has no bars
        let mut sky = pic.clone();
        for y in 0..120 {
            for x in 0..512 {
                sky.put_pixel(x, y, Rgb([170, 200, 230]));
            }
        }
        assert_eq!(picture_bars(&DynamicImage::ImageRgb8(sky.clone())), None);
        // and boxed, the bar ends where the sky starts
        let [_, t, _, b] = picture_bars(&DynamicImage::ImageRgb8(boxed(&sky, 512, 640, [0, 0, 0])))
            .expect("bars above a sky");
        assert!((t - 0.1).abs() < 0.02 && (b - 0.9).abs() < 0.02, "{t} {b}");
    }

    #[test]
    fn the_bars_say_what_was_added_or_removed() {
        let k = (512, 512);
        let mut r = relation_deep(
            &perceptual::signature_from_image(&DynamicImage::ImageRgb8(picture(64, 64))),
            &perceptual::signature_from_image(&DynamicImage::ImageRgb8(picture(64, 64))),
            None,
            false,
        );
        for (b, w, m) in [
            (Boxed::Letter, "letterboxed", (512, 640)),
            (Boxed::Pillar, "pillarboxed", (680, 512)),
            (Boxed::Border, "with a border", (600, 600)),
            (Boxed::Removed, "bars removed", (512, 410)),
        ] {
            r.boxed = b;
            assert_eq!(r.words_for(k, m, None, None), w);
        }
    }
}
