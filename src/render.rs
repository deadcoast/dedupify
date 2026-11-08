//! Output (docs/CLI.md §3). Every line starts with one space of margin, is
//! measured in display cells and fitted to the terminal width.

use std::io::{self, Write};
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::cli::{Command, Invocation};
use crate::exact::{Group, PipelineStats, Progress, Stage, Tier};
use crate::keeper::POLICY_LABEL;
use crate::near::{NearGroup, NearStats, NEAR_POLICY_LABEL};
use crate::pdq::PDQ_MAX;
use crate::perceptual::{DHASH_MAX, PHASH_MAX};
use crate::theme::{Role, Theme};
use crate::util::{
    counted, fit, fmt_bytes, fmt_clock, fmt_count, fmt_date, fmt_duration, join_fit, plural,
    rel_display, shorten_middle, under_root, visible_width, width,
};
use crate::video::{VideoGroup, VideoStats, LEVEL2_CONFIRM, LEVEL2_MIN, VIDEO_POLICY_LABEL};
use crate::walk::ScanStats;
use crate::MILKED_DIR;

pub const BAR_CELLS: usize = 30;
const REDRAW_EVERY: Duration = Duration::from_millis(40);
/// Cells a stage line spends before its text: margin, label, space, bar, gap.
const STAGE_LEAD: usize = 1 + 6 + 1 + BAR_CELLS + 2;
/// The verb column of the `next` / `near` / `video` / `undo` lines.
const HINT_VERB: usize = 5;

// ---------------------------------------------------------------------------
// live progress (Sync — ticked from rayon workers)
// ---------------------------------------------------------------------------

struct StageLine {
    stage: Stage,
    done: u64,
    total: Option<u64>,
    started: Instant,
    text: String,
    finished: bool,
}

struct ProgressState {
    current: Option<StageLine>,
    last_draw: Instant,
    ticks: u64,
}

pub struct LiveProgress {
    theme: Theme,
    state: Mutex<ProgressState>,
}

impl LiveProgress {
    pub fn new(theme: Theme) -> LiveProgress {
        LiveProgress {
            theme,
            state: Mutex::new(ProgressState {
                current: None,
                last_draw: Instant::now(),
                ticks: 0,
            }),
        }
    }

    /// Open a stage with an unknown total (SCAN).
    pub fn begin(&self, stage: Stage) {
        let mut st = self.state.lock().unwrap();
        st.current = Some(StageLine {
            stage,
            done: 0,
            total: None,
            started: Instant::now(),
            text: String::new(),
            finished: false,
        });
        st.ticks = 0;
        self.draw(&mut st, false);
    }

    pub fn update_text(&self, stage: Stage, done: u64, text: String) {
        let mut st = self.state.lock().unwrap();
        match &mut st.current {
            Some(c) if c.stage == stage => {
                c.done = done;
                c.text = text;
            }
            _ => {
                st.current = Some(StageLine {
                    stage,
                    done,
                    total: None,
                    started: Instant::now(),
                    text,
                    finished: false,
                });
            }
        }
        st.ticks += 1;
        self.draw(&mut st, false);
    }

    /// Close the current stage with its final text; the line stays.
    pub fn finish(&self, stage: Stage, text: String) {
        let mut st = self.state.lock().unwrap();
        let started = match &st.current {
            Some(c) if c.stage == stage => c.started,
            _ => Instant::now(),
        };
        st.current = Some(StageLine {
            stage,
            done: 1,
            total: Some(1),
            started,
            text,
            finished: true,
        });
        self.draw(&mut st, true);
        st.current = None;
    }

    fn draw(&self, st: &mut ProgressState, force: bool) {
        let t = &self.theme;
        let Some(c) = &st.current else { return };
        if !t.live && !c.finished {
            return;
        }
        if !force && st.last_draw.elapsed() < REDRAW_EVERY {
            return;
        }
        st.last_draw = Instant::now();

        let filled = match (c.total, c.finished) {
            (_, true) => BAR_CELLS,
            (Some(total), _) if total > 0 => {
                ((c.done as f64 / total as f64) * BAR_CELLS as f64).round() as usize
            }
            _ => usize::MAX, // sweep
        };
        let bar = if filled == usize::MAX {
            sweep(t, st.ticks)
        } else {
            let on = filled.min(BAR_CELLS);
            format!(
                "{}{}",
                t.paint(
                    if c.finished { Role::Ok } else { Role::Keep },
                    &t.glyphs.bar_on.repeat(on)
                ),
                t.paint(Role::Rule, &t.glyphs.bar_off.repeat(BAR_CELLS - on))
            )
        };
        let label = fit(c.stage.label(), 6);
        let label = if c.finished {
            t.paint(Role::Plate, &label)
        } else {
            t.paint_bold(Role::Plate, &label)
        };
        // the text is segments joined by three spaces; drop from the right
        // rather than wrap, which would leave the in-place redraw behind
        let segs: Vec<String> = c.text.split("   ").map(str::to_string).collect();
        let text = join_fit(&segs, t.width.saturating_sub(STAGE_LEAD + 1));
        let line = format!(" {label} {bar}  {text}");
        let mut out = io::stdout().lock();
        if t.live {
            let _ = write!(out, "\r\x1b[2K{line}");
            if c.finished {
                let _ = writeln!(out);
            }
        } else if c.finished {
            let _ = writeln!(out, "{line}");
        }
        let _ = out.flush();
    }
}

fn sweep(t: &Theme, ticks: u64) -> String {
    let len = 6usize;
    let span = BAR_CELLS - len;
    let period = span * 2;
    let p = (ticks as usize) % period;
    let pos = if p <= span { p } else { period - p };
    format!(
        "{}{}{}",
        t.paint(Role::Rule, &t.glyphs.bar_off.repeat(pos)),
        t.paint(Role::Keep, &t.glyphs.bar_on.repeat(len)),
        t.paint(Role::Rule, &t.glyphs.bar_off.repeat(BAR_CELLS - len - pos))
    )
}

impl LiveProgress {
    pub fn stage_summary(&self, stage: Stage, p: &PipelineStats, elapsed: Duration) -> String {
        let t = &self.theme;
        match stage {
            Stage::Size => format!(
                "{} share a size",
                t.paint(Role::Info, &fmt_count(p.size_sharing))
            ),
            Stage::Sample => format!(
                "{} share a sample   {}",
                t.paint(Role::Info, &fmt_count(p.sample_sharing)),
                t.paint(Role::Dupe, &fmt_bytes(p.bytes_sampled))
            ),
            Stage::Hash => format!(
                "{} confirmed   {}   {}",
                t.paint(Role::Info, &fmt_count(p.confirmed)),
                t.paint(Role::Dupe, &fmt_bytes(p.bytes_hashed)),
                t.paint(Role::Dupe, &fmt_duration(elapsed))
            ),
            Stage::Verify => format!(
                "{} verified byte for byte",
                t.paint(Role::Info, &fmt_count(p.confirmed))
            ),
            Stage::Scan | Stage::Decode | Stage::Match | Stage::Frames => String::new(),
        }
    }

    /// Near-pass stage texts; `actions` closes these stages with `finish`.
    pub fn decode_summary(&self, n: &NearStats, elapsed: Duration) -> String {
        let t = &self.theme;
        if n.decoded < 2 {
            return format!(
                "{} {}   {}",
                t.paint(Role::Info, &fmt_count(n.decoded)),
                plural(n.decoded, "image", "images"),
                t.paint(Role::Dupe, "nothing to compare")
            );
        }
        let mut v = vec![format!(
            "{} {} hashed",
            t.paint(Role::Info, &fmt_count(n.decoded)),
            plural(n.decoded, "image", "images")
        )];
        if n.deep {
            v.push(t.paint(Role::Dupe, "+deep"));
        }
        if n.deep_failed > 0 {
            v.push(t.paint(
                Role::Dupe,
                &format!("{} embeddings failed", fmt_count(n.deep_failed)),
            ));
        }
        if n.undecodable > 0 {
            v.push(t.paint(
                Role::Dupe,
                &format!("{} undecodable", fmt_count(n.undecodable)),
            ));
        }
        if n.low_quality > 0 {
            v.push(t.paint(
                Role::Dupe,
                &format!("{} flat, skipped", fmt_count(n.low_quality)),
            ));
        }
        v.push(t.paint(Role::Dupe, &fmt_duration(elapsed)));
        v.join("   ")
    }

    pub fn match_summary(&self, n: &NearStats, elapsed: Duration) -> String {
        let t = &self.theme;
        let mut parts = vec![format!(
            "{} similar pairs",
            t.paint(Role::Info, &fmt_count(n.pairs))
        )];
        if n.deep {
            parts.push(format!(
                "{} added by deep",
                t.paint(Role::Info, &fmt_count(n.deep_pairs))
            ));
        }
        parts.push(format!(
            "{} images grouped",
            t.paint(Role::Info, &fmt_count(n.grouped))
        ));
        if n.fits > 0 {
            parts.push(format!(
                "{} fits, {} confirmed",
                t.paint(Role::Info, &fmt_count(n.fits)),
                t.paint(Role::Info, &fmt_count(n.fit_confirmed))
            ));
        }
        if n.kin > 0 {
            parts.push(format!("{} kin", t.paint(Role::Info, &fmt_count(n.kin))));
        }
        parts.push(t.paint(Role::Dupe, &fmt_duration(elapsed)));
        parts.join("   ")
    }

    pub fn frames_summary(&self, v: &VideoStats, elapsed: Duration) -> String {
        let t = &self.theme;
        if v.hashed + v.unreadable < 2 {
            return format!(
                "{} {}   {}",
                t.paint(Role::Info, &fmt_count(v.hashed)),
                plural(v.hashed, "video", "videos"),
                t.paint(Role::Dupe, "nothing to compare")
            );
        }
        let mut parts = vec![
            format!(
                "{} {} hashed",
                t.paint(Role::Info, &fmt_count(v.hashed)),
                plural(v.hashed, "video", "videos")
            ),
            t.paint(
                Role::Dupe,
                &format!("{} frames at 15 fps", fmt_count(v.frames)),
            ),
        ];
        if v.unreadable > 0 {
            parts.push(t.paint(
                Role::Dupe,
                &format!("{} unreadable", fmt_count(v.unreadable)),
            ));
        }
        parts.push(t.paint(Role::Dupe, &fmt_duration(elapsed)));
        parts.join("   ")
    }

    pub fn video_match_summary(&self, v: &VideoStats, elapsed: Duration) -> String {
        let t = &self.theme;
        format!(
            "{} similar pairs   {} videos grouped   {}",
            t.paint(Role::Info, &fmt_count(v.pairs)),
            t.paint(Role::Info, &fmt_count(v.grouped)),
            t.paint(Role::Dupe, &fmt_duration(elapsed))
        )
    }

    pub fn finish_stage(&self, stage: Stage, text: String) {
        self.finish(stage, text);
    }

    pub fn stage_elapsed(&self, stage: Stage) -> Duration {
        let st = self.state.lock().unwrap();
        match &st.current {
            Some(c) if c.stage == stage => c.started.elapsed(),
            _ => Duration::ZERO,
        }
    }
}

impl Progress for LiveProgress {
    fn done(&self, stage: Stage, stats: &PipelineStats) {
        let elapsed = {
            let st = self.state.lock().unwrap();
            match &st.current {
                Some(c) if c.stage == stage => c.started.elapsed(),
                _ => Duration::ZERO,
            }
        };
        let text = self.stage_summary(stage, stats, elapsed);
        self.finish(stage, text);
    }

    fn tick(&self, stage: Stage, done: u64, total: u64) {
        let mut st = self.state.lock().unwrap();
        let fresh = !matches!(&st.current, Some(c) if c.stage == stage);
        if fresh {
            st.current = Some(StageLine {
                stage,
                done,
                total: Some(total),
                started: Instant::now(),
                text: String::new(),
                finished: false,
            });
            st.ticks = 0;
        }
        if let Some(c) = &mut st.current {
            c.done = done;
            c.total = Some(total);
            c.text = format!(
                "{} / {}",
                self.theme.paint(Role::Info, &fmt_count(done)),
                self.theme.paint(Role::Dupe, &fmt_count(total))
            );
        }
        st.ticks += 1;
        self.draw(&mut st, false);
    }
}

// ---------------------------------------------------------------------------
// renderer (main thread)
// ---------------------------------------------------------------------------

pub struct Renderer {
    pub theme: Theme,
}

impl Renderer {
    pub fn new(theme: Theme) -> Renderer {
        Renderer { theme }
    }

    fn line(&self, s: &str) {
        let mut out = io::stdout().lock();
        let _ = writeln!(out, "{s}");
    }

    pub fn blank(&self) {
        self.line("");
    }

    pub fn rule(&self) {
        self.line(&self.theme.rule());
    }

    pub fn header(&self, inv: &Invocation) {
        let t = &self.theme;
        // everything but the root first, so the root gets what is left
        let lead = format!(
            "{} {}",
            t.paint_bold(Role::Plate, "dedupe"),
            t.paint_bold(Role::Keep, inv.command.word())
        );
        let mut tail: Vec<String> = Vec::new();
        if matches!(inv.command, Command::Find | Command::Milk | Command::Purge) {
            tail.push(t.paint(
                Role::Dupe,
                if inv.recurse {
                    "recursive"
                } else {
                    "this folder"
                },
            ));
            tail.push(t.paint(Role::Dupe, inv.kind.label()));
            if inv.quick {
                tail.push(t.paint(Role::Warn, "quick: size + sample only"));
            }
        }
        if inv.dry_run && matches!(inv.command, Command::Milk | Command::Purge | Command::Undo) {
            tail.push(t.paint_bold(Role::Warn, "dry run"));
        }
        let used =
            1 + visible_width(&lead) + 2 + tail.iter().map(|p| visible_width(p) + 2).sum::<usize>();
        let root = inv.root.to_string_lossy();
        let root = shorten_middle(
            &root,
            t.width.saturating_sub(used + 1).max(24),
            t.glyphs.ellipsis,
        );
        let mut parts = vec![lead, t.paint(Role::Info, &root)];
        parts.extend(tail);
        self.line(&format!(" {}", parts.join("  ")));
        self.rule();
    }

    pub fn scan_text(&self, s: &ScanStats, elapsed: Option<Duration>) -> String {
        let t = &self.theme;
        let mut v = vec![
            format!(
                "{} {}",
                t.paint(Role::Info, &fmt_count(s.files)),
                plural(s.files, "file", "files")
            ),
            t.paint(Role::Info, &fmt_bytes(s.bytes)),
        ];
        if let Some(e) = elapsed {
            v.push(t.paint(Role::Dupe, &fmt_duration(e)));
        }
        v.join("   ")
    }

    /// The group list. `root` makes paths relative.
    pub fn groups(&self, groups: &[Group], root: &Path) {
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
            self.line(&format!(" {}", parts.join("   ")));

            // member lines:  "   keep  <path>  <date>"
            let date_cells = 10;
            let path_cells = t.width.saturating_sub(3 + 6 + 2 + date_cells + 1).max(12);
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
    /// exact groups so a line in the output has one number.
    pub fn near_groups(&self, groups: &[NearGroup], root: &Path, offset: usize) {
        let t = &self.theme;
        if groups.is_empty() {
            return;
        }
        self.blank();
        let digits = (offset + groups.len()).to_string().len().max(3);

        // Every member's tail (everything after the path) is built first,
        // plain for measuring and painted for printing, so the path column
        // is as wide as this list allows and no line exceeds the width.
        struct Tail {
            plain: String,
            painted: String,
        }
        // `compact` drops the dimensions column when the width is short, so
        // the path keeps at least 14 cells beside a long evidence tail
        let compact = std::cell::Cell::new(false);
        let lead_cols = |w: u32, h: u32, size: u64| -> (String, String) {
            let mut cols: Vec<String> = Vec::new();
            if !compact.get() {
                cols.push(fit(&format!("{w}{}{h}", t.glyphs.times), 11));
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
        let keeper_tail = |c: &crate::walk::Candidate, w: u32, h: u32| -> Tail {
            let (lp, lq) = lead_cols(w, h, c.size);
            let date = fmt_date(c.mtime);
            Tail {
                plain: format!("{lp}  {date}"),
                painted: format!("{lq}  {}", t.paint(Role::Dupe, &date)),
            }
        };
        let member_tail = |m: &crate::near::NearMember| -> Tail {
            let c = &m.hashed.rep.cand;
            let r = m.relation.unwrap();
            let (lp, lq) = lead_cols(m.hashed.sig.width, m.hashed.sig.height, c.size);
            // d, p: 64-bit distances to the keeper; q: PDQ (256-bit). A
            // distance over its threshold is lit: d/p when the member is in
            // through the deep tier alone, q when purge would not bin on it.
            let dp_txt = format!("d {:>2}  p {:>2}", r.d, r.p);
            let q_txt = format!("q {:>3}", r.q);
            let dp_over = r.d > DHASH_MAX || r.p > PHASH_MAX;
            let q_over = r.q > PDQ_MAX;
            let mut plain = format!("{lp}  {dp_txt}  {q_txt}");
            let mut painted = format!(
                "{lq}  {}  {}",
                t.paint(if dp_over { Role::Warn } else { Role::Dupe }, &dp_txt),
                t.paint(if q_over { Role::Warn } else { Role::Dupe }, &q_txt)
            );
            if let Some(cos) = r.deep {
                let txt = format!("deep {cos:.2}");
                let role = if r.deep_only() {
                    Role::Info
                } else {
                    Role::Dupe
                };
                plain.push_str(&format!("  {txt}"));
                painted.push_str(&format!("  {}", t.paint(role, &txt)));
            }
            // the geometric fit, where it was asked for: lit when it is the
            // evidence that confirmed the member, warning when it fell short
            if let Some(f) = r.fit {
                let txt = f.label();
                let role = if r.fit_ok() && !r.pdq_ok() {
                    Role::Info
                } else if !r.fit_ok() {
                    Role::Warn
                } else {
                    Role::Dupe
                };
                plain.push_str(&format!("  {txt}"));
                painted.push_str(&format!("  {}", t.paint(role, &txt)));
            }
            // a different capture moment: the member's own time
            if r.kin {
                if let Some(c) = &m.hashed.capture {
                    let txt = format!("taken {}", c.display());
                    plain.push_str(&format!("  {txt}"));
                    painted.push_str(&format!("  {}", t.paint(Role::Info, &txt)));
                }
            }
            // the orientation is a 64-bit finding; meaningless on a deep-only line
            let label = r.variant.label();
            if !label.is_empty() && !r.deep_only() {
                plain.push_str(&format!("  {label}"));
                painted.push_str(&format!("  {}", t.paint(Role::Dupe, label)));
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
                .flat_map(|g| g.members.iter().enumerate())
                .map(|(j, m)| {
                    if j == 0 {
                        width(
                            &keeper_tail(
                                &m.hashed.rep.cand,
                                m.hashed.sig.width,
                                m.hashed.sig.height,
                            )
                            .plain,
                        )
                    } else {
                        width(&member_tail(m).plain)
                    }
                })
                .max()
                .unwrap_or(0)
        };
        // margin 1 + indent 2 + verb 4 + 2 + path + 2 + tail, inside the width
        let lead = 1 + 2 + 4 + 2 + 2;
        let mut widest_tail = widest();
        if t.width < lead + widest_tail + 14 {
            compact.set(true);
            widest_tail = widest();
        }
        let path_cells = t.width.saturating_sub(lead + widest_tail).max(12);

        for (i, g) in groups.iter().enumerate() {
            let n = format!("{:0width$}", offset + i + 1, width = digits);
            let head = t.paint_bold(Role::Info, &format!("NEAR {n}"));
            let deep_n = g.deep_only_count();
            let kin_n = g.kin_count();
            let similar = g.members.len() - 1 - deep_n - kin_n;
            let mut count = format!("{} similar", t.paint(Role::Info, &similar.to_string()));
            if deep_n > 0 {
                count.push_str(&format!(
                    "  {}",
                    t.paint(Role::Info, &format!("+{deep_n} deep"))
                ));
            }
            if kin_n > 0 {
                count.push_str(&format!(
                    "  {}",
                    t.paint(Role::Info, &format!("+{kin_n} kin"))
                ));
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
            self.line(&format!(" {}", parts.join("   ")));

            for (j, m) in g.members.iter().enumerate() {
                let c = &m.hashed.rep.cand;
                let rel = rel_display(&c.path, root);
                let shown = fit(
                    &shorten_middle(&rel, path_cells, t.glyphs.ellipsis),
                    path_cells,
                );
                if j == 0 {
                    let tail = keeper_tail(c, m.hashed.sig.width, m.hashed.sig.height);
                    self.line(&format!(
                        "   {}  {}  {}",
                        t.paint_bold(Role::Keep, "keep"),
                        t.paint(Role::Plate, &shown),
                        tail.painted
                    ));
                } else {
                    let r = m.relation.unwrap();
                    let verb = match r.verb() {
                        "near" => t.paint(Role::Dupe, "near"),
                        other => t.paint(Role::Info, &fit(other, 4)),
                    };
                    let tail = member_tail(m);
                    self.line(&format!(
                        "   {}  {}  {}",
                        verb,
                        t.paint(Role::Dupe, &shown),
                        tail.painted
                    ));
                }
            }
        }
    }

    /// Video groups; numbering continues after exact and near groups.
    pub fn video_groups(&self, groups: &[VideoGroup], root: &Path, offset: usize) {
        let t = &self.theme;
        if groups.is_empty() {
            return;
        }
        self.blank();
        let digits = (offset + groups.len()).to_string().len().max(3);

        struct Tail {
            plain: String,
            painted: String,
        }
        // "1920×1080  04:12  h264   1.1 GB" is common to both line kinds.
        // `compact` drops columns when the width is short: 1 drops the
        // codec, 2 also drops the dimensions (duration and size stay).
        let compact = std::cell::Cell::new(0u8);
        let meta_cols = |m: &crate::video::VideoMember| -> (String, String) {
            let c = &m.hashed.rep.cand;
            let meta = &m.hashed.sig.meta;
            let mut cols: Vec<String> = Vec::new();
            if compact.get() < 2 {
                cols.push(fit(
                    &format!("{}{}{}", meta.width, t.glyphs.times, meta.height),
                    11,
                ));
            }
            cols.push(fit(&fmt_clock(meta.duration), 6));
            if compact.get() < 1 {
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
        let member_tail =
            |m: &crate::video::VideoMember, keeper: &crate::video::VideoMember| -> Tail {
                let keeper_dur = keeper.hashed.sig.meta.duration;
                let keeper_bars = keeper.hashed.sig.bars.is_some();
                let (mut plain, mut painted) = meta_cols(m);
                let r = m.relation.unwrap();
                match r.pos {
                    Some((d, p, q)) => {
                        let over = d > DHASH_MAX || p > PHASH_MAX;
                        let dp = format!("d {d:>2} p {p:>2}");
                        let qq = format!("q {q:>3}");
                        plain.push_str(&format!("  {dp}  {qq}"));
                        painted.push_str(&format!(
                            "  {}  {}",
                            t.paint(if over { Role::Warn } else { Role::Dupe }, &dp),
                            t.paint(if q > PDQ_MAX { Role::Warn } else { Role::Dupe }, &qq)
                        ));
                    }
                    None => {
                        let txt = "frames     -";
                        plain.push_str(&format!("  {txt}"));
                        painted.push_str(&format!("  {}", t.paint(Role::Dupe, txt)));
                    }
                }
                // level 2 is only computed when the mean features agree (level 1);
                // a mirror or different content shows "-" and the frames carry it
                let tmk = if r.level1 >= crate::video::LEVEL1_MIN {
                    format!("tmk {:.2}", r.level2)
                } else {
                    "tmk    -".to_string()
                };
                let tmk_role = if r.level2 >= LEVEL2_MIN && r.level2 < LEVEL2_CONFIRM {
                    Role::Warn
                } else {
                    Role::Dupe
                };
                plain.push_str(&format!("  {tmk}"));
                painted.push_str(&format!("  {}", t.paint(tmk_role, &tmk)));
                let label = r.variant.label();
                if !label.is_empty() && r.positional_ok() {
                    plain.push_str(&format!("  {label}"));
                    painted.push_str(&format!("  {}", t.paint(Role::Dupe, label)));
                }
                let dur = m.hashed.sig.meta.duration;
                if keeper_dur > 0.0 && (dur - keeper_dur).abs() / keeper_dur > 0.02 {
                    plain.push_str("  trim");
                    painted.push_str(&format!("  {}", t.paint(Role::Dupe, "trim")));
                }
                // one side carries black bars the other does not: the frames
                // were compared inside them, which is why the sizes differ
                if m.hashed.sig.bars.is_some() != keeper_bars {
                    plain.push_str("  bars");
                    painted.push_str(&format!("  {}", t.paint(Role::Dupe, "bars")));
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
        // give the path at least 18 cells before dropping columns
        let lead = 1 + 2 + 4 + 2 + 2;
        let mut widest_tail = widest();
        while compact.get() < 2 && t.width < lead + widest_tail + 18 {
            compact.set(compact.get() + 1);
            widest_tail = widest();
        }
        let path_cells = t.width.saturating_sub(lead + widest_tail).max(12);

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
            self.line(&format!(" {}", parts.join("   ")));

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
    }

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
        self.line(&format!(" {}", counts.join("   ")));
        if !policy.is_empty() {
            self.line(&format!("   {}", join_fit(&policy, max.saturating_sub(3))));
        }
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

    /// A `verb   command   note` line: the verb in a fixed column, the
    /// note shortened before the command ever is.
    pub fn hint(&self, verb: &str, command: &str, note: &str) {
        let t = &self.theme;
        let lead = 1 + HINT_VERB + 2;
        let room = t.width.saturating_sub(lead + width(command) + 3 + 1);
        let note = shorten_middle(note, room.max(8), t.glyphs.ellipsis);
        self.line(&format!(
            " {}  {}   {}",
            t.paint_bold(Role::Keep, &fit(verb, HINT_VERB)),
            t.paint(Role::Plate, command),
            t.paint(Role::Dupe, &note)
        ));
    }

    /// After a run that saw videos without the video pass. The suggested
    /// command is the full form: no `--quick-search`, which has no near pass.
    pub fn video_hint(&self, inv: &Invocation, videos: u64) {
        let mut i = inv.clone();
        i.kind = crate::kind::Kind::Video;
        i.quick = false;
        self.hint(
            "video",
            &i.command_line_as(Command::Find),
            &format!(
                "{} seen; adds near-duplicate matching",
                counted(videos, "video", "videos")
            ),
        );
    }

    pub fn near_totals(&self, groups: &[NearGroup]) {
        let t = &self.theme;
        if groups.is_empty() {
            return;
        }
        let deep_only: u64 = groups.iter().map(|g| g.deep_only_count() as u64).sum();
        let kin: u64 = groups.iter().map(|g| g.kin_count() as u64).sum();
        let similar: u64 = groups
            .iter()
            .map(|g| g.members.len() as u64 - 1)
            .sum::<u64>()
            - deep_only
            - kin;
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
                    "taken at a different moment: another photograph, left alone",
                )],
            );
        }
        let deep_on = groups
            .iter()
            .flat_map(|g| g.members.iter())
            .any(|m| m.relation.is_some_and(|r| r.deep.is_some()));
        let deep_txt = if deep_on {
            format!("  deep {}{:.2} lists", t.glyphs.ge, crate::deep::DEEP_MIN)
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

    /// After an exact-only run that saw images: how to add the near pass.
    pub fn near_hint(&self, inv: &Invocation, images: u64) {
        let mut i = inv.clone();
        i.kind = crate::kind::Kind::Image;
        i.quick = false;
        self.hint(
            "near",
            &i.command_line_as(Command::Find),
            &format!(
                "{} seen; adds near-duplicate matching",
                counted(images, "image", "images")
            ),
        );
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

    /// The follow-up commands after `find`, in their full form (a quick
    /// search is for looking; moving and binning run on full evidence).
    pub fn next_after_find(&self, inv: &Invocation) {
        let t = &self.theme;
        let mut full = inv.clone();
        full.quick = false;
        let milk = full.command_line_as(Command::Milk);
        let purge = full.command_line_as(Command::Purge);
        let w = width(&milk).max(width(&purge)) + 2;
        let lead = 1 + HINT_VERB + 2 + w + 1;
        let dest = under_root(inv.root_display.as_deref(), MILKED_DIR);
        let note = shorten_middle(
            &format!("move duplicates into {dest}"),
            t.width.saturating_sub(lead).max(8),
            t.glyphs.ellipsis,
        );
        self.line(&format!(
            " {}  {}{}",
            t.paint_bold(Role::Keep, &fit("next", HINT_VERB)),
            t.paint(Role::Plate, &fit(&milk, w)),
            t.paint(Role::Dupe, &note)
        ));
        self.line(&format!(
            " {}  {}{}",
            " ".repeat(HINT_VERB),
            t.paint(Role::Plate, &fit(&purge, w)),
            t.paint(Role::Dupe, "send duplicates to the Recycle Bin")
        ));
    }

    /// One condensed line per action: verb, path, and a right-aligned
    /// detail. The path is shortened first; the detail only when the path
    /// is already at its minimum.
    pub fn action(&self, verb: &str, role: Role, path: &str, detail: &str) {
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

    pub fn action_totals(
        &self,
        verb_past: &str,
        n: u64,
        bytes: u64,
        destination: &str,
        journal_id: Option<&str>,
    ) {
        let t = &self.theme;
        self.rule();
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

    pub fn undo_hint(&self, inv: &Invocation) {
        let mut cmd = String::from("dedupe --undo");
        if let Some(r) = &inv.root_display {
            cmd.push(' ');
            cmd.push_str(&if r.contains(' ') {
                format!("\"{r}\"")
            } else {
                r.clone()
            });
        }
        self.hint("undo", &cmd, "reverses this change");
    }

    pub fn note(&self, role: Role, s: &str) {
        self.line(&format!(" {}", self.theme.paint(role, s)));
    }

    /// A note whose second part may drop to an indented line when the width
    /// is short; `detail` is shortened in the middle if even that fails.
    pub fn note_fit(&self, role: Role, lead: &str, detail: &str) {
        let t = &self.theme;
        let max = t.width.saturating_sub(1);
        let one = format!("{lead}   {detail}");
        if width(&one) < max {
            self.line(&format!(" {}", t.paint(role, &one)));
        } else {
            self.line(&format!(" {}", t.paint(role, lead)));
            self.line(&format!(
                "   {}",
                t.paint(
                    Role::Dupe,
                    &shorten_middle(detail, max.saturating_sub(3), t.glyphs.ellipsis)
                )
            ));
        }
    }

    pub fn error(&self, msg: &str) {
        let t = &self.theme;
        let mut out = io::stderr().lock();
        let _ = writeln!(
            out,
            " {}  {}",
            t.paint_bold(Role::Danger, "dedupe"),
            t.paint(Role::Plate, msg)
        );
        let _ = writeln!(out, "         {}", t.paint(Role::Dupe, "dedupe --help"));
    }
}
