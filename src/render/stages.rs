//! The stage lines while a scan runs: one per stage, redrawn in place on
//! a terminal, its summary left behind; the time left on the long ones.

use std::io::{self, Write};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::exact::{PipelineStats, Progress, Stage};
use crate::near::NearStats;
use crate::theme::{Role, Theme};
use crate::util::{fit, fmt_bytes, fmt_count, fmt_duration, join_fit, plural};
use crate::video::VideoStats;
use crate::walk::ScanStats;

use super::{Renderer, BAR_CELLS, REDRAW_EVERY, STAGE_LEAD};

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

    /// Close the current stage with its final text; the line stays.
    pub fn finish(&self, stage: Stage, text: String) {
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
        let st = self.lock();
        match &st.current {
            Some(c) if c.stage == stage => c.started.elapsed(),
            _ => Duration::ZERO,
        }
    }
}

impl Progress for LiveProgress {
    fn done(&self, stage: Stage, stats: &PipelineStats) {
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

impl Renderer {
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
