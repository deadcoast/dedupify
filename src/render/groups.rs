//! The listing: exact, near and video groups, one shared date column per
//! listing (Phase 10), the member lines fitted to the width.

use std::path::Path;

use crate::exact::{Group, Tier};
use crate::near::NearGroup;
use crate::theme::Role;
use crate::util::{
    fit, fmt_bytes, fmt_clock, fmt_date, join_fit, rel_display, shorten_middle, width,
};
use crate::video::VideoGroup;

use super::{Renderer, EXACT_POST, ROW_LEAD};

/// One listing section's columns at each compaction level (index):
/// `pre` the cells between the path and the date-or-words column
/// (dimensions, size, ... with their gaps), `post` the widest date, words
/// and evidence after it, `room` the fewest path cells a level needs (the
/// last level needs none), `cost` what the level gives up: 1 the evidence
/// numbers, 2 the codec, 3 the dimensions.
struct Measure {
    pre: Vec<usize>,
    post: Vec<usize>,
    room: &'static [usize],
    cost: &'static [u8],
}

impl Measure {
    /// An empty section: no columns, never compacted.
    fn none() -> Measure {
        Measure {
            pre: vec![0],
            post: vec![0],
            room: &[],
            cost: &[0],
        }
    }
}

impl Renderer {
    /// The groups of one listing, paths relative to `root`: exact, then
    /// near, then video, numbered on from each other. One column holds
    /// every keeper's date and every member's words in all three blocks,
    /// so the eye finds them in one place; each block's path column takes
    /// what its own columns leave (docs/CLI.md §3).
    pub fn listing(&self, groups: &[Group], near: &[NearGroup], video: &[VideoGroup], root: &Path) {
        let n = self.near_section(near, root, 0, None);
        let v = self.video_section(video, root, 0, None);
        let exact_post = if groups.is_empty() { 0 } else { EXACT_POST };
        let col_at = |ln: usize, lv: usize| {
            let post = exact_post.max(n.post[ln]).max(v.post[lv]);
            self.theme.width.saturating_sub(post)
        };
        // every pair of levels is tried; the least severe one that leaves
        // each path its room wins: the worst column any block drops first
        // (evidence numbers before a codec, a codec before dimensions),
        // then the total. The last levels need no room.
        let mut best: Option<((u8, u8), usize, usize)> = None;
        for ln in 0..n.pre.len() {
            for lv in 0..v.pre.len() {
                let col = col_at(ln, lv);
                let fits = |m: &Measure, l: usize| {
                    l >= m.room.len() || col >= ROW_LEAD + m.pre[l] + m.room[l]
                };
                if !(fits(&n, ln) && fits(&v, lv)) {
                    continue;
                }
                let key = (n.cost[ln].max(v.cost[lv]), n.cost[ln] + v.cost[lv]);
                if best.is_none_or(|b| key < b.0) {
                    best = Some((key, ln, lv));
                }
            }
        }
        let (ln, lv) = best.map_or((n.pre.len() - 1, v.pre.len() - 1), |b| (b.1, b.2));
        let col = col_at(ln, lv);
        self.exact_section(groups, root, col);
        self.near_section(near, root, groups.len(), Some((col, ln)));
        self.video_section(video, root, groups.len() + near.len(), Some((col, lv)));
    }

    /// Exact groups; the keeper's date starts at `col`.
    fn exact_section(&self, groups: &[Group], root: &Path, col: usize) {
        let t = &self.theme;
        if groups.is_empty() {
            return;
        }
        self.blank();
        let digits = groups.len().to_string().len().max(3);
        for (i, g) in groups.iter().enumerate() {
            let n = format!("{:0width$}", i + 1, width = digits);
            let head = match g.tier {
                Tier::Full => t.paint_bold(Role::Plate, &format!("GROUP {n}")),
                Tier::Sample => t.paint_bold(Role::Warn, &format!("LIKELY {n}")),
            };
            let noun = if g.tier == Tier::Full {
                "copies"
            } else {
                "files"
            };
            let copies = format!(
                "{} {noun}",
                t.paint(Role::Info, &g.members.len().to_string())
            );
            let each = format!("{} each", t.paint(Role::Plate, &fmt_bytes(g.size)));
            let recl = format!(
                "{} reclaimable",
                t.paint(Role::Info, &fmt_bytes(g.reclaimable()))
            );
            let tail = if g.tier == Tier::Sample {
                t.paint(Role::Warn, "size + sample match")
            } else {
                String::new()
            };
            let mut parts = vec![head, copies, each, recl];
            if !tail.is_empty() {
                parts.push(tail);
            }
            // a short terminal loses the rightmost parts, never wraps
            self.line(&format!(" {}", join_fit(&parts, t.width.saturating_sub(1))));

            // member lines:  "   keep  <path>  <date>"
            let path_cells = col.saturating_sub(ROW_LEAD).max(12);
            for (j, m) in g.members.iter().enumerate() {
                let rel = rel_display(&m.path, root);
                let shown = shorten_middle(&rel, path_cells, t.glyphs.ellipsis);
                if j == 0 {
                    self.line(&format!(
                        "   {}  {}  {}",
                        t.paint_bold(Role::Keep, "keep"),
                        t.paint(Role::Plate, &fit(&shown, path_cells)),
                        t.paint(Role::Dupe, &fmt_date(m.mtime))
                    ));
                } else {
                    self.line(&format!(
                        "   {}  {}",
                        t.paint(Role::Dupe, "dupe"),
                        t.paint(Role::Dupe, &shown)
                    ));
                }
            }
        }
    }

    /// Near-duplicate groups. `offset` continues the numbering after the
    /// exact groups so a line in the output has one number. Without a
    /// `plan` (date column, compaction level) the section is measured, not
    /// printed.
    fn near_section(
        &self,
        groups: &[NearGroup],
        root: &Path,
        offset: usize,
        plan: Option<(usize, usize)>,
    ) -> Measure {
        let t = &self.theme;
        if groups.is_empty() {
            return Measure::none();
        }
        let digits = (offset + groups.len()).to_string().len().max(3);

        // Every member's tail (everything after the path) is built first,
        // plain for measuring and painted for printing, so the path column
        // is as wide as this list allows and no line exceeds the width.
        struct Tail {
            plain: String,
            painted: String,
        }
        // Two steps of compaction when the width is short: first the
        // evidence numbers go (the words stay), then the dimensions column,
        // so the path keeps at least 14 cells.
        let compact = std::cell::Cell::new(0usize);
        // a RAW shows `RAW` where the size goes: its preview's size is not
        // the picture's (Phase 10)
        let lead_cols = |w: u32, h: u32, raw: bool, size: u64| -> (String, String) {
            let mut cols: Vec<String> = Vec::new();
            if compact.get() < 2 {
                let dims = if raw {
                    "RAW".to_string()
                } else {
                    format!("{w}{}{h}", t.glyphs.times)
                };
                cols.push(fit(&dims, 11));
            }
            cols.push(fit(&fmt_bytes(size), 7));
            (
                cols.join("  "),
                cols.iter()
                    .map(|c| t.paint(Role::Dupe, c))
                    .collect::<Vec<_>>()
                    .join("  "),
            )
        };
        let keeper_tail = |c: &crate::walk::Candidate, w: u32, h: u32, raw: bool| -> Tail {
            let (lp, lq) = lead_cols(w, h, raw, c.size);
            let date = fmt_date(c.mtime);
            Tail {
                plain: format!("{lp}  {date}"),
                painted: format!("{lq}  {}", t.paint(Role::Dupe, &date)),
            }
        };
        // the words column is as wide as the longest words in the list
        let words_cells = groups
            .iter()
            .flat_map(|g| {
                let k = &g.members[0].hashed;
                g.members[1..].iter().map(move |m| {
                    width(
                        &m.relation
                            .expect("invariant: every member after the keeper is related to it")
                            .words(k, &m.hashed),
                    )
                })
            })
            .max()
            .unwrap_or(0);
        let member_tail = |m: &crate::near::NearMember, k: &crate::near::Hashed| -> Tail {
            let c = &m.hashed.rep.cand;
            let r = m
                .relation
                .expect("invariant: every member after the keeper is related to it");
            let (lp, lq) = lead_cols(
                m.hashed.sig.width,
                m.hashed.sig.height,
                m.hashed.raw,
                c.size,
            );
            // the words: what the member is. Lit in the info colour when the
            // member stays (another shot, the descriptor's word alone, or
            // nothing confirmed it), plain otherwise.
            let words = r.words(k, &m.hashed);
            let words_role = if r.listed_only() || !r.confirmed() {
                Role::Info
            } else {
                Role::Dupe
            };
            let mut plain = format!("{lp}  {}", fit(&words, words_cells));
            let mut painted = format!("{lq}  {}", t.paint(words_role, &fit(&words, words_cells)));
            // the evidence, dim, dropped first when the width is short
            if compact.get() < 1 {
                let numbers = r.numbers();
                plain.push_str(&format!("  {numbers}"));
                painted.push_str(&format!("  {}", t.paint(Role::Rule, &numbers)));
            }
            if let Some(eg) = m.hashed.rep.exact_group {
                let txt = format!("+GROUP {:0width$}", eg + 1, width = digits);
                plain.push_str(&format!("  {txt}"));
                painted.push_str(&format!("  {}", t.paint(Role::Dupe, &txt)));
            }
            Tail { plain, painted }
        };

        let widest = || {
            groups
                .iter()
                .flat_map(|g| {
                    let k = &g.members[0].hashed;
                    g.members.iter().enumerate().map(move |(j, m)| (j, m, k))
                })
                .map(|(j, m, k)| {
                    if j == 0 {
                        width(
                            &keeper_tail(
                                &m.hashed.rep.cand,
                                m.hashed.sig.width,
                                m.hashed.sig.height,
                                m.hashed.raw,
                            )
                            .plain,
                        )
                    } else {
                        width(&member_tail(m, k).plain)
                    }
                })
                .max()
                .unwrap_or(0)
        };
        // the columns before the date or words, with their gap
        let pre = || width(&lead_cols(0, 0, false, 0).0) + 2;
        // The path is the identifier: the evidence numbers go before it
        // drops under 32 cells, the dimensions before it drops under 18.
        let Some((col, level)) = plan else {
            let mut m = Measure {
                pre: Vec::new(),
                post: Vec::new(),
                room: &[32, 18],
                cost: &[0, 1, 3],
            };
            for l in 0..=m.room.len() {
                compact.set(l);
                m.pre.push(pre());
                m.post.push(widest().saturating_sub(pre()));
            }
            return m;
        };
        compact.set(level);
        let path_cells = col.saturating_sub(ROW_LEAD + pre()).max(12);
        self.blank();

        for (i, g) in groups.iter().enumerate() {
            let n = format!("{:0width$}", offset + i + 1, width = digits);
            let head = t.paint_bold(Role::Info, &format!("NEAR {n}"));
            let kinds = g.kinds();
            let mut count = format!(
                "{} similar",
                t.paint(Role::Info, &kinds.similar.to_string())
            );
            for (n, word) in [
                (kinds.deep, "deep"),
                (kinds.kin, "kin"),
                (kinds.pair, "pair"),
                (kinds.raw, "RAW"),
            ] {
                if n > 0 {
                    count.push_str(&format!(
                        "  {}",
                        t.paint(Role::Info, &format!("+{n} {word}"))
                    ));
                }
            }
            let recl = format!(
                "{} reclaimable",
                t.paint(Role::Info, &fmt_bytes(g.reclaimable()))
            );
            let mut parts = vec![head, count, recl];
            if let Some(k) = g.members.first() {
                if let Some(eg) = k.hashed.rep.exact_group {
                    parts.push(t.paint(
                        Role::Dupe,
                        &format!("keeper of GROUP {:0width$}", eg + 1, width = digits),
                    ));
                }
            }
            // a short terminal loses the rightmost parts, never wraps
            self.line(&format!(" {}", join_fit(&parts, t.width.saturating_sub(1))));

            let keeper = &g.members[0].hashed;
            for (j, m) in g.members.iter().enumerate() {
                let c = &m.hashed.rep.cand;
                let rel = rel_display(&c.path, root);
                let shown = fit(
                    &shorten_middle(&rel, path_cells, t.glyphs.ellipsis),
                    path_cells,
                );
                if j == 0 {
                    let tail =
                        keeper_tail(c, m.hashed.sig.width, m.hashed.sig.height, m.hashed.raw);
                    self.line(&format!(
                        "   {}  {}  {}",
                        t.paint_bold(Role::Keep, "keep"),
                        t.paint(Role::Plate, &shown),
                        tail.painted
                    ));
                } else {
                    let r = m
                        .relation
                        .expect("invariant: every member after the keeper is related to it");
                    let verb = match r.verb() {
                        "near" => t.paint(Role::Dupe, "near"),
                        other => t.paint(Role::Info, &fit(other, 4)),
                    };
                    let tail = member_tail(m, keeper);
                    self.line(&format!(
                        "   {}  {}  {}",
                        verb,
                        t.paint(Role::Dupe, &shown),
                        tail.painted
                    ));
                }
            }
        }
        Measure::none()
    }

    /// Video groups; numbering continues after exact and near groups.
    /// Measured, not printed, without a `plan` (as `near_section`).
    fn video_section(
        &self,
        groups: &[VideoGroup],
        root: &Path,
        offset: usize,
        plan: Option<(usize, usize)>,
    ) -> Measure {
        let t = &self.theme;
        if groups.is_empty() {
            return Measure::none();
        }
        let digits = (offset + groups.len()).to_string().len().max(3);

        struct Tail {
            plain: String,
            painted: String,
        }
        // "1920×1080  04:12  h264   1.1 GB" is common to both line kinds.
        // Compaction when the width is short: 1 drops the evidence numbers
        // (the words stay), 2 drops the codec, 3 drops the dimensions
        // (duration and size stay).
        let compact = std::cell::Cell::new(0usize);
        let meta_cols = |m: &crate::video::VideoMember| -> (String, String) {
            let c = &m.hashed.rep.cand;
            let meta = &m.hashed.sig.meta;
            let mut cols: Vec<String> = Vec::new();
            if compact.get() < 3 {
                cols.push(fit(
                    &format!("{}{}{}", meta.width, t.glyphs.times, meta.height),
                    11,
                ));
            }
            cols.push(fit(&fmt_clock(meta.duration), 6));
            if compact.get() < 2 {
                cols.push(fit(&meta.codec, 5));
            }
            cols.push(fit(&fmt_bytes(c.size), 7));
            (
                cols.join("  "),
                cols.iter()
                    .map(|c| t.paint(Role::Dupe, c))
                    .collect::<Vec<_>>()
                    .join("  "),
            )
        };
        let keeper_tail = |m: &crate::video::VideoMember| -> Tail {
            let (plain, painted) = meta_cols(m);
            let date = fmt_date(m.hashed.rep.cand.mtime);
            Tail {
                plain: format!("{plain}  {date}"),
                painted: format!("{painted}  {}", t.paint(Role::Dupe, &date)),
            }
        };
        let words_cells = groups
            .iter()
            .flat_map(|g| {
                let k = &g.members[0].hashed.sig;
                g.members[1..].iter().map(move |m| {
                    width(
                        &m.relation
                            .expect("invariant: every member after the keeper is related to it")
                            .words(k, &m.hashed.sig),
                    )
                })
            })
            .max()
            .unwrap_or(0);
        let member_tail =
            |m: &crate::video::VideoMember, keeper: &crate::video::VideoMember| -> Tail {
                let (mut plain, mut painted) = meta_cols(m);
                let r = m
                    .relation
                    .expect("invariant: every member after the keeper is related to it");
                // the words: what the member is; lit when nothing confirmed it
                let words = r.words(&keeper.hashed.sig, &m.hashed.sig);
                let words_role = if r.confirmed() {
                    Role::Dupe
                } else {
                    Role::Info
                };
                plain.push_str(&format!("  {}", fit(&words, words_cells)));
                painted.push_str(&format!(
                    "  {}",
                    t.paint(words_role, &fit(&words, words_cells))
                ));
                // the evidence, dim, dropped first
                if compact.get() < 1 {
                    let numbers = r.numbers();
                    plain.push_str(&format!("  {numbers}"));
                    painted.push_str(&format!("  {}", t.paint(Role::Rule, &numbers)));
                }
                if let Some(eg) = m.hashed.rep.exact_group {
                    let txt = format!("+GROUP {:0width$}", eg + 1, width = digits);
                    plain.push_str(&format!("  {txt}"));
                    painted.push_str(&format!("  {}", t.paint(Role::Dupe, &txt)));
                }
                Tail { plain, painted }
            };

        let widest = || {
            groups
                .iter()
                .flat_map(|g| {
                    let k = &g.members[0];
                    g.members.iter().enumerate().map(move |(j, m)| (j, m, k))
                })
                .map(|(j, m, k)| {
                    if j == 0 {
                        width(&keeper_tail(m).plain)
                    } else {
                        width(&member_tail(m, k).plain)
                    }
                })
                .max()
                .unwrap_or(0)
        };
        // the columns before the date or words, with their gap
        let pre = || width(&meta_cols(&groups[0].members[0]).0) + 2;
        // the path is the identifier: the evidence numbers go before it
        // drops under 32 cells, the codec and the dimensions under 18
        let Some((col, level)) = plan else {
            let mut m = Measure {
                pre: Vec::new(),
                post: Vec::new(),
                room: &[32, 18, 18],
                cost: &[0, 1, 2, 3],
            };
            for l in 0..=m.room.len() {
                compact.set(l);
                m.pre.push(pre());
                m.post.push(widest().saturating_sub(pre()));
            }
            return m;
        };
        compact.set(level);
        let path_cells = col.saturating_sub(ROW_LEAD + pre()).max(12);
        self.blank();

        for (i, g) in groups.iter().enumerate() {
            let n = format!("{:0width$}", offset + i + 1, width = digits);
            let head = t.paint_bold(Role::Info, &format!("VIDEO {n}"));
            let count = format!(
                "{} similar",
                t.paint(Role::Info, &(g.members.len() - 1).to_string())
            );
            let recl = format!(
                "{} reclaimable",
                t.paint(Role::Info, &fmt_bytes(g.reclaimable()))
            );
            let mut parts = vec![head, count, recl];
            if let Some(eg) = g.members[0].hashed.rep.exact_group {
                parts.push(t.paint(
                    Role::Dupe,
                    &format!("keeper of GROUP {:0width$}", eg + 1, width = digits),
                ));
            }
            // a short terminal loses the rightmost parts, never wraps
            self.line(&format!(" {}", join_fit(&parts, t.width.saturating_sub(1))));

            let keeper = &g.members[0];
            for (j, m) in g.members.iter().enumerate() {
                let c = &m.hashed.rep.cand;
                let rel = rel_display(&c.path, root);
                let shown = fit(
                    &shorten_middle(&rel, path_cells, t.glyphs.ellipsis),
                    path_cells,
                );
                if j == 0 {
                    self.line(&format!(
                        "   {}  {}  {}",
                        t.paint_bold(Role::Keep, "keep"),
                        t.paint(Role::Plate, &shown),
                        keeper_tail(m).painted
                    ));
                } else {
                    self.line(&format!(
                        "   {}  {}  {}",
                        t.paint(Role::Dupe, "near"),
                        t.paint(Role::Dupe, &shown),
                        member_tail(m, keeper).painted
                    ));
                }
            }
        }
        Measure::none()
    }
}
