//! The stage lines while a scan runs: one per stage, redrawn in place on
//! a terminal, its summary left behind; the time left on the long ones.

use std::io::{self, Write};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::exact::{PipelineStats, Progress, Stage};
use crate::near::NearStats;
use crate::theme::{Role, Theme};
use crate::util::{fit, fmt_bytes, fmt_count, fmt_duration, join_fit, plural, visible_width};
use crate::video::VideoStats;
use crate::walk::ScanStats;

use super::fold::{self, Hue, Mark};
use super::{Renderer, BAR_CELLS, REDRAW_EVERY};

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
    /// The stage lines' state. A thread that panicked while holding it
    /// leaves lines that may be half drawn, never wrong data: the lines
    /// go on being drawn rather than panicking a second time.
    fn lock(&self) -> std::sync::MutexGuard<'_, ProgressState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

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
        if self.theme.fold {
            fold::open(&self.theme, self.section_of(stage));
            return;
        }
        let mut st = self.lock();
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
        if self.theme.fold {
            fold::step(phase_of(stage), done, None, String::new());
            return;
        }
        let mut st = self.lock();
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

    /// Close the current stage with its final text; the line stays. The
    /// fold closes sections, not stages (`settle`).
    pub fn finish(&self, stage: Stage, text: String) {
        if self.theme.fold {
            return;
        }
        let mut st = self.lock();
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

        let cells = bar_cells(t.width);
        let filled = match (c.total, c.finished) {
            (_, true) => cells,
            (Some(total), _) if total > 0 => {
                ((c.done as f64 / total as f64) * cells as f64).round() as usize
            }
            _ => usize::MAX, // sweep
        };
        let line = compose(t, c.stage, c.finished, filled, st.ticks, &c.text);
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

impl LiveProgress {
    /// The walk so far: a stage line, or the fold's `files` line.
    pub fn scan(&self, s: &ScanStats) {
        if self.theme.fold {
            fold::step(
                "",
                s.files,
                None,
                format!(
                    "{} · {}",
                    plural(s.files, "file", "files"),
                    fmt_bytes(s.bytes)
                ),
            );
        } else {
            self.update_text(Stage::Scan, s.files, scan_text(&self.theme, s, None));
        }
    }

    /// Fold a section (nothing in the stage lines, which closed stage by
    /// stage).
    pub(crate) fn settle(&self, hue: Hue, mark: Mark, segments: &[String]) {
        if self.theme.fold {
            fold::settle(&self.theme, hue, mark, segments);
        }
    }

    /// The fold's section for a stage: MATCH belongs to whichever of the
    /// image and video passes is live.
    fn section_of(&self, stage: Stage) -> Hue {
        match stage {
            Stage::Scan => Hue::Files,
            Stage::Size | Stage::Sample | Stage::Hash => Hue::Copies,
            Stage::Verify => Hue::Verify,
            Stage::Decode => Hue::Similar,
            Stage::Frames => Hue::Video,
            Stage::Match => match fold::current() {
                Some(Hue::Video) => Hue::Video,
                _ => Hue::Similar,
            },
        }
    }

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
        if n.decoded + n.undecodable < 2 {
            // what did not decode is said even here: a single HEIC on a
            // machine without ffmpeg used to read "0 images" and no reason
            let mut v = vec![
                format!(
                    "{} {}",
                    t.paint(Role::Info, &fmt_count(n.decoded)),
                    plural(n.decoded, "image", "images")
                ),
                t.paint(Role::Dupe, "nothing to compare"),
            ];
            v.extend(self.decode_failures(n));
            return v.join("   ");
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
        v.extend(self.decode_failures(n));
        if n.low_quality > 0 {
            v.push(t.paint(
                Role::Dupe,
                &format!("{} flat, skipped", fmt_count(n.low_quality)),
            ));
        }
        v.push(t.paint(Role::Dupe, &fmt_duration(elapsed)));
        v.join("   ")
    }

    /// What was not decoded, by the reason a person can act on.
    fn decode_failures(&self, n: &NearStats) -> Vec<String> {
        let t = &self.theme;
        let stalled = n.stalled.len() as u64;
        let unreadable = n
            .undecodable
            .saturating_sub(n.need_ffmpeg + n.ffmpeg_failed + n.no_preview + stalled);
        [
            (stalled, "timed out", Role::Warn),
            (n.need_ffmpeg, "need ffmpeg", Role::Warn),
            (n.ffmpeg_failed, "ffmpeg could not decode", Role::Warn),
            (n.no_preview, "RAW without a preview", Role::Dupe),
            (unreadable, "undecodable", Role::Dupe),
        ]
        .into_iter()
        .filter(|(k, _, _)| *k > 0)
        .map(|(k, what, role)| t.paint(role, &format!("{} {what}", fmt_count(k))))
        .collect()
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
        let stalled = v.stalled.len() as u64;
        if stalled > 0 {
            parts.push(t.paint(Role::Warn, &format!("{} timed out", fmt_count(stalled))));
        }
        if v.unreadable > stalled {
            parts.push(t.paint(
                Role::Dupe,
                &format!("{} unreadable", fmt_count(v.unreadable - stalled)),
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
        let st = self.lock();
        match &st.current {
            Some(c) if c.stage == stage => c.started.elapsed(),
            _ => Duration::ZERO,
        }
    }
}

impl Progress for LiveProgress {
    fn done(&self, stage: Stage, stats: &PipelineStats) {
        if self.theme.fold {
            // VERIFY is a section of its own; the others fold with theirs
            if stage == Stage::Verify {
                let total = fold::total().unwrap_or(stats.confirmed);
                let t = &self.theme;
                let mark = if stats.confirmed == total {
                    Mark::Done
                } else {
                    Mark::Warn
                };
                let mut segs = vec![format!(
                    "{} {} {}",
                    fold::figure(t, &fmt_count(stats.confirmed)),
                    fold::dim(t, "of"),
                    fold::count(t, total, "copy", "copies")
                )];
                segs.push(fold::dim(t, "identical to their keepers, byte for byte"));
                if stats.confirmed < total {
                    segs.insert(
                        1,
                        fold::warn(
                            t,
                            &format!("{} differ, kept", fmt_count(total - stats.confirmed)),
                        ),
                    );
                }
                fold::settle(t, Hue::Verify, mark, &segs);
            }
            return;
        }
        let elapsed = {
            let st = self.lock();
            match &st.current {
                Some(c) if c.stage == stage => c.started.elapsed(),
                _ => Duration::ZERO,
            }
        };
        let text = self.stage_summary(stage, stats, elapsed);
        self.finish(stage, text);
    }

    fn tick(&self, stage: Stage, done: u64, total: u64) {
        if self.theme.fold {
            let hue = self.section_of(stage);
            if fold::current() != Some(hue) {
                fold::open(&self.theme, hue);
            }
            let eta = eta_text(stage, fold::since_open(), done, total)
                .map(|e| format!("· {e}"))
                .unwrap_or_default();
            fold::step(phase_of(stage), done, Some(total), eta);
            return;
        }
        let mut st = self.lock();
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
            if let Some(eta) = eta_text(stage, c.started.elapsed(), done, total) {
                c.text.push_str("   ");
                c.text.push_str(&self.theme.paint(Role::Dupe, &eta));
            }
        }
        st.ticks += 1;
        self.draw(&mut st, false);
    }
}

/// The stage's word on a fold line.
fn phase_of(stage: Stage) -> &'static str {
    match stage {
        Stage::Scan => "",
        Stage::Size => "size",
        Stage::Sample => "sample",
        Stage::Hash => "hash",
        Stage::Verify => "verify",
        Stage::Decode => "decode",
        Stage::Match => "match",
        Stage::Frames => "frames",
    }
}

/// Seconds a stage runs before it estimates the rest: the first files
/// carry the start-up (decoder, model, pool) and say little about the
/// rate. [CLAUDE, Phase 11]
const ETA_AFTER: f64 = 5.0;

/// `about 40 s left`, from the stage's running rate (QOL-07, Phase 11):
/// the time so far times the share still to do. Only DECODE and FRAMES,
/// whose items cost about the same; HASH and VERIFY take the largest
/// files first, so a count would overstate the rest. Rounded so the
/// number does not flicker: to 5 s under a minute, to 30 s under ten, to
/// the minute above. `None` before `ETA_AFTER`, or with under 3 s left.
fn eta_text(stage: Stage, elapsed: Duration, done: u64, total: u64) -> Option<String> {
    if !matches!(stage, Stage::Decode | Stage::Frames) || done == 0 || done >= total {
        return None;
    }
    let spent = elapsed.as_secs_f64();
    if spent < ETA_AFTER {
        return None;
    }
    let left = spent * (total - done) as f64 / done as f64;
    if left < 3.0 {
        return None;
    }
    let up = |v: f64, step: f64| ((v / step).ceil() * step) as u64;
    Some(if left < 60.0 {
        format!("about {} s left", up(left, 5.0))
    } else if left < 600.0 {
        let s = up(left, 30.0);
        match s % 60 {
            0 => format!("about {} m left", s / 60),
            _ => format!("about {} m 30 s left", s / 60),
        }
    } else if left < 3600.0 {
        format!("about {} m left", up(left, 60.0) / 60)
    } else {
        let m = up(left, 60.0) / 60;
        format!("about {} h {:02} m left", m / 60, m % 60)
    })
}

/// Cells before a stage line's bar (margin, label, space) and after it
/// (the gap before the text).
const STAGE_PRE: usize = 1 + 6 + 1;
const STAGE_GAP: usize = 2;

/// The text a stage line keeps at least, the bar giving way first.
const STAGE_TEXT_MIN: usize = 24;

/// The bar's cells on a line `width` wide: `BAR_CELLS`, fewer when the
/// text would keep under `STAGE_TEXT_MIN`, never under 8 — at 40 columns
/// the bar took every cell and the text none (Phase 16, QOL-11).
pub(crate) fn bar_cells(width: usize) -> usize {
    width
        .saturating_sub(STAGE_PRE + STAGE_GAP + 1 + STAGE_TEXT_MIN)
        .clamp(8, BAR_CELLS)
}

/// One stage line: label, bar (`filled` cells of `bar_cells`, or the
/// sweep when `usize::MAX`), and the text's segments, dropped from the
/// right rather than wrapped, which would leave the in-place redraw
/// behind.
fn compose(
    t: &Theme,
    stage: Stage,
    finished: bool,
    filled: usize,
    ticks: u64,
    text: &str,
) -> String {
    let cells = bar_cells(t.width);
    let bar = if filled == usize::MAX {
        sweep(t, ticks, cells)
    } else {
        let on = filled.min(cells);
        format!(
            "{}{}",
            t.paint(
                if finished { Role::Ok } else { Role::Keep },
                &t.glyphs.bar_on.repeat(on)
            ),
            t.paint(Role::Rule, &t.glyphs.bar_off.repeat(cells - on))
        )
    };
    let label = fit(stage.label(), 6);
    let label = if finished {
        t.paint(Role::Plate, &label)
    } else {
        t.paint_bold(Role::Plate, &label)
    };
    let segs: Vec<String> = text.split("   ").map(str::to_string).collect();
    let room = t.width.saturating_sub(STAGE_PRE + cells + STAGE_GAP + 1);
    let mut text = join_fit(&segs, room);
    if visible_width(&text) > room {
        // one segment longer than the room: its words up to the edge
        text = crate::util::fit(&text, room);
    }
    format!(" {label} {bar}  {text}")
}

/// A finished stage's line at `width`, as `draw` leaves it behind (for
/// the width test).
#[cfg(test)]
pub(crate) fn stage_line(t: &Theme, width: usize, stage: Stage, text: &str) -> String {
    let mut t = *t;
    t.width = width;
    compose(&t, stage, true, usize::MAX - 1, 0, text)
}

/// Stage texts as long as a real scan writes them, for the width test.
#[cfg(test)]
pub(crate) fn sample_stage_texts() -> Vec<(Stage, String)> {
    vec![
        (Stage::Scan, "1,492 files   269 MB   2 bin and system folders left out   3 online only, not read   0.2 s".into()),
        (Stage::Size, "1,204 share a size   0.004 s".into()),
        (Stage::Sample, "400 share a sample   135 MB read   0.04 s".into()),
        (Stage::Hash, "400 confirmed   78 MB read   200 groups   0.02 s".into()),
        (Stage::Decode, "280 images hashed   2 timed out   12 undecodable   1 RAW preview   3.3 s".into()),
        (Stage::Match, "120 near groups   100 fits   2 split from another   1.8 s".into()),
        (Stage::Frames, "12 videos hashed   1,800 frames at 15 fps   1 timed out   12.6 s".into()),
    ]
}

fn sweep(t: &Theme, ticks: u64, cells: usize) -> String {
    let len = 6usize.min(cells);
    let span = cells - len;
    let period = (span * 2).max(1);
    let p = (ticks as usize) % period;
    let pos = if p <= span { p } else { period - p };
    format!(
        "{}{}{}",
        t.paint(Role::Rule, &t.glyphs.bar_off.repeat(pos)),
        t.paint(Role::Keep, &t.glyphs.bar_on.repeat(len)),
        t.paint(Role::Rule, &t.glyphs.bar_off.repeat(cells - len - pos))
    )
}

impl Renderer {
    pub fn scan_text(&self, s: &ScanStats, elapsed: Option<Duration>) -> String {
        scan_text(&self.theme, s, elapsed)
    }
}

/// The SCAN stage's text: files, bytes, and what the filters left out.
fn scan_text(t: &Theme, s: &ScanStats, elapsed: Option<Duration>) -> String {
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
    // files a sync client keeps online: reading them would fetch them
    if s.online > 0 {
        v.push(t.paint(
            Role::Dupe,
            &format!("{} online only, not read", fmt_count(s.online)),
        ));
    }
    // the bin and the system's own folders, never entered (Phase 13)
    if s.system > 0 {
        v.push(t.paint(
            Role::Dupe,
            &format!(
                "{} {} left out",
                fmt_count(s.system),
                plural(s.system, "bin or system folder", "bin and system folders")
            ),
        ));
    }
    if let Some(e) = elapsed {
        v.push(t.paint(Role::Dupe, &fmt_duration(e)));
    }
    v.join("   ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimates_only_long_even_stages_and_rounds_them() {
        let s = Duration::from_secs_f64;
        // a fifth done in 5 s: 20 s left, rounded up to 5 s
        assert_eq!(
            eta_text(Stage::Decode, s(5.0), 20, 100).as_deref(),
            Some("about 20 s left")
        );
        assert_eq!(
            eta_text(Stage::Frames, s(6.0), 10, 100).as_deref(),
            Some("about 55 s left")
        );
        assert_eq!(
            eta_text(Stage::Frames, s(6.0), 10, 120).as_deref(),
            Some("about 1 m 30 s left")
        );
        assert_eq!(
            eta_text(Stage::Decode, s(12.0), 10, 60).as_deref(),
            Some("about 1 m left")
        );
        assert_eq!(
            eta_text(Stage::Decode, s(10.0), 10, 200).as_deref(),
            Some("about 3 m 30 s left")
        );
        assert_eq!(
            eta_text(Stage::Decode, s(60.0), 10, 200).as_deref(),
            Some("about 19 m left")
        );
        assert_eq!(
            eta_text(Stage::Decode, s(600.0), 100, 1000).as_deref(),
            Some("about 1 h 30 m left")
        );
        // too early, nearly done, nothing done, or a stage that sorts by size
        assert_eq!(eta_text(Stage::Decode, s(4.9), 20, 100), None);
        assert_eq!(eta_text(Stage::Decode, s(60.0), 99, 100), None);
        assert_eq!(eta_text(Stage::Decode, s(60.0), 0, 100), None);
        assert_eq!(eta_text(Stage::Decode, s(60.0), 100, 100), None);
        assert_eq!(eta_text(Stage::Hash, s(60.0), 10, 100), None);
        assert_eq!(eta_text(Stage::Verify, s(60.0), 10, 100), None);
    }
}
