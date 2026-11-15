//! The fold: dedupe's default output (Phase 17, docs/CLI.md §3a). [CLAUDE]
//!
//! A run is a handful of sections — files, copies, similar, video, verify,
//! then the action — and each is one line. While a section works, its line
//! is live: a twin-dot orbit (a file and its copy circling), the label
//! lit by a sheen that travels across it, a track that fills toward the
//! stage's total (a comet that runs when the total is unknown), counts
//! that roll toward their value, the time on the right. When the section
//! is done the line folds: its mark, its label, what it found, how long it
//! took; the next section opens under it. After the sections, the yield:
//! what can be reclaimed as one figure, then a bar of everything scanned
//! on which every group has its share in its kind's colour, and under the
//! bar the largest groups named beneath their own shares. `--verbose`
//! prints the stage lines and every group and file instead.
//!
//! The live line is drawn by a ticker thread at 25 frames a second, so it
//! moves while a stage is silent (a large file hashing). Everything else
//! that writes to stdout takes `OUT` and clears the live line first; the
//! ticker redraws it under what was written on its next frame.

use std::collections::VecDeque;
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use crate::theme::{Rgb, Role, Theme};
use crate::util::{fit, fmt_bytes, fmt_count, fmt_duration, shorten_middle, visible_width, width};

/// Taken by every write to stdout while a fold may be live.
pub(crate) static OUT: Mutex<()> = Mutex::new(());
static LIVE: Mutex<Option<Live>> = Mutex::new(None);
/// A live line is on screen (read by `Renderer::line` without `LIVE`).
static LIVE_ON: AtomicBool = AtomicBool::new(false);
static TICKER: OnceLock<()> = OnceLock::new();
/// Bumped when a line opens or folds: a frame composed before is stale.
static GEN: AtomicU64 = AtomicU64::new(0);
/// The film strip is under the live line and takes pictures.
static STRIP_ON: AtomicBool = AtomicBool::new(false);
/// When the strip last took a picture, in milliseconds since `EPOCH`.
static LAST_STILL: AtomicU64 = AtomicU64::new(0);
static EPOCH: OnceLock<Instant> = OnceLock::new();

/// One frame: 25 a second.
pub(crate) const FRAME: Duration = Duration::from_millis(40);

/// The label column.
const LABEL: usize = 8;

/// The groove of an empty track and the dim parts of a line.
pub(crate) const GROOVE: Rgb = Rgb(0x3A, 0x3A, 0x3A);
pub(crate) const DIM: Rgb = Rgb(0x8A, 0x8A, 0x8A);
pub(crate) const PLATE: Rgb = Rgb(0xE8, 0xE4, 0xD8);

/// A section: its label and its colour, which its groups carry on the
/// yield's bar too.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hue {
    Files,
    Copies,
    Similar,
    Video,
    Verify,
    Milk,
    Purge,
    Undo,
}

impl Hue {
    pub(crate) fn rgb(self) -> Rgb {
        match self {
            Hue::Files => PLATE,
            Hue::Copies => Rgb(0xFF, 0xB0, 0x00),
            Hue::Similar => Rgb(0x5F, 0xB3, 0xFF),
            Hue::Video => Rgb(0xB4, 0x8C, 0xFF),
            Hue::Verify => Rgb(0x7C, 0xD9, 0x92),
            Hue::Milk => Rgb(0xF6, 0xF3, 0xEA),
            Hue::Purge => Rgb(0xFF, 0x7A, 0x59),
            Hue::Undo => Rgb(0x5F, 0xB3, 0xFF),
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Hue::Files => "files",
            Hue::Copies => "copies",
            Hue::Similar => "similar",
            Hue::Video => "video",
            Hue::Verify => "verify",
            Hue::Milk => "milk",
            Hue::Purge => "purge",
            Hue::Undo => "undo",
        }
    }
}

/// How a section folded: done (found something), done and found nothing,
/// done with something to read (a warning), or failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mark {
    Done,
    Empty,
    Warn,
    Failed,
}

struct Live {
    theme: Theme,
    hue: Hue,
    opened: Instant,
    phase: &'static str,
    done: u64,
    total: Option<u64>,
    /// `done` as drawn: it rolls toward the value rather than jumping.
    shown: f64,
    drawn: Instant,
    /// After the count: the unit, the time left, the file in hand.
    detail: String,
    /// The film strip under the line, on a long pass (`strip_open`).
    strip: Option<Strip>,
    /// The track's ribs: how far they have run, when last drawn.
    ribs: Mutex<(f64, Instant)>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Open `hue`'s section: its line goes live under the last one.
pub(crate) fn open(t: &Theme, hue: Hue) {
    let now = Instant::now();
    let mut live = lock(&LIVE);
    GEN.fetch_add(1, Ordering::SeqCst);
    *live = Some(Live {
        theme: *t,
        hue,
        opened: now,
        phase: "",
        done: 0,
        total: None,
        shown: 0.0,
        drawn: now,
        detail: String::new(),
        strip: None,
        ribs: Mutex::new((0.0, now)),
    });
    STRIP_ON.store(false, Ordering::SeqCst);
    if t.live {
        start_ticker();
        if !LIVE_ON.swap(true, Ordering::SeqCst) {
            let _o = lock(&OUT);
            let mut out = io::stdout().lock();
            let _ = write!(out, "\x1b[?25l");
            let _ = out.flush();
        }
    }
}

/// The section whose line is live.
pub(crate) fn current() -> Option<Hue> {
    lock(&LIVE).as_ref().map(|l| l.hue)
}

/// The live stage's total, when it has one.
pub(crate) fn total() -> Option<u64> {
    lock(&LIVE).as_ref().and_then(|l| l.total)
}

/// Time since the live section opened.
pub(crate) fn since_open() -> Duration {
    lock(&LIVE)
        .as_ref()
        .map_or(Duration::ZERO, |l| l.opened.elapsed())
}

/// What the live line says: the stage's word, how far it is, of what
/// total, and what follows the count.
pub(crate) fn step(phase: &'static str, done: u64, total: Option<u64>, detail: String) {
    if let Some(l) = lock(&LIVE).as_mut() {
        if l.phase != phase {
            // a new stage in the same section starts its count afresh
            l.shown = 0.0;
        }
        l.phase = phase;
        l.done = done;
        l.total = total;
        l.detail = detail;
    }
}

/// Fold `hue`'s section: its settled line replaces the live one (or, if
/// it never went live, is printed with no time). `segments` are what it
/// found, most important first; those that do not fit go from the right.
pub(crate) fn settle(t: &Theme, hue: Hue, mark: Mark, segments: &[String]) {
    let mut live = lock(&LIVE);
    let elapsed = match live.as_ref() {
        Some(l) if l.hue == hue => Some(l.opened.elapsed()),
        _ => None,
    };
    *live = None;
    STRIP_ON.store(false, Ordering::SeqCst);
    GEN.fetch_add(1, Ordering::SeqCst);
    drop(live);
    let line = settled_line(t, hue, mark, segments, elapsed);
    let _o = lock(&OUT);
    let mut out = io::stdout().lock();
    if LIVE_ON.swap(false, Ordering::SeqCst) {
        // the live line and the strip under it go; the settled line stays
        let _ = write!(out, "\r\x1b[J{line}\n\x1b[?25h");
    } else {
        let _ = writeln!(out, "{line}");
    }
    let _ = out.flush();
}

/// Before anything else writes to stdout: the live line and the strip
/// under it off the screen (the ticker draws them again under what is
/// written).
pub(crate) fn clear_for_write(out: &mut impl Write) {
    if LIVE_ON.load(Ordering::SeqCst) {
        let _ = write!(out, "\r\x1b[J");
    }
}

/// Ctrl-C while a line is live: the strip gone, the cursor back, the line
/// ended.
pub(crate) fn abandon() {
    STRIP_ON.store(false, Ordering::SeqCst);
    if LIVE_ON.swap(false, Ordering::SeqCst) {
        let mut out = io::stdout();
        let _ = write!(out, "\x1b[s\n\x1b[J\x1b[u");
        let _ = writeln!(out, "\x1b[?25h");
        let _ = out.flush();
    }
}

fn start_ticker() {
    TICKER.get_or_init(|| {
        let _ = std::thread::Builder::new()
            .name("fold".into())
            .spawn(|| loop {
                std::thread::sleep(FRAME);
                // composed under LIVE, written under OUT only: the workers
                // that step the line never wait on the terminal
                let (line, gen) = {
                    let mut live = lock(&LIVE);
                    let Some(l) = live.as_mut() else { continue };
                    if !l.theme.live {
                        continue;
                    }
                    let now = Instant::now();
                    l.roll(now);
                    let mut frame = format!("\r{}\x1b[K", live_line(l, now));
                    if let Some(rows) = strip_lines(l, now) {
                        // the strip under the line, then back up to it
                        let n = rows.len();
                        for r in rows {
                            frame.push_str(&format!("\n{r}\x1b[K"));
                        }
                        frame.push_str(&format!("\x1b[{n}A\r"));
                    }
                    (frame, GEN.load(Ordering::SeqCst))
                };
                let _o = lock(&OUT);
                if GEN.load(Ordering::SeqCst) != gen || !LIVE_ON.load(Ordering::SeqCst) {
                    continue;
                }
                let mut out = io::stdout().lock();
                let _ = write!(out, "{line}");
                let _ = out.flush();
            });
    });
}

impl Live {
    /// Move the drawn count toward the real one: most of the way in a
    /// tenth of a second, so a burst reads as a roll and a stall as still.
    fn roll(&mut self, now: Instant) {
        let dt = now.duration_since(self.drawn).as_secs_f64();
        self.drawn = now;
        let target = self.done as f64;
        if self.total.is_some_and(|t| self.done >= t) || (target - self.shown).abs() < 0.5 {
            self.shown = target;
            return;
        }
        let k = 1.0 - (-dt / 0.09).exp();
        self.shown += (target - self.shown) * k;
    }
}

// ---------------------------------------------------------------------------
// drawing
// ---------------------------------------------------------------------------

/// Characters in runs of one colour, so a frame is a few escapes, not one
/// per cell.
struct Brush<'a> {
    t: &'a Theme,
    out: String,
    run: String,
    colour: Option<(Rgb, bool)>,
}

impl<'a> Brush<'a> {
    fn new(t: &'a Theme) -> Brush<'a> {
        Brush {
            t,
            out: String::new(),
            run: String::new(),
            colour: None,
        }
    }

    fn put(&mut self, c: Rgb, bold: bool, s: &str) {
        if self.colour != Some((c, bold)) {
            self.flush();
            self.colour = Some((c, bold));
        }
        self.run.push_str(s);
    }

    fn raw(&mut self, s: &str) {
        self.flush();
        self.colour = None;
        self.out.push_str(s);
    }

    /// A cell on its own ground: written at once, the run ended.
    fn put_on(&mut self, fg: Rgb, bg: Rgb, s: &str) {
        self.flush();
        self.colour = None;
        if self.t.color {
            let (Rgb(r, g, b), Rgb(br, bgc, bb)) = (fg, bg);
            self.out.push_str(&format!(
                "\x1b[38;2;{r};{g};{b};48;2;{br};{bgc};{bb}m{s}\x1b[0m"
            ));
        } else {
            self.out.push_str(s);
        }
    }

    fn flush(&mut self) {
        if self.run.is_empty() {
            return;
        }
        let run = std::mem::take(&mut self.run);
        match self.colour {
            Some((c, true)) => self.out.push_str(&self.t.paint_rgb_bold(c, &run)),
            Some((c, false)) => self.out.push_str(&self.t.paint_rgb(c, &run)),
            None => self.out.push_str(&run),
        }
    }

    fn done(mut self) -> String {
        self.flush();
        self.out
    }
}

/// The twin-dot orbit: two dots opposite each other on the rim of a
/// braille cell, a quarter turn a frame.
const ORBIT: [&str; 4] = ["⢁", "⡈", "⠔", "⠢"];
const ORBIT_ASCII: [&str; 4] = ["|", "/", "-", "\\"];

fn wave(secs: f64, period: f64) -> f64 {
    0.5 + 0.5 * (secs / period * std::f64::consts::TAU).sin()
}

/// The ribs' speed at a stage's start, cells a second.
const RIB_SPEED: f64 = 3.0;

/// Where the ribs are: their phase integrates a speed that rises with
/// `p`, so they quicken smoothly instead of jumping when the count does.
fn rib_phase(l: &Live, now: Instant, p: f64) -> f64 {
    let mut r = l.ribs.lock().unwrap_or_else(|e| e.into_inner());
    let dt = now.duration_since(r.1).as_secs_f64().min(0.2);
    r.0 += dt * RIB_SPEED * (1.0 + 3.0 * p * p);
    r.1 = now;
    r.0
}

/// The track's cells on a line `w` wide: none under 64 columns, where
/// the words need the room.
fn track_cells(w: usize) -> usize {
    if w < 64 {
        0
    } else {
        (w / 5).clamp(12, 28)
    }
}

fn live_line(l: &Live, now: Instant) -> String {
    let t = &l.theme;
    let secs = now.duration_since(l.opened).as_secs_f64();
    let hue = l.hue.rgb();
    let mut b = Brush::new(t);
    b.raw(" ");

    // the orbit, breathing
    let frame = (secs / 0.09) as usize % 4;
    let spin = if t.unicode {
        ORBIT[frame]
    } else {
        ORBIT_ASCII[frame]
    };
    b.put(
        hue.lerp(Rgb::WHITE, 0.15 + 0.35 * wave(secs, 1.1)),
        true,
        spin,
    );
    b.raw(" ");

    // the label, a sheen running across it every 1.8 s
    let label = fit(l.hue.label(), LABEL);
    let span = LABEL as f64 + 8.0;
    let pos = (secs % 1.8) / 1.8 * span - 4.0;
    for (i, ch) in label.chars().enumerate() {
        let lit = (1.0 - (i as f64 - pos).abs() / 2.2).max(0.0);
        let mut s = [0u8; 4];
        b.put(
            hue.lerp(Rgb::WHITE, 0.8 * lit),
            true,
            ch.encode_utf8(&mut s),
        );
    }
    b.raw(" ");

    // the track
    let cells = track_cells(t.width);
    if cells > 0 {
        let (on, off, head) = if t.unicode {
            ("━", "─", "╸")
        } else {
            ("=", "-", "=")
        };
        match l.total {
            Some(total) if total > 0 => {
                let p = (l.shown / total as f64).clamp(0.0, 1.0);
                let f = p * cells as f64;
                let full = f.floor() as usize;
                let half = f - full as f64 >= 0.5;
                // ribs in the fill that run back against it, quickening as
                // the stage nears its end: the two augmentations that made
                // a bar's wait read shorter (Harrison, Yeo, Hudson, CHI
                // 2010). One rib every four cells, `RIB_SPEED` cells a
                // second at the start, four times that at the end.
                let phase = rib_phase(l, now, p);
                for i in 0..cells {
                    if i < full {
                        let base = hue.lerp(GROOVE, 0.4 - 0.4 * i as f64 / cells as f64);
                        let d = ((i as f64 + phase).rem_euclid(4.0) - 1.0).abs();
                        let rib = (1.0 - d).max(0.0) * 0.38;
                        b.put(base.lerp(Rgb::WHITE, rib), false, on);
                    } else if i == full && half {
                        b.put(hue, false, head);
                    } else {
                        b.put(GROOVE, false, off);
                    }
                }
            }
            _ => {
                // the comet: a bright head, a tail that fades behind it
                let headpos = (secs * 20.0) % (cells as f64 + 8.0) - 1.0;
                for i in 0..cells {
                    let d = headpos - i as f64;
                    if (0.0..7.0).contains(&d) {
                        b.put(GROOVE.lerp(hue, 1.0 - d / 7.0), false, on);
                    } else {
                        b.put(GROOVE, false, off);
                    }
                }
            }
        }
        b.raw("  ");
    }

    // the count, then its detail; the time on the right
    let clock = clock(now.duration_since(l.opened));
    let used = 1 + 1 + 1 + LABEL + 1 + if cells > 0 { cells + 2 } else { 0 };
    let room = t.width.saturating_sub(used + 2 + width(&clock) + 1);
    let mut words: Vec<(Rgb, bool, String)> = Vec::new();
    if !l.phase.is_empty() {
        words.push((DIM, false, format!("{} ", l.phase)));
    }
    let shown = fmt_count(l.shown.round() as u64);
    words.push((PLATE, true, shown));
    if let Some(total) = l.total {
        words.push((DIM, false, format!(" / {}", fmt_count(total))));
    }
    if !l.detail.is_empty() {
        words.push((DIM, false, format!(" {}", l.detail)));
    }
    let mut left = room;
    let mut wrote = 0;
    for (c, bold, s) in words {
        if left == 0 {
            break;
        }
        let s = if width(&s) > left { fit(&s, left) } else { s };
        left -= width(&s);
        wrote += width(&s);
        b.put(c, bold, &s);
    }
    b.raw(&" ".repeat(room - wrote + 2));
    b.put(DIM, false, &clock);
    b.done()
}

// ---------------------------------------------------------------------------
// the darkroom: a film strip under a long pass [CLAUDE, Phase 19]
// ---------------------------------------------------------------------------
//
// While the similar pass decodes a large folder, the pictures it reads go
// by under its line on a strip of film: each frame the next picture on
// quadrant blocks, sprocket holes above and below that travel with the
// film, the newest frame entering at the right and developing out of the
// dark (a moment in the amber of a safelight first). Under the strip, how
// fast it goes: a trace of pictures a second over the last twenty
// seconds, the figure, the file in hand. Time with something to watch in
// it passes faster; the track's ribs (`live_line`) do the rest.

/// A frame on the strip, in cells: a 4:3 picture fills it on quadrant
/// blocks (22 × 8 pixels, each twice as tall as wide).
pub(crate) const FRAME_W: usize = 11;
pub(crate) const FRAME_H: usize = 4;
/// The strip's rows: sprockets, the frames, sprockets, the trace.
const STRIP_ROWS: usize = FRAME_H + 3;
/// Rows a terminal needs for the strip to be drawn under the line.
const STRIP_MIN_ROWS: u16 = 20;
/// Columns it needs.
const STRIP_MIN_COLS: usize = 60;
/// How often the strip takes a picture.
const STILL_EVERY: Duration = Duration::from_millis(110);
/// How long a frame takes to develop.
const DEVELOP: Duration = Duration::from_millis(520);
/// The film's base, its holes, the dark a frame develops out of.
const FILM: Rgb = Rgb(0x1D, 0x1A, 0x16);
const HOLE: Rgb = Rgb(0x55, 0x4E, 0x43);
const DARK: Rgb = Rgb(0x0B, 0x09, 0x07);
const SAFELIGHT: Rgb = Rgb(0xC8, 0x3C, 0x14);

struct Still {
    px: Vec<Rgb>,
    w: usize,
    h: usize,
    came: Instant,
}

struct Strip {
    stills: VecDeque<Still>,
    /// Cells the film still has to travel left (it eases to 0).
    slide: f64,
    /// Cells it has travelled in all: the holes move with it.
    travelled: f64,
    /// (when, how many done), twice a second: the trace.
    samples: VecDeque<(Instant, u64)>,
    name: String,
}

fn epoch_ms() -> u64 {
    EPOCH.get_or_init(Instant::now).elapsed().as_millis() as u64
}

/// The live section's pass is long: the strip goes under its line, on a
/// terminal with the room for it.
pub(crate) fn strip_open() {
    let mut live = lock(&LIVE);
    let Some(l) = live.as_mut() else { return };
    if l.strip.is_some() || !l.theme.live || !l.theme.color || !l.theme.unicode {
        return;
    }
    let rows = crossterm::terminal::size().map(|(_, r)| r).unwrap_or(0);
    if rows < STRIP_MIN_ROWS || l.theme.width < STRIP_MIN_COLS {
        return;
    }
    l.strip = Some(Strip {
        stills: VecDeque::new(),
        slide: 0.0,
        travelled: 0.0,
        samples: VecDeque::new(),
        name: String::new(),
    });
    STRIP_ON.store(true, Ordering::SeqCst);
}

/// Whether the strip takes the picture just decoded (one every
/// `STILL_EVERY`; the first caller after that takes it).
pub(crate) fn wants_picture() -> bool {
    if !STRIP_ON.load(Ordering::Relaxed) {
        return false;
    }
    let now = epoch_ms();
    let last = LAST_STILL.load(Ordering::Relaxed);
    now.saturating_sub(last) >= STILL_EVERY.as_millis() as u64
        && LAST_STILL
            .compare_exchange(last, now, Ordering::SeqCst, Ordering::Relaxed)
            .is_ok()
}

/// A picture onto the strip: it enters at the right.
pub(crate) fn picture(still: &image::RgbImage, name: &str) {
    let mut live = lock(&LIVE);
    let Some(strip) = live.as_mut().and_then(|l| l.strip.as_mut()) else {
        return;
    };
    let px = still.pixels().map(|p| Rgb(p[0], p[1], p[2])).collect();
    strip.stills.push_back(Still {
        px,
        w: still.width() as usize,
        h: still.height() as usize,
        came: Instant::now(),
    });
    while strip.stills.len() > 40 {
        strip.stills.pop_front();
    }
    let step = (FRAME_W + 1) as f64;
    strip.slide = (strip.slide + step).min(3.0 * step);
    strip.travelled += step;
    strip.name = name.to_string();
}

/// The strip's rows for this frame, when it is on.
fn strip_lines(l: &mut Live, now: Instant) -> Option<Vec<String>> {
    let done = l.done;
    let hue = l.hue.rgb();
    let t = l.theme;
    let s = l.strip.as_mut()?;
    // the film eases into place: most of the way in a fifth of a second
    let dt = FRAME.as_secs_f64();
    s.slide *= (-dt / 0.07).exp();
    if s.slide < 0.05 {
        s.slide = 0.0;
    }
    if s.samples
        .back()
        .is_none_or(|(at, _)| now.duration_since(*at) >= Duration::from_millis(500))
    {
        s.samples.push_back((now, done));
        while s.samples.len() > 41 {
            s.samples.pop_front();
        }
    }
    let w = t.width.saturating_sub(2);
    let shift = s.slide.round() as usize;
    let phase = (s.travelled - s.slide).round() as i64;
    let mut rows: Vec<String> = Vec::with_capacity(STRIP_ROWS);

    // sprockets: a hole every third cell, travelling with the film
    let sprockets = |b: &mut Brush| {
        b.raw(" ");
        for c in 0..w {
            if (c as i64 + phase).rem_euclid(3) == 0 {
                b.put_on(HOLE, FILM, "▪");
            } else {
                b.put_on(FILM, FILM, " ");
            }
        }
    };
    let mut b = Brush::new(&t);
    sprockets(&mut b);
    rows.push(b.done());

    // the frames, newest at the right
    let step = FRAME_W + 1;
    for r in 0..FRAME_H {
        let mut b = Brush::new(&t);
        b.raw(" ");
        let mut c = 0usize;
        while c < w {
            // which frame this cell is in: counted from the right edge,
            // the film shifted right by what it still has to travel
            let from_right = (w - 1 - c) as i64 + shift as i64 - (step as i64 - 1);
            let k = from_right.div_euclid(step as i64);
            let x = (step as i64 - 1) - from_right.rem_euclid(step as i64) - 1;
            let still = if k >= 0 && (0..FRAME_W as i64).contains(&x) {
                s.stills
                    .len()
                    .checked_sub(1 + k as usize)
                    .map(|i| &s.stills[i])
            } else {
                None
            };
            match still {
                Some(f) => {
                    let (glyph, fg, bg) = still_cell(f, x as usize, r, now);
                    b.put_on(fg, bg, glyph);
                }
                None => b.put_on(FILM, FILM, " "),
            }
            c += 1;
        }
        rows.push(b.done());
    }
    let mut b = Brush::new(&t);
    sprockets(&mut b);
    rows.push(b.done());

    // the trace: pictures a second over the last twenty seconds
    let rates: Vec<f64> = s
        .samples
        .iter()
        .zip(s.samples.iter().skip(1))
        .map(|((a, da), (b, db))| (db - da) as f64 / b.duration_since(*a).as_secs_f64().max(1e-3))
        .collect();
    let mut b = Brush::new(&t);
    b.raw("   ");
    let top = rates.iter().cloned().fold(1.0, f64::max);
    let trace_w = 40usize.min(w / 3);
    for i in 0..trace_w {
        let v = rates
            .len()
            .checked_sub(trace_w - i)
            .and_then(|k| rates.get(k))
            .copied();
        match v {
            Some(v) => {
                let lvl = ((v / top) * 7.0).round().clamp(0.0, 7.0) as usize;
                b.put(
                    GROOVE.lerp(hue, 0.35 + 0.65 * lvl as f64 / 7.0),
                    false,
                    BLOCKS[lvl],
                );
            }
            None => b.put(GROOVE, false, "▁"),
        }
    }
    let now_rate =
        rates.iter().rev().take(4).sum::<f64>() / rates.iter().rev().take(4).count().max(1) as f64;
    b.raw("  ");
    if !rates.is_empty() {
        b.put(PLATE, true, &fmt_count(now_rate.round() as u64));
        b.put(DIM, false, " a second");
    }
    let used = 3
        + trace_w
        + 2
        + if rates.is_empty() {
            0
        } else {
            fmt_count(now_rate.round() as u64).len() + 9
        };
    let room = t.width.saturating_sub(used + 6);
    if !s.name.is_empty() && room > 8 {
        let n = std::path::Path::new(&s.name)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        b.put(DIM, false, "   ");
        b.put(
            GROOVE.lerp(PLATE, 0.55),
            false,
            &shorten_middle(&n, room, "…"),
        );
    }
    rows.push(b.done());
    Some(rows)
}

/// One cell of a frame: its four pixels as a quadrant block, the picture
/// centred in the frame on the film's base, developing out of the dark
/// through the safelight's amber.
fn still_cell(f: &Still, x: usize, r: usize, now: Instant) -> (&'static str, Rgb, Rgb) {
    let a = (now.duration_since(f.came).as_secs_f64() / DEVELOP.as_secs_f64()).min(1.0);
    let a = 1.0 - (1.0 - a).powi(3);
    let (ox, oy) = ((2 * FRAME_W - f.w) / 2, (2 * FRAME_H - f.h) / 2);
    let px = |px: usize, py: usize| -> Rgb {
        if px < ox || py < oy || px >= ox + f.w || py >= oy + f.h {
            return FILM;
        }
        let c = f.px[(py - oy) * f.w + (px - ox)];
        // dark, then the safelight's amber on the picture's light, then it
        let lum = (c.0 as f64 * 0.3 + c.1 as f64 * 0.59 + c.2 as f64 * 0.11) / 255.0;
        let lit = DARK.lerp(SAFELIGHT, lum * 0.8);
        if a < 0.45 {
            DARK.lerp(lit, a / 0.45)
        } else {
            lit.lerp(c, (a - 0.45) / 0.55)
        }
    };
    let (m, fg, bg) = crate::workspace::quad([
        px(2 * x, 2 * r),
        px(2 * x + 1, 2 * r),
        px(2 * x, 2 * r + 1),
        px(2 * x + 1, 2 * r + 1),
    ]);
    (crate::workspace::QUADS[m], fg, bg)
}

/// A section's time: milliseconds under a second, where tenths would
/// read `0.0 s` for most of the sections of a small folder.
fn clock(d: Duration) -> String {
    let ms = d.as_secs_f64() * 1000.0;
    if ms < 1.0 {
        "<1 ms".into()
    } else if ms < 1000.0 {
        format!("{} ms", ms.floor())
    } else {
        fmt_duration(d)
    }
}

/// A folded section: mark, label, the segments that fit, the time.
pub(crate) fn settled_line(
    t: &Theme,
    hue: Hue,
    mark: Mark,
    segments: &[String],
    elapsed: Option<Duration>,
) -> String {
    let (glyph, colour) = match (mark, t.unicode) {
        (Mark::Done, true) => ("✔", Rgb(0x7C, 0xD9, 0x92)),
        (Mark::Empty, true) => ("✔", Rgb(0x4F, 0x7A, 0x5A)),
        (Mark::Warn, true) => ("▲", Rgb(0xFF, 0xB0, 0x00)),
        (Mark::Failed, true) => ("✖", Rgb(0xFF, 0x4D, 0x4D)),
        (Mark::Done | Mark::Empty, false) => ("+", Rgb(0x7C, 0xD9, 0x92)),
        (Mark::Warn, false) => ("!", Rgb(0xFF, 0xB0, 0x00)),
        (Mark::Failed, false) => ("x", Rgb(0xFF, 0x4D, 0x4D)),
    };
    let clock = elapsed.map(clock).unwrap_or_default();
    let lead = 1 + 1 + 1 + LABEL + 1;
    let tail = if clock.is_empty() {
        1
    } else {
        2 + width(&clock) + 1
    };
    let room = t.width.saturating_sub(lead + tail);
    let text = fit_segments(t, segments, room);
    let pad = room.saturating_sub(visible_width(&text));
    let label_colour = if mark == Mark::Empty { DIM } else { hue.rgb() };
    format!(
        " {} {} {}{}{}",
        t.paint_rgb_bold(colour, glyph),
        t.paint_rgb(label_colour, &fit(hue.label(), LABEL)),
        text,
        " ".repeat(pad + if clock.is_empty() { 0 } else { 2 }),
        t.paint_rgb(DIM, &clock)
    )
}

/// The segments joined by a dim dot, as many as fit `max`, from the left;
/// a first segment longer than `max` is cut at the edge.
pub(crate) fn fit_segments(t: &Theme, segments: &[String], max: usize) -> String {
    let dot = t.paint_rgb(GROOVE.lerp(DIM, 0.5), if t.unicode { " · " } else { "   " });
    let mut out = String::new();
    let mut used = 0;
    for (i, s) in segments.iter().enumerate() {
        let w = visible_width(s);
        let sep = if i == 0 { 0 } else { 3 };
        if used + sep + w > max {
            if i == 0 {
                return fit(&crate::util::strip_sgr(s), max);
            }
            break;
        }
        if i > 0 {
            out.push_str(&dot);
        }
        out.push_str(s);
        used += sep + w;
    }
    out
}

/// A number lit, its words dim: `12 groups`.
pub(crate) fn count(t: &Theme, n: u64, one: &str, many: &str) -> String {
    format!(
        "{} {}",
        t.paint_rgb_bold(PLATE, &fmt_count(n)),
        t.paint_rgb(DIM, if n == 1 { one } else { many })
    )
}

/// A count and the groups it is in: `11 in 10 groups`.
pub(crate) fn within(t: &Theme, n: u64, groups: u64) -> String {
    format!(
        "{} {} {}",
        t.paint_rgb_bold(PLATE, &fmt_count(n)),
        t.paint_rgb(DIM, "in"),
        count(t, groups, "group", "groups")
    )
}

/// The cells an aside's words have.
pub(crate) fn aside_room(t: &Theme) -> usize {
    t.width.saturating_sub(1 + 1 + 1 + LABEL + 1 + 1)
}

/// A line under the section above it, in its text column, hooked to it:
/// why something in it is as it is.
pub(crate) fn aside_line(t: &Theme, segments: &[String]) -> String {
    let lead = 1 + 1 + 1 + LABEL + 1;
    let hook = if t.unicode { "└ " } else { "` " };
    let room = aside_room(t);
    format!(
        "{}{}{}",
        " ".repeat(lead - 2),
        t.paint_rgb(GROOVE.lerp(DIM, 0.5), hook),
        fit_segments(t, segments, room)
    )
}

/// A figure lit: `3.02 MB`.
pub(crate) fn figure(t: &Theme, s: &str) -> String {
    t.paint_rgb_bold(PLATE, s)
}

/// Words dim.
pub(crate) fn dim(t: &Theme, s: &str) -> String {
    t.paint_rgb(DIM, s)
}

/// Words that ask to be read: a warning.
pub(crate) fn warn(t: &Theme, s: &str) -> String {
    t.paint(Role::Warn, s)
}

// ---------------------------------------------------------------------------
// the yield
// ---------------------------------------------------------------------------

/// What a run can give back: everything scanned, each kind's share, and
/// every group with its keeper's name.
pub(crate) struct Yield {
    pub(crate) scanned: u64,
    pub(crate) files_scanned: u64,
    /// Bytes each kind frees, in the order of the sections.
    pub(crate) parts: Vec<(Hue, u64)>,
    /// Files those bytes are.
    pub(crate) files: u64,
    /// Every group in screen order: its kind, what it frees, its keeper.
    pub(crate) groups: Vec<(Hue, u64, String)>,
}

impl Yield {
    pub(crate) fn total(&self) -> u64 {
        self.parts.iter().map(|p| p.1).sum()
    }
}

pub(crate) const BLOCKS: [&str; 8] = ["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];

/// What members are, by their words without the figures, most common
/// first (a tie in the order given): `resized 75%` and `resized 50%` are
/// `resized 2`; `another shot, 05:06:08` is `another shot`; `similar,
/// unconfirmed` is `unconfirmed`.
pub(crate) fn tally(words: impl Iterator<Item = String>) -> Vec<(String, u64)> {
    let mut v: Vec<(String, u64)> = Vec::new();
    for w in words {
        let k = if w.starts_with("another shot") {
            "another shot".to_string()
        } else if w == "similar, unconfirmed" {
            "unconfirmed".to_string()
        } else {
            w.split(',')
                .next()
                .unwrap_or("")
                .split_whitespace()
                .filter(|x| !x.chars().any(|c| c.is_ascii_digit()))
                .collect::<Vec<_>>()
                .join(" ")
        };
        if k.is_empty() {
            continue;
        }
        match v.iter_mut().find(|e| e.0 == k) {
            Some(e) => e.1 += 1,
            None => v.push((k, 1)),
        }
    }
    v.sort_by_key(|e| std::cmp::Reverse(e.1));
    v
}

/// A tally as segments: the word dim, a count above one lit.
pub(crate) fn tally_segments(t: &Theme, v: Vec<(String, u64)>) -> Vec<String> {
    v.into_iter()
        .map(|(k, n)| {
            if n == 1 {
                dim(t, &k)
            } else {
                format!("{} {}", dim(t, &k), figure(t, &fmt_count(n)))
            }
        })
        .collect()
}

fn ease_out(p: f64) -> f64 {
    1.0 - (1.0 - p.clamp(0.0, 1.0)).powi(3)
}

/// The yield's lines at `p` of its reveal (1 is the final picture).
pub(crate) fn yield_lines(t: &Theme, y: &Yield, p: f64) -> Vec<String> {
    let total = y.total();
    let e = ease_out(p);
    let max = t.width.saturating_sub(1);
    if total == 0 {
        let lead = if y.groups.is_empty() {
            t.paint_bold(Role::Ok, "no duplicates")
        } else {
            t.paint_bold(Role::Ok, "nothing reclaimable")
        };
        let rest = if y.groups.is_empty() {
            vec![
                count(t, y.files_scanned, "file", "files"),
                format!("{} {}", figure(t, &fmt_bytes(y.scanned)), dim(t, "checked")),
            ]
        } else {
            vec![
                count(t, y.groups.len() as u64, "group", "groups"),
                dim(t, "listed to look at; none confirmed"),
            ]
        };
        let text = fit_segments(t, &rest, max.saturating_sub(1 + visible_width(&lead) + 3));
        return vec![format!(" {lead}   {text}")];
    }

    // the figure, rolling up with the bar
    let shown = (total as f64 * e).round() as u64;
    let pct = if y.scanned > 0 {
        (total as f64 / y.scanned as f64 * 100.0).round() as u64
    } else {
        0
    };
    let big = t.paint_rgb_bold(Rgb::WHITE, &fmt_bytes(shown));
    let head = vec![
        count(t, y.files, "file", "files"),
        count(t, y.groups.len() as u64, "group", "groups"),
        format!(
            "{} {} {}",
            figure(t, &format!("{pct}%")),
            dim(t, "of"),
            figure(t, &fmt_bytes(y.scanned))
        ),
    ];
    let lead = format!(" {big} {}", dim(t, "reclaimable"));
    let room = max.saturating_sub(visible_width(&lead) + 3);
    let mut lines = vec![format!("{lead}   {}", fit_segments(t, &head, room))];

    // the bar: everything scanned; on it every group's share, in its
    // kind's colour, neighbours in two tones so each group shows
    let cells = max.saturating_sub(1);
    let spans = spans(y, cells);
    let poured = (spans.last().map_or(0, |s| s.end) as f64 * e).round() as usize;
    let (on, off) = match (t.unicode, t.color) {
        (true, true) => ("▄", "▄"),
        // without colour the groove must look like one: a baseline
        (true, false) => ("▄", "▁"),
        _ => ("#", "."),
    };
    let mut b = Brush::new(t);
    b.raw(" ");
    let mut drawn = 0usize;
    for s in &spans {
        let upto = s.end.min(poured);
        if upto > drawn {
            b.put(s.tone, false, &on.repeat(upto - drawn));
            drawn = upto;
        }
    }
    b.put(GROOVE, false, &off.repeat(cells.saturating_sub(drawn)));
    lines.push(b.done());

    // under the bar, the largest groups named beneath their own share,
    // each as its share is poured
    let named = name_spans(t, &spans, cells);
    if !named.is_empty() {
        let mut b = Brush::new(t);
        b.raw(" ");
        let mut col = 0;
        let (right, left) = if t.unicode {
            ("╰ ", " ╯")
        } else {
            ("^ ", " ^")
        };
        for n in named {
            let s = &spans[n.span];
            if s.end > poured {
                continue;
            }
            b.raw(&" ".repeat(n.from - col));
            if n.hook == n.from {
                b.put(s.tone, false, right);
                b.put(s.hue.rgb(), false, &n.name);
                b.put(DIM, false, &format!(" {}", n.size));
            } else {
                b.put(s.hue.rgb(), false, &n.name);
                b.put(DIM, false, &format!(" {}", n.size));
                b.put(s.tone, false, left);
            }
            col = n.to;
        }
        lines.push(b.done());
    }
    lines
}

/// One group's cells on the yield's bar: `start..end`, its tone, its
/// keeper's name, what it frees.
struct Span<'a> {
    start: usize,
    end: usize,
    hue: Hue,
    tone: Rgb,
    name: &'a str,
    bytes: u64,
}

/// The groups along a bar `cells` wide, kind by kind in the sections'
/// order, the largest first within a kind. A group's cells are its share
/// of everything scanned; a kind with bytes has a cell however small its
/// share. Neighbours in a kind alternate between its colour and a deeper
/// tone of it, so a group as narrow as one cell is told from the next.
fn spans(y: &Yield, cells: usize) -> Vec<Span<'_>> {
    let mut out = Vec::new();
    let mut acc = 0u64;
    let mut end = 0usize;
    for &(hue, _) in &y.parts {
        let mut gs: Vec<&(Hue, u64, String)> =
            y.groups.iter().filter(|g| g.0 == hue && g.1 > 0).collect();
        gs.sort_by_key(|g| std::cmp::Reverse(g.1));
        let kind_start = end;
        let n = gs.len();
        let mut k = 0;
        for (j, g) in gs.into_iter().enumerate() {
            acc += g.1;
            let mut upto = if y.scanned > 0 {
                ((acc as f64 / y.scanned as f64) * cells as f64).round() as usize
            } else {
                cells
            };
            if j + 1 == n {
                upto = upto.max(kind_start + 1);
            }
            let upto = upto.min(cells).max(end);
            if upto > end {
                // a group of one cell is a sliver of its kind, not a group
                // to tell apart: the kind's colour, and the next group of
                // two cells or more takes the turn
                let base = hue.rgb();
                let tone = if upto - end < 2 || k % 2 == 0 {
                    base
                } else {
                    base.lerp(GROOVE, 0.45)
                };
                out.push(Span {
                    start: end,
                    end: upto,
                    hue,
                    tone,
                    name: &g.2,
                    bytes: g.1,
                });
                if upto - end >= 2 {
                    k += 1;
                }
                end = upto;
            }
        }
    }
    out
}

/// Where the largest groups' names go under the bar, the largest first:
/// each hooked under the first cells of its own share (or, where the
/// room is to its left, ending on them), two cells clear of the others.
/// The first name with no such place ends the list, so the names shown
/// are always the largest. In the bar's order: (column, span, name, size,
/// hooked on the right).
fn name_spans(t: &Theme, spans: &[Span], cells: usize) -> Vec<Named> {
    let mut by_size: Vec<usize> = (0..spans.len()).collect();
    by_size.sort_by_key(|&i| std::cmp::Reverse(spans[i].bytes));
    let mut taken: Vec<Named> = Vec::new();
    let free = |taken: &[Named], a: usize, b: usize| {
        taken.iter().all(|o| b + 2 <= o.from || o.to + 2 <= a)
    };
    for i in by_size.into_iter().take(3) {
        let s = &spans[i];
        let size = fmt_bytes(s.bytes);
        let full = width(s.name);
        let mut placed = None;
        // the name as it is, then shortened to eight cells
        'fit: for w in [full.min(28), full.min(8)] {
            let name = shorten_middle(s.name, w, t.glyphs.ellipsis);
            let len = 2 + width(&name) + 1 + width(&size);
            // hook first: `╰ name size` from a cell of the share
            for at in s.start..s.end {
                if at + len <= cells && free(&taken, at, at + len) {
                    placed = Some(Named {
                        from: at,
                        to: at + len,
                        hook: at,
                        span: i,
                        name,
                        size: size.clone(),
                    });
                    break 'fit;
                }
            }
            // hook last: `name size ╯` ending on a cell of the share
            for at in (s.start..s.end).rev() {
                if at + 1 >= len && free(&taken, at + 1 - len, at + 1) {
                    placed = Some(Named {
                        from: at + 1 - len,
                        to: at + 1,
                        hook: at,
                        span: i,
                        name,
                        size: size.clone(),
                    });
                    break 'fit;
                }
            }
        }
        match placed {
            Some(n) => taken.push(n),
            None => break,
        }
    }
    taken.sort_by_key(|o| o.from);
    taken
}

/// A group named under the bar: the cells its words take, the cell of
/// the hook (the first or the last of them), its share, its words.
struct Named {
    from: usize,
    to: usize,
    hook: usize,
    span: usize,
    name: String,
    size: String,
}

/// Print the yield; on a terminal it is revealed over a third of a
/// second — the figure rolls up, the bar pours, each name comes as its
/// share is poured.
pub(crate) fn reveal(t: &Theme, y: &Yield) {
    let frames = if t.live && y.total() > 0 { 9 } else { 0 };
    let _o = lock(&OUT);
    let mut out = io::stdout().lock();
    if frames == 0 {
        for l in yield_lines(t, y, 1.0) {
            let _ = writeln!(out, "{l}");
        }
        let _ = out.flush();
        return;
    }
    let _ = write!(out, "\x1b[?25l");
    let mut n = 0;
    for k in 0..=frames {
        let lines = yield_lines(t, y, k as f64 / frames as f64);
        if n > 0 {
            let _ = write!(out, "\x1b[{n}A\r");
        }
        for l in &lines {
            let _ = writeln!(out, "{l}\x1b[K");
        }
        n = lines.len();
        let _ = out.flush();
        if k < frames {
            std::thread::sleep(FRAME);
        }
    }
    let _ = write!(out, "\x1b[?25h");
    let _ = out.flush();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::strip_sgr;

    fn yield_of(groups: &[(Hue, u64)]) -> Yield {
        let mut parts: Vec<(Hue, u64)> = Vec::new();
        for h in [Hue::Copies, Hue::Similar, Hue::Video] {
            let b: u64 = groups.iter().filter(|g| g.0 == h).map(|g| g.1).sum();
            if b > 0 {
                parts.push((h, b));
            }
        }
        Yield {
            scanned: 8_290_000,
            files_scanned: 33,
            parts,
            files: groups.len() as u64,
            groups: groups
                .iter()
                .enumerate()
                .map(|(i, g)| (g.0, g.1, format!("keeper_{i}.jpg")))
                .collect(),
        }
    }

    #[test]
    fn every_fold_line_fits_from_40_to_200_cells() {
        let y = yield_of(&[
            (Hue::Copies, 301_000),
            (Hue::Copies, 37_100),
            (Hue::Similar, 961_000),
            (Hue::Similar, 935_000),
            (Hue::Similar, 0),
            (Hue::Video, 2_000_000),
        ]);
        for unicode in [true, false] {
            for w in 40..=200 {
                let t = Theme::custom(true, unicode, w, true);
                let segs = vec![
                    count(&t, 10, "group", "groups"),
                    count(&t, 13, "similar", "similar"),
                    figure(&t, "2.68 MB"),
                    dim(&t, "resized 2 · mirrored 2 · re-encoded 2 · rotated 1"),
                ];
                let mut lines = yield_lines(&t, &y, 1.0);
                lines.push(settled_line(
                    &t,
                    Hue::Similar,
                    Mark::Done,
                    &segs,
                    Some(Duration::from_secs(754)),
                ));
                let l = Live {
                    theme: t,
                    hue: Hue::Similar,
                    opened: Instant::now(),
                    phase: "decode",
                    done: 1_234_567,
                    total: Some(9_876_543),
                    shown: 1_234_567.0,
                    drawn: Instant::now(),
                    detail: "about 1 h 30 m left".into(),
                    strip: None,
                    ribs: Mutex::new((0.0, Instant::now())),
                };
                lines.push(live_line(&l, Instant::now() + Duration::from_secs(4000)));
                for line in lines {
                    assert!(
                        visible_width(&line) < w,
                        "{w} cells: {} cells: {}",
                        visible_width(&line),
                        strip_sgr(&line)
                    );
                }
            }
        }
    }

    #[test]
    fn the_tally_counts_what_members_are_without_their_figures() {
        let words = [
            "resized 75%",
            "mirrored",
            "resized 50%",
            "rotated 90°",
            "another shot, 05:06:08",
            "similar, unconfirmed",
            "mirrored, re-encoded",
            "rotated +5°",
        ];
        let v = tally(words.iter().map(|w| w.to_string()));
        assert_eq!(
            v,
            [
                ("resized".to_string(), 2),
                ("mirrored".to_string(), 2),
                ("rotated".to_string(), 2),
                ("another shot".to_string(), 1),
                ("unconfirmed".to_string(), 1),
            ]
        );
    }

    #[test]
    fn the_bar_gives_each_kind_its_share_and_a_cell_at_least() {
        let t = Theme::custom(false, true, 102, false);
        let y = yield_of(&[(Hue::Copies, 4_145_000), (Hue::Similar, 1)]);
        let bar = &yield_lines(&t, &y, 1.0)[1];
        // 100 cells for 8.29 MB: half for the copies, one for the 1-byte
        // similar, the groove the rest
        assert_eq!(bar.matches('▄').count(), 51);
        assert_eq!(bar.matches('▁').count(), 49);
        let t = Theme::custom(false, false, 102, false);
        let bar = &yield_lines(&t, &y, 1.0)[1];
        assert_eq!(bar.matches('#').count(), 51);
        assert_eq!(bar.matches('.').count(), 49);
    }

    #[test]
    fn the_names_under_the_bar_are_the_largest_each_under_its_share() {
        let y = yield_of(&[
            (Hue::Copies, 301_000),
            (Hue::Copies, 37_100),
            (Hue::Similar, 961_000),
            (Hue::Similar, 935_000),
            (Hue::Similar, 358_000),
            (Hue::Similar, 12_000),
            (Hue::Video, 700_000),
        ]);
        for unicode in [true, false] {
            for w in 40..=200 {
                let t = Theme::custom(true, unicode, w, true);
                let cells = w - 2;
                let spans = spans(&y, cells);
                let named = name_spans(&t, &spans, cells);
                // largest first, none skipped: the names are a prefix of
                // the groups by size
                let mut sizes: Vec<u64> = named.iter().map(|n| spans[n.span].bytes).collect();
                sizes.sort_by_key(|b| std::cmp::Reverse(*b));
                let mut all: Vec<u64> = spans.iter().map(|s| s.bytes).collect();
                all.sort_by_key(|b| std::cmp::Reverse(*b));
                assert_eq!(sizes, all[..sizes.len()], "{w}");
                assert!(!named.is_empty(), "{w}: the largest has a place");
                for (k, n) in named.iter().enumerate() {
                    let s = &spans[n.span];
                    assert!(
                        (s.start..s.end).contains(&n.hook),
                        "{w}: hook off its share"
                    );
                    assert!(n.hook == n.from || n.hook + 1 == n.to);
                    assert!(n.to <= cells, "{w}: past the bar");
                    if k > 0 {
                        assert!(named[k - 1].to + 2 <= n.from, "{w}: names touch");
                    }
                }
                let lines = yield_lines(&t, &y, 1.0);
                assert_eq!(lines.len(), 3);
                assert!(visible_width(&lines[2]) < w);
            }
        }
    }

    #[test]
    fn the_clock_reads_milliseconds_under_a_second() {
        assert_eq!(clock(Duration::from_micros(300)), "<1 ms");
        assert_eq!(clock(Duration::from_micros(1_900)), "1 ms");
        assert_eq!(clock(Duration::from_millis(551)), "551 ms");
        assert_eq!(clock(Duration::from_millis(1_250)), "1.2 s");
    }
}
