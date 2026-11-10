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
/// A group row before its tail: margin, indent, verb, gap; then the gap
/// after the path. The tail starts `ROW_LEAD + path` cells in.
const ROW_LEAD: usize = 1 + 2 + 4 + 2 + 2;
/// An exact keeper row after its path: the date and one cell of margin.
const EXACT_POST: usize = 10 + 1;

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
        // the routes other than the image crate, by name (Phase 10)
        for (k, name) in [
            (n.heif, "HEIC"),
            (n.avif, "AVIF"),
            (n.raw, "RAW"),
            (n.ffmpeg, "via ffmpeg"),
        ] {
            if k > 0 {
                v.push(format!("{} {name}", t.paint(Role::Info, &fmt_count(k))));
            }
        }
        if n.deep {
            v.push(t.paint(Role::Dupe, "+deep"));
        }
        if n.deep_failed > 0 {
            v.push(t.paint(
                Role::Dupe,
                &format!("{} embeddings failed", fmt_count(n.deep_failed)),
            ));
        }
        // what was not decoded, by the reason a person can act on
        let unreadable = n
            .undecodable
            .saturating_sub(n.need_ffmpeg + n.ffmpeg_failed + n.no_preview);
        for (k, what, role) in [
            (n.need_ffmpeg, "need ffmpeg", Role::Warn),
            (n.ffmpeg_failed, "ffmpeg could not decode", Role::Warn),
            (n.no_preview, "RAW without a preview", Role::Dupe),
            (unreadable, "undecodable", Role::Dupe),
        ] {
            if k > 0 {
                v.push(t.paint(role, &format!("{} {what}", fmt_count(k))));
            }
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
    /// When set, lines go here instead of to stdout (the workspace
    /// captures the listing to show it in its own screen).
    sink: Option<std::cell::RefCell<Vec<String>>>,
}

impl Renderer {
    pub fn new(theme: Theme) -> Renderer {
        Renderer { theme, sink: None }
    }

    /// Run `f` against a renderer `width` cells wide whose lines are
    /// collected instead of printed.
    pub fn capture(&self, width: usize, f: impl FnOnce(&Renderer)) -> Vec<String> {
        let mut theme = self.theme;
        theme.width = width;
        let r = Renderer {
            theme,
            sink: Some(std::cell::RefCell::new(Vec::new())),
        };
        f(&r);
        r.sink.map(|s| s.into_inner()).unwrap_or_default()
    }

    pub fn line(&self, s: &str) {
        if let Some(sink) = &self.sink {
            sink.borrow_mut().push(s.to_string());
            return;
        }
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
            if inv.min > 0 {
                tail.push(t.paint(Role::Dupe, &format!("min {}", fmt_bytes(inv.min))));
            }
            if !inv.skip.is_empty() {
                tail.push(t.paint(Role::Dupe, &format!("skip {}", inv.skip.join(", "))));
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
        // what the filters left out, so a short count is never a mystery
        if s.skipped > 0 {
            v.push(t.paint(Role::Dupe, &format!("{} skipped", fmt_count(s.skipped))));
        }
        if s.small > 0 {
            v.push(t.paint(
                Role::Dupe,
                &format!("{} under {}", fmt_count(s.small), fmt_bytes(s.min)),
            ));
        }
        if let Some(e) = elapsed {
            v.push(t.paint(Role::Dupe, &fmt_duration(e)));
        }
        v.join("   ")
    }

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
                g.members[1..]
                    .iter()
                    .map(move |m| width(&m.relation.unwrap().words(k, &m.hashed)))
            })
            .max()
            .unwrap_or(0);
        let member_tail = |m: &crate::near::NearMember, k: &crate::near::Hashed| -> Tail {
            let c = &m.hashed.rep.cand;
            let r = m.relation.unwrap();
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
                    let r = m.relation.unwrap();
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
                g.members[1..]
                    .iter()
                    .map(move |m| width(&m.relation.unwrap().words(k, &m.hashed.sig)))
            })
            .max()
            .unwrap_or(0);
        let member_tail =
            |m: &crate::video::VideoMember, keeper: &crate::video::VideoMember| -> Tail {
                let (mut plain, mut painted) = meta_cols(m);
                let r = m.relation.unwrap();
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
    /// command always whole (it is there to be copied), the note whole or
    /// not at all — a note cut to its first and last letters says nothing.
    pub fn hint(&self, verb: &str, command: &str, note: &str) {
        let t = &self.theme;
        let lead = 1 + HINT_VERB + 2;
        let room = t.width.saturating_sub(lead + width(command) + 3 + 1);
        let note = if width(note) <= room { note } else { "" };
        self.line(&format!(
            " {}  {}   {}",
            t.paint_bold(Role::Keep, &fit(verb, HINT_VERB)),
            t.paint(Role::Plate, command),
            t.paint(Role::Dupe, note)
        ));
    }

    /// After a quick search that saw images or videos: the full form of
    /// the same command, which matches near duplicates too.
    pub fn full_hint(&self, inv: &Invocation, images: u64, videos: u64) {
        let mut i = inv.clone();
        i.quick = false;
        let mut seen: Vec<String> = Vec::new();
        if images >= 2 {
            seen.push(counted(images, "image", "images"));
        }
        if videos >= 2 {
            seen.push(counted(videos, "video", "videos"));
        }
        self.hint(
            "full",
            &i.command_line_as(Command::Find),
            &format!(
                "{} seen; the full run matches near duplicates",
                seen.join(", ")
            ),
        );
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
                    "taken at a different moment: another photograph, left alone",
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
        let room = t.width.saturating_sub(lead);
        // notes whole or not at all: the commands are the payload and a
        // note cut to its ends says nothing. The milk note names the full
        // destination when there is room, the bare folder name otherwise.
        let dest = under_root(inv.root_display.as_deref(), MILKED_DIR);
        let long_note = format!("move duplicates into {dest}");
        let short_note = format!("move duplicates into {MILKED_DIR}");
        let milk_note = if width(&long_note) <= room {
            long_note.as_str()
        } else if width(&short_note) <= room {
            short_note.as_str()
        } else {
            ""
        };
        let purge_note = "send duplicates to the Recycle Bin";
        let purge_note = if width(purge_note) <= room {
            purge_note
        } else {
            ""
        };
        self.line(&format!(
            " {}  {}{}",
            t.paint_bold(Role::Keep, &fit("next", HINT_VERB)),
            t.paint(Role::Plate, &fit(&milk, w)),
            t.paint(Role::Dupe, milk_note)
        ));
        self.line(&format!(
            " {}  {}{}",
            " ".repeat(HINT_VERB),
            t.paint(Role::Plate, &fit(&purge, w)),
            t.paint(Role::Dupe, purge_note)
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
