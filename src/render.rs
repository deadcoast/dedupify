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
    fit, fmt_bytes, fmt_clock, fmt_count, fmt_date, fmt_duration, rel_display, shorten_middle,
    width,
};
use crate::video::{VideoGroup, VideoStats, LEVEL2_CONFIRM, LEVEL2_MIN, VIDEO_POLICY_LABEL};
use crate::walk::ScanStats;
use crate::MILKED_DIR;

pub const BAR_CELLS: usize = 30;
const REDRAW_EVERY: Duration = Duration::from_millis(40);

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
        let line = format!(" {label} {bar}  {}", c.text);
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

pub fn plural<'a>(n: u64, one: &'a str, many: &'a str) -> &'a str {
    if n == 1 {
        one
    } else {
        many
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
        let mut v = vec![format!(
            "{} images hashed",
            t.paint(Role::Info, &fmt_count(n.decoded))
        )];
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
        format!(
            "{} similar pairs   {} images grouped   {}",
            t.paint(Role::Info, &fmt_count(n.pairs)),
            t.paint(Role::Info, &fmt_count(n.grouped)),
            t.paint(Role::Dupe, &fmt_duration(elapsed))
        )
    }

    pub fn frames_summary(&self, v: &VideoStats, elapsed: Duration) -> String {
        let t = &self.theme;
        let mut parts = vec![
            format!(
                "{} videos hashed",
                t.paint(Role::Info, &fmt_count(v.hashed))
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
        let mut parts: Vec<String> = Vec::new();
        parts.push(format!(
            "{} {}",
            t.paint_bold(Role::Plate, "dedupe"),
            t.paint_bold(Role::Keep, inv.command.word())
        ));
        let root = inv.root.to_string_lossy();
        let root = shorten_middle(&root, t.width.saturating_sub(40).max(24), t.glyphs.ellipsis);
        parts.push(t.paint(Role::Info, &root));
        if matches!(inv.command, Command::Find | Command::Milk | Command::Purge) {
            parts.push(t.paint(
                Role::Dupe,
                if inv.recurse {
                    "recursive"
                } else {
                    "this folder"
                },
            ));
            parts.push(t.paint(Role::Dupe, inv.kind.label()));
            if inv.quick {
                parts.push(t.paint(Role::Warn, "quick: size + sample only"));
            }
        }
        if inv.dry_run && matches!(inv.command, Command::Milk | Command::Purge | Command::Undo) {
            parts.push(t.paint_bold(Role::Warn, "dry run"));
        }
        self.line(&format!(" {}", parts.join("  ")));
        self.rule();
    }

    pub fn scan_text(&self, s: &ScanStats, elapsed: Option<Duration>) -> String {
        let t = &self.theme;
        let mut v = vec![
            format!("{} files", t.paint(Role::Info, &fmt_count(s.files))),
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
        for (i, g) in groups.iter().enumerate() {
            let n = format!("{:0width$}", offset + i + 1, width = digits);
            let head = t.paint_bold(Role::Info, &format!("NEAR {n}"));
            let count = format!(
                "{} similar",
                t.paint(Role::Info, &g.members.len().to_string())
            );
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

            // member lines: "   keep  <path>  4000×3000  2.1 MB  2019-06-12"
            //               "   near  <path>  1600×1200  380 KB  d 2  p 5  q 12  mirror"
            let tail_cells = 11 + 2 + 7 + 2 + 21;
            let path_cells = t.width.saturating_sub(3 + 6 + 2 + tail_cells + 1).max(12);
            for (j, m) in g.members.iter().enumerate() {
                let c = &m.hashed.rep.cand;
                let rel = rel_display(&c.path, root);
                let shown = fit(
                    &shorten_middle(&rel, path_cells, t.glyphs.ellipsis),
                    path_cells,
                );
                let dims = format!(
                    "{}{}{}",
                    m.hashed.sig.width, t.glyphs.times, m.hashed.sig.height
                );
                let dims = fit(&dims, 11);
                let size = fit(&fmt_bytes(c.size), 7);
                if j == 0 {
                    self.line(&format!(
                        "   {}  {}  {}  {}  {}",
                        t.paint_bold(Role::Keep, "keep"),
                        t.paint(Role::Plate, &shown),
                        t.paint(Role::Dupe, &dims),
                        t.paint(Role::Dupe, &size),
                        t.paint(Role::Dupe, &fmt_date(c.mtime))
                    ));
                } else {
                    let r = m.relation.unwrap();
                    // d, p: 64-bit distances to the keeper; q: PDQ (256-bit).
                    // q is lit in the warning colour when purge would not
                    // bin on it — the member stays a milk candidate.
                    let dp_txt = format!("d {:>2}  p {:>2}", r.d, r.p);
                    let q_txt = format!("q {:>3}", r.q);
                    let dp_over = r.d > DHASH_MAX || r.p > PHASH_MAX;
                    let q_over = r.q > PDQ_MAX;
                    let mut rel_txt = format!(
                        "{}  {}",
                        t.paint(if dp_over { Role::Warn } else { Role::Dupe }, &dp_txt),
                        t.paint(if q_over { Role::Warn } else { Role::Dupe }, &q_txt)
                    );
                    let label = r.variant.label();
                    if !label.is_empty() {
                        rel_txt.push_str("  ");
                        rel_txt.push_str(&t.paint(Role::Dupe, label));
                    }
                    let mut extra = String::new();
                    if let Some(eg) = m.hashed.rep.exact_group {
                        extra = t.paint(
                            Role::Dupe,
                            &format!("  +GROUP {:0width$}", eg + 1, width = digits),
                        );
                    }
                    self.line(&format!(
                        "   {}  {}  {}  {}  {}{}",
                        t.paint(Role::Dupe, "near"),
                        t.paint(Role::Dupe, &shown),
                        t.paint(Role::Dupe, &dims),
                        t.paint(Role::Dupe, &size),
                        rel_txt,
                        extra
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
        for (i, g) in groups.iter().enumerate() {
            let n = format!("{:0width$}", offset + i + 1, width = digits);
            let head = t.paint_bold(Role::Info, &format!("VIDEO {n}"));
            let count = format!(
                "{} similar",
                t.paint(Role::Info, &g.members.len().to_string())
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

            // "   keep  <path>  1920×1080  04:12  h264  1.1 GB   2021-06-01"
            // "   near  <path>  1280×720   04:12  hevc  410 MB   d 1 p 2 q 6   tmk 0.98"
            let tail_cells = 11 + 2 + 6 + 2 + 5 + 2 + 7 + 2 + 30;
            let path_cells = t.width.saturating_sub(3 + 6 + 2 + tail_cells).max(12);
            for (j, m) in g.members.iter().enumerate() {
                let c = &m.hashed.rep.cand;
                let meta = &m.hashed.sig.meta;
                let rel = rel_display(&c.path, root);
                let shown = fit(
                    &shorten_middle(&rel, path_cells, t.glyphs.ellipsis),
                    path_cells,
                );
                let dims = fit(
                    &format!("{}{}{}", meta.width, t.glyphs.times, meta.height),
                    11,
                );
                let dur = fit(&fmt_clock(meta.duration), 6);
                let codec = fit(&meta.codec, 5);
                let size = fit(&fmt_bytes(c.size), 7);
                if j == 0 {
                    self.line(&format!(
                        "   {}  {}  {}  {}  {}  {}  {}",
                        t.paint_bold(Role::Keep, "keep"),
                        t.paint(Role::Plate, &shown),
                        t.paint(Role::Dupe, &dims),
                        t.paint(Role::Dupe, &dur),
                        t.paint(Role::Dupe, &codec),
                        t.paint(Role::Dupe, &size),
                        t.paint(Role::Dupe, &fmt_date(c.mtime))
                    ));
                } else {
                    let r = m.relation.unwrap();
                    let mut rel_txt = String::new();
                    match r.pos {
                        Some((d, p, q)) => {
                            let over = d > crate::perceptual::DHASH_MAX
                                || p > crate::perceptual::PHASH_MAX;
                            rel_txt.push_str(&t.paint(
                                if over { Role::Warn } else { Role::Dupe },
                                &format!("d {d:>2} p {p:>2}"),
                            ));
                            rel_txt.push_str("  ");
                            rel_txt.push_str(&t.paint(
                                if q > crate::pdq::PDQ_MAX {
                                    Role::Warn
                                } else {
                                    Role::Dupe
                                },
                                &format!("q {q:>3}"),
                            ));
                        }
                        None => rel_txt.push_str(&t.paint(Role::Dupe, "frames  -  ")),
                    }
                    let tmk_role = if r.level2 >= LEVEL2_CONFIRM {
                        Role::Dupe
                    } else if r.level2 >= LEVEL2_MIN {
                        Role::Warn
                    } else {
                        Role::Dupe
                    };
                    rel_txt.push_str("  ");
                    if r.level1 >= crate::video::LEVEL1_MIN {
                        rel_txt.push_str(&t.paint(tmk_role, &format!("tmk {:.2}", r.level2)));
                    } else {
                        // level 2 is not computed when the mean features disagree
                        // (a mirror, or different content); the frames carry it
                        rel_txt.push_str(&t.paint(Role::Dupe, "tmk   -  "));
                    }
                    let label = r.variant.label();
                    if !label.is_empty() && r.positional_ok() {
                        rel_txt.push_str("  ");
                        rel_txt.push_str(&t.paint(Role::Dupe, label));
                    }
                    let keeper_dur = g.members[0].hashed.sig.meta.duration;
                    if keeper_dur > 0.0 && (meta.duration - keeper_dur).abs() / keeper_dur > 0.02 {
                        rel_txt.push_str("  ");
                        rel_txt.push_str(&t.paint(Role::Dupe, "trim"));
                    }
                    let mut extra = String::new();
                    if let Some(eg) = m.hashed.rep.exact_group {
                        extra = t.paint(
                            Role::Dupe,
                            &format!("  +GROUP {:0width$}", eg + 1, width = digits),
                        );
                    }
                    self.line(&format!(
                        "   {}  {}  {}  {}  {}  {}  {}{}",
                        t.paint(Role::Dupe, "near"),
                        t.paint(Role::Dupe, &shown),
                        t.paint(Role::Dupe, &dims),
                        t.paint(Role::Dupe, &dur),
                        t.paint(Role::Dupe, &codec),
                        t.paint(Role::Dupe, &size),
                        rel_txt,
                        extra
                    ));
                }
            }
        }
    }

    pub fn video_totals(&self, groups: &[VideoGroup]) {
        let t = &self.theme;
        if groups.is_empty() {
            return;
        }
        let similar: u64 = groups.iter().map(|g| g.members.len() as u64 - 1).sum();
        let recl: u64 = groups.iter().map(VideoGroup::reclaimable).sum();
        self.line(&format!(
            " {} video {}   {} similar   {} reclaimable   {}   {}",
            t.paint_bold(Role::Info, &fmt_count(groups.len() as u64)),
            plural(groups.len() as u64, "group", "groups"),
            t.paint_bold(Role::Info, &fmt_count(similar)),
            t.paint_bold(Role::Info, &fmt_bytes(recl)),
            t.paint(Role::Dupe, VIDEO_POLICY_LABEL),
            t.paint(
                Role::Dupe,
                &format!("tmk {ge}{LEVEL2_MIN:.1} groups, {ge}{LEVEL2_CONFIRM:.1} or frames q {le}{} to bin", crate::pdq::PDQ_MAX, ge = t.glyphs.ge, le = t.glyphs.le)
            )
        ));
    }

    /// After a run that saw videos without the video pass.
    pub fn video_hint(&self, inv: &Invocation, videos: u64) {
        let t = &self.theme;
        let mut i = inv.clone();
        i.kind = crate::kind::Kind::Video;
        self.line(&format!(
            " {}   {}   {}",
            t.paint_bold(Role::Keep, "video"),
            t.paint(Role::Plate, &i.command_line_as(Command::Find)),
            t.paint(
                Role::Dupe,
                &format!(
                    "{} videos seen; adds near-duplicate matching",
                    fmt_count(videos)
                )
            )
        ));
    }

    pub fn near_totals(&self, groups: &[NearGroup]) {
        let t = &self.theme;
        if groups.is_empty() {
            return;
        }
        let similar: u64 = groups.iter().map(|g| g.members.len() as u64 - 1).sum();
        let recl: u64 = groups.iter().map(NearGroup::reclaimable).sum();
        self.line(&format!(
            " {} near {}   {} similar   {} reclaimable   {}   {}",
            t.paint_bold(Role::Info, &fmt_count(groups.len() as u64)),
            plural(groups.len() as u64, "group", "groups"),
            t.paint_bold(Role::Info, &fmt_count(similar)),
            t.paint_bold(Role::Info, &fmt_bytes(recl)),
            t.paint(Role::Dupe, NEAR_POLICY_LABEL),
            t.paint(
                Role::Dupe,
                &format!(
                    "dHash {le}{DHASH_MAX}  pHash {le}{PHASH_MAX}  PDQ {le}{PDQ_MAX} to bin",
                    le = t.glyphs.le
                )
            )
        ));
    }

    /// After an exact-only run that saw images: how to add the near pass.
    pub fn near_hint(&self, inv: &Invocation, images: u64) {
        let t = &self.theme;
        let mut i = inv.clone();
        i.kind = crate::kind::Kind::Image;
        self.line(&format!(
            " {}   {}   {}",
            t.paint_bold(Role::Keep, "near"),
            t.paint(Role::Plate, &i.command_line_as(Command::Find)),
            t.paint(
                Role::Dupe,
                &format!(
                    "{} images seen; adds near-duplicate matching",
                    fmt_count(images)
                )
            )
        ));
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

    pub fn totals(&self, groups: &[Group], scan: &ScanStats) {
        let t = &self.theme;
        self.rule();
        if groups.is_empty() {
            self.line(&format!(
                " {}   {} files   {} checked",
                t.paint_bold(Role::Ok, "no duplicates"),
                t.paint(Role::Info, &fmt_count(scan.files)),
                t.paint(Role::Info, &fmt_bytes(scan.bytes))
            ));
            return;
        }
        let dupes: u64 = groups.iter().map(|g| g.members.len() as u64 - 1).sum();
        let recl: u64 = groups.iter().map(Group::reclaimable).sum();
        let sample_only = groups.iter().any(|g| g.tier == Tier::Sample);
        self.line(&format!(
            " {} {}   {} {}   {} reclaimable   {}",
            t.paint_bold(Role::Info, &fmt_count(groups.len() as u64)),
            plural(groups.len() as u64, "group", "groups"),
            t.paint_bold(Role::Info, &fmt_count(dupes)),
            plural(dupes, "duplicate", "duplicates"),
            t.paint_bold(Role::Info, &fmt_bytes(recl)),
            t.paint(Role::Dupe, POLICY_LABEL)
        ));
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

    pub fn next_after_find(&self, inv: &Invocation) {
        let t = &self.theme;
        let milk = inv.command_line_as(Command::Milk);
        let purge = inv.command_line_as(Command::Purge);
        let w = width(&milk).max(width(&purge)) + 2;
        let milked = inv.root.join(MILKED_DIR);
        self.line(&format!(
            " {}   {}{}",
            t.paint_bold(Role::Keep, "next"),
            t.paint(Role::Plate, &fit(&milk, w)),
            t.paint(
                Role::Dupe,
                &format!("move duplicates into {}", milked.to_string_lossy())
            )
        ));
        self.line(&format!(
            "        {}{}",
            t.paint(Role::Plate, &fit(&purge, w)),
            t.paint(Role::Dupe, "send duplicates to the Recycle Bin")
        ));
    }

    /// One condensed line per action.
    pub fn action(&self, verb: &str, role: Role, path: &str, detail: &str) {
        let t = &self.theme;
        let verb_cells = 8;
        let detail_cells = if detail.is_empty() {
            0
        } else {
            width(detail).min(t.width / 3)
        };
        let path_cells = t
            .width
            .saturating_sub(
                1 + verb_cells + 2 + detail_cells + if detail_cells > 0 { 2 } else { 0 },
            )
            .max(12);
        let shown = shorten_middle(path, path_cells, t.glyphs.ellipsis);
        if detail.is_empty() {
            self.line(&format!(
                " {}  {}",
                t.paint(role, &fit(verb, verb_cells)),
                t.paint(Role::Plate, &shown)
            ));
        } else {
            let det = shorten_middle(detail, detail_cells, t.glyphs.ellipsis);
            self.line(&format!(
                " {}  {}  {}",
                t.paint(role, &fit(verb, verb_cells)),
                t.paint(Role::Plate, &fit(&shown, path_cells)),
                t.paint(Role::Dupe, &det)
            ));
        }
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
        let mut parts = vec![
            format!("{} {verb_past}", t.paint_bold(Role::Info, &fmt_count(n))),
            format!(
                "{} {destination}",
                t.paint_bold(Role::Info, &fmt_bytes(bytes))
            ),
        ];
        if let Some(id) = journal_id {
            parts.push(format!(
                "{} {}",
                t.paint(Role::Dupe, "journal"),
                t.paint(Role::Dupe, id)
            ));
        }
        self.line(&format!(" {}", parts.join("   ")));
    }

    pub fn undo_hint(&self, inv: &Invocation) {
        let t = &self.theme;
        let mut cmd = String::from("dedupe --undo");
        if let Some(r) = &inv.root_display {
            cmd.push(' ');
            cmd.push_str(r);
        }
        self.line(&format!(
            " {}   {}",
            t.paint_bold(Role::Keep, "undo"),
            t.paint(Role::Plate, &cmd)
        ));
    }

    pub fn note(&self, role: Role, s: &str) {
        self.line(&format!(" {}", self.theme.paint(role, s)));
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
