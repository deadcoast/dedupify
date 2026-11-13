//! Totals and policy lines under a listing, and the action lines and
//! totals of `milk`, `purge` and undo.

use crate::exact::{Group, PipelineStats, Tier};
use crate::keeper::POLICY_LABEL;
use crate::near::{NearGroup, NEAR_POLICY_LABEL};
use crate::pdq::PDQ_MAX;
use crate::perceptual::{DHASH_MAX, PHASH_MAX};
use crate::theme::Role;
use crate::util::{
    counted, fit, fmt_bytes, fmt_count, pack, plural, shorten_middle, visible_width, width,
};
use crate::video::{VideoGroup, LEVEL2_CONFIRM, LEVEL2_MIN, VIDEO_POLICY_LABEL};
use crate::walk::ScanStats;

use super::fold::{self, Hue, Mark};
use super::{ActFold, Renderer};

impl Renderer {
    /// A totals line: the counts, then the policy and thresholds. When the
    /// whole does not fit the width, the policy part moves to a second line
    /// indented under the first; nothing wraps mid-word.
    fn totals_line(&self, counts: Vec<String>, policy: Vec<String>) {
        let max = self.theme.width.saturating_sub(1);
        let mut all = counts.clone();
        all.extend(policy.iter().cloned());
        let one = all.join("   ");
        if visible_width(&one) < max {
            self.line(&format!(" {one}"));
            return;
        }
        // the counts, then the policy indented under them; a part too long
        // for the width breaks at spaces (Phase 16: at 40 columns the near
        // policy ran 30 cells past the edge)
        for l in pack(&counts, max, |l| l.to_string()) {
            self.line(&format!(" {l}"));
        }
        for l in pack(&policy, max.saturating_sub(3), |l| {
            self.theme.paint(Role::Dupe, l)
        }) {
            self.line(&format!("   {l}"));
        }
    }

    /// `others` says whether near or video groups follow, so an empty exact
    /// list reads "no exact duplicates" rather than "no duplicates".
    pub fn totals(&self, groups: &[Group], scan: &ScanStats, others: bool) {
        let t = &self.theme;
        self.rule();
        if groups.is_empty() {
            self.line(&format!(
                " {}   {}   {} checked",
                t.paint_bold(
                    Role::Ok,
                    if others {
                        "no exact duplicates"
                    } else {
                        "no duplicates"
                    }
                ),
                t.paint(Role::Info, &counted(scan.files, "file", "files")),
                t.paint(Role::Info, &fmt_bytes(scan.bytes))
            ));
            return;
        }
        let dupes: u64 = groups.iter().map(|g| g.members.len() as u64 - 1).sum();
        let recl: u64 = groups.iter().map(Group::reclaimable).sum();
        let sample_only = groups.iter().any(|g| g.tier == Tier::Sample);
        self.totals_line(
            vec![
                format!(
                    "{} {}",
                    t.paint_bold(Role::Info, &fmt_count(groups.len() as u64)),
                    plural(groups.len() as u64, "group", "groups")
                ),
                format!(
                    "{} {}",
                    t.paint_bold(Role::Info, &fmt_count(dupes)),
                    plural(dupes, "duplicate", "duplicates")
                ),
                format!("{} reclaimable", t.paint_bold(Role::Info, &fmt_bytes(recl))),
            ],
            vec![t.paint(Role::Dupe, POLICY_LABEL)],
        );
        if sample_only {
            self.line(&format!(
                " {}",
                t.paint(
                    Role::Warn,
                    "quick search: matches are size + sample, not full content"
                )
            ));
        }
    }

    pub fn near_totals(&self, groups: &[NearGroup]) {
        let t = &self.theme;
        if groups.is_empty() {
            return;
        }
        let sum = |f: fn(&crate::near::Kinds) -> usize| -> u64 {
            groups.iter().map(|g| f(&g.kinds()) as u64).sum()
        };
        let deep_only = sum(|k| k.deep);
        let kin = sum(|k| k.kin);
        let pairs = sum(|k| k.pair);
        let raws = sum(|k| k.raw);
        let similar = sum(|k| k.similar);
        let recl: u64 = groups.iter().map(NearGroup::reclaimable).sum();
        if deep_only > 0 {
            self.totals_line(
                vec![format!(
                    "{} {} by deep only",
                    t.paint_bold(Role::Info, &fmt_count(deep_only)),
                    plural(deep_only, "image", "images")
                )],
                vec![t.paint(Role::Dupe, "listed for review; milk and purge leave them")],
            );
        }
        if kin > 0 {
            self.totals_line(
                vec![format!("{} kin", t.paint_bold(Role::Info, &fmt_count(kin)))],
                vec![t.paint(
                    Role::Dupe,
                    "another shot: another moment, or another frame of its second; left alone",
                )],
            );
        }
        if pairs > 0 {
            self.totals_line(
                vec![format!(
                    "{} RAW + JPEG",
                    t.paint_bold(Role::Info, &fmt_count(pairs))
                )],
                vec![t.paint(
                    Role::Dupe,
                    "the camera's JPEG beside its RAW: kept with it; choose it by hand in the workspace",
                )],
            );
        }
        if raws > 0 {
            self.totals_line(
                vec![format!(
                    "{} RAW",
                    t.paint_bold(Role::Info, &fmt_count(raws))
                )],
                vec![t.paint(
                    Role::Dupe,
                    "a camera RAW grouped under another file: never moved or binned",
                )],
            );
        }
        let deep_on = groups
            .iter()
            .flat_map(|g| g.members.iter())
            .any(|m| m.relation.is_some_and(|r| r.deep.is_some()));
        let deep_txt = if deep_on {
            format!(
                "  deep {ge}{:.2} lists, {ge}{:.2} with the fit",
                crate::deep::DEEP_MIN,
                crate::deep::DEEP_FIT_MIN,
                ge = t.glyphs.ge
            )
        } else {
            String::new()
        };
        self.totals_line(
            vec![
                format!(
                    "{} near {}",
                    t.paint_bold(Role::Info, &fmt_count(groups.len() as u64)),
                    plural(groups.len() as u64, "group", "groups")
                ),
                format!("{} similar", t.paint_bold(Role::Info, &fmt_count(similar))),
                format!("{} reclaimable", t.paint_bold(Role::Info, &fmt_bytes(recl))),
            ],
            vec![
                t.paint(Role::Dupe, NEAR_POLICY_LABEL),
                t.paint(
                    Role::Dupe,
                    &format!(
                        "dHash {le}{DHASH_MAX}  pHash {le}{PHASH_MAX}{deep_txt}  PDQ {le}{PDQ_MAX} or fit {ge}{:.2} to act",
                        crate::geometry::FIT_MIN,
                        le = t.glyphs.le,
                        ge = t.glyphs.ge
                    ),
                ),
            ],
        );
    }

    pub fn video_totals(&self, groups: &[VideoGroup]) {
        let t = &self.theme;
        if groups.is_empty() {
            return;
        }
        let similar: u64 = groups.iter().map(|g| g.members.len() as u64 - 1).sum();
        let recl: u64 = groups.iter().map(VideoGroup::reclaimable).sum();
        self.totals_line(
            vec![
                format!(
                    "{} video {}",
                    t.paint_bold(Role::Info, &fmt_count(groups.len() as u64)),
                    plural(groups.len() as u64, "group", "groups")
                ),
                format!("{} similar", t.paint_bold(Role::Info, &fmt_count(similar))),
                format!("{} reclaimable", t.paint_bold(Role::Info, &fmt_bytes(recl))),
            ],
            vec![
                t.paint(Role::Dupe, VIDEO_POLICY_LABEL),
                t.paint(
                    Role::Dupe,
                    &format!(
                        "tmk {ge}{LEVEL2_MIN:.1} groups, {ge}{LEVEL2_CONFIRM:.1} bins, or frames q {le}{}",
                        crate::pdq::PDQ_MAX,
                        ge = t.glyphs.ge,
                        le = t.glyphs.le
                    ),
                ),
            ],
        );
    }

    /// The action's line folds: what it took and where, what it could not
    /// take; then the files it could not, on one line, with their reasons.
    fn act_settle(&self, a: ActFold, verb_past: &str, n: u64, bytes: u64, destination: &str) {
        let t = &self.theme;
        let mut segs = vec![
            format!(
                "{} {}",
                fold::figure(t, &fmt_count(n)),
                fold::dim(t, verb_past)
            ),
            format!(
                "{} {}",
                fold::figure(t, &fmt_bytes(bytes)),
                fold::dim(t, destination)
            ),
        ];
        if !a.failed.is_empty() {
            segs.insert(
                1,
                t.paint(
                    Role::Danger,
                    &format!("{} could not be", fmt_count(a.failed.len() as u64)),
                ),
            );
        }
        if a.left > 0 {
            segs.push(fold::dim(
                t,
                &format!("{} left in place", fmt_count(a.left)),
            ));
        }
        let mark = match (n, a.failed.len()) {
            (0, 0) => Mark::Empty,
            (0, _) => Mark::Failed,
            (_, 0) => Mark::Done,
            _ => Mark::Warn,
        };
        fold::settle(t, a.hue, mark, &segs);
        if !a.failed.is_empty() {
            let names: Vec<String> = a
                .failed
                .iter()
                .map(|(p, why)| {
                    format!(
                        "{} {}",
                        t.paint(Role::Plate, &shorten_middle(p, 40, t.glyphs.ellipsis)),
                        fold::dim(t, why)
                    )
                })
                .collect();
            let lead = t.paint(Role::Danger, "   not taken  ");
            let room = t.width.saturating_sub(1 + 14);
            self.line(&format!("{lead}{}", fold::fit_segments(t, &names, room)));
        }
    }

    pub fn skipped(&self, scan: &ScanStats, p: &PipelineStats) {
        let t = &self.theme;
        let mut v: Vec<String> = Vec::new();
        if p.unreadable > 0 {
            v.push(format!("{} unreadable", fmt_count(p.unreadable)));
        }
        if scan.errors > 0 {
            v.push(format!("{} not listed", fmt_count(scan.errors)));
        }
        if p.hardlinks_collapsed > 0 {
            v.push(format!(
                "{} hard links collapsed",
                fmt_count(p.hardlinks_collapsed)
            ));
        }
        if scan.empty > 0 {
            v.push(format!("{} empty", fmt_count(scan.empty)));
        }
        if !v.is_empty() {
            self.line(&format!(
                " {}  {}",
                t.paint(Role::Dupe, "skipped"),
                t.paint(Role::Dupe, &v.join("   "))
            ));
        }
    }

    /// An action begins: in the fold, its line goes live, `total` files to
    /// take. (The full listing prints a line per file instead.)
    pub(crate) fn act_open(&self, hue: Hue, total: u64, dry_run: bool) {
        if !self.folding() {
            return;
        }
        let phase = match (hue, dry_run) {
            (Hue::Milk, false) => "moving",
            (Hue::Milk, true) => "would move",
            (Hue::Purge, false) => "binning",
            (Hue::Purge, true) => "would bin",
            (_, false) => "restoring",
            (_, true) => "would restore",
        };
        fold::open(&self.theme, hue);
        fold::step(phase, 0, Some(total), String::new());
        *self.act.borrow_mut() = Some(ActFold {
            hue,
            total,
            done: 0,
            phase,
            failed: Vec::new(),
            left: 0,
        });
    }

    /// One condensed line per action: verb, path, and a right-aligned
    /// detail. The path is shortened first; the detail only when the path
    /// is already at its minimum. In the fold, the action's live line
    /// counts it and names the file in hand.
    pub fn action(&self, verb: &str, role: Role, path: &str, detail: &str) {
        if let Some(a) = self.act.borrow_mut().as_mut() {
            match verb {
                "failed" => {
                    a.failed.push((path.to_string(), detail.to_string()));
                    a.done += 1;
                }
                "kept" | "kept~" | "exists" => a.left += 1,
                _ => a.done += 1,
            }
            fold::step(
                a.phase,
                a.done,
                Some(a.total.max(a.done)),
                format!("· {}", shorten_middle(path, 48, self.theme.glyphs.ellipsis)),
            );
            return;
        }
        let t = &self.theme;
        let verb_cells = 8;
        if detail.is_empty() {
            let path_cells = t.width.saturating_sub(1 + verb_cells + 2 + 1).max(12);
            self.line(&format!(
                " {}  {}",
                t.paint(role, &fit(verb, verb_cells)),
                t.paint(
                    Role::Plate,
                    &shorten_middle(path, path_cells, t.glyphs.ellipsis)
                )
            ));
            return;
        }
        let fixed = 1 + verb_cells + 2 + 2 + 1;
        let mut detail_cells = width(detail);
        let mut path_cells = t.width.saturating_sub(fixed + detail_cells);
        if path_cells < 12 {
            path_cells = 12;
            detail_cells = t.width.saturating_sub(fixed + path_cells).max(8);
        }
        let shown = shorten_middle(path, path_cells, t.glyphs.ellipsis);
        let det = shorten_middle(detail, detail_cells, t.glyphs.ellipsis);
        self.line(&format!(
            " {}  {}  {}",
            t.paint(role, &fit(verb, verb_cells)),
            t.paint(Role::Plate, &fit(&shown, path_cells)),
            t.paint(Role::Dupe, &det)
        ));
    }

    /// `destination` names where the files went; `short_destination`, when
    /// given, replaces it if the line would not fit the width (`into
    /// _milked` for `into D:\long\path\_milked`).
    pub fn action_totals(
        &self,
        verb_past: &str,
        n: u64,
        bytes: u64,
        destination: &str,
        short_destination: Option<&str>,
        journal_id: Option<&str>,
    ) {
        let t = &self.theme;
        if let Some(a) = self.act.borrow_mut().take() {
            self.act_settle(
                a,
                verb_past,
                n,
                bytes,
                short_destination.unwrap_or(destination),
            );
            return;
        }
        self.rule();
        let lead = format!(" {} {verb_past}   {} ", fmt_count(n), fmt_bytes(bytes));
        let destination = match short_destination {
            Some(short) if width(&lead) + width(destination) > t.width => short,
            _ => destination,
        };
        let counts = vec![
            format!("{} {verb_past}", t.paint_bold(Role::Info, &fmt_count(n))),
            format!(
                "{} {destination}",
                t.paint_bold(Role::Info, &fmt_bytes(bytes))
            ),
        ];
        let journal = match journal_id {
            Some(id) => vec![format!(
                "{} {}",
                t.paint(Role::Dupe, "journal"),
                t.paint(Role::Dupe, id)
            )],
            None => Vec::new(),
        };
        self.totals_line(counts, journal);
    }
}
