//! The result is the workspace (docs/CLI.md §2a; Phase 8, deepened in
//! Phase 9). After `find` lists its groups on a terminal, the same groups
//! open in a screen of their own: a cursor, a mark beside every file that
//! says whether it will be acted on, and one key per decision — choose,
//! change the keeper, look, reveal, preview, move, bin, undo. Every action
//! works on the groups already found; nothing is scanned twice. [CLAUDE,
//! under the brief's FUOE rule: the operational experience is the design]
//!
//! This module is the shell: the terminal, the files, the actions. The
//! decisions live in `state` and are tested there without a terminal.
//! What an action prints goes to the main screen, under the listing `find`
//! printed, so the scrollback is the record of the sitting; the groups
//! screen is the alternate screen and leaves no trace.

mod screen;
pub mod state;

use std::collections::HashSet;
use std::io::{self, Write};
use std::path::PathBuf;

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::{cursor, execute, queue, terminal};

use crate::actions::{self, Found, EXIT_OK};
use crate::cli::{Command, Invocation};
use crate::fsops::ActError;
use crate::preview::{self, Protocol, Visual};
use crate::render::Renderer;
use crate::theme::Role as Paint;
use crate::util::{fmt_count, rel_display, shorten_middle};

use state::{Act, Effect, GroupInfo, Key, Layout, MemberInfo, Row, State};

/// The workspace's view of the groups.
pub(crate) fn layout_of(f: &Found, root: &std::path::Path) -> Layout {
    Layout {
        groups: f
            .views()
            .map(|g| GroupInfo {
                members: (0..g.len())
                    .map(|i| MemberInfo {
                        path: g.path(i).to_path_buf(),
                        shown: rel_display(g.path(i), root),
                        size: g.size(i),
                        role: g.role(i),
                    })
                    .collect(),
                reclaim: g.reclaim(),
            })
            .collect(),
    }
}

/// `f`: every group with a member under `folder` (as listed) takes the
/// first such member for keeper, unless its keeper is under it already.
/// Returns (new keeper, old keeper) for each group changed, so the rule
/// can be taken back. (Phase 16, NF-10)
fn prefer_folder(f: &mut Found, root: &std::path::Path, folder: &str) -> Vec<(PathBuf, PathBuf)> {
    let under = |p: &std::path::Path| {
        let shown = rel_display(p, root);
        shown
            .strip_prefix(folder)
            .is_some_and(|rest| folder.is_empty() || rest.starts_with(['/', '\\']))
    };
    let mut changed = Vec::new();
    for n in 1..=f.group_count() {
        let Some(g) = f.view_mut(n - 1) else { continue };
        if under(g.path(0)) {
            continue;
        }
        if let Some(i) = (1..g.len()).find(|&i| under(g.path(i))) {
            let (new, old) = (g.path(i).to_path_buf(), g.path(0).to_path_buf());
            g.set_keeper(i);
            changed.push((new, old));
        }
    }
    changed
}

/// The preferred folder taken back: each group whose keeper `f` chose
/// gets its old keeper again, when both are still in it.
fn prefer_undo(f: &mut Found, changed: &[(PathBuf, PathBuf)]) {
    for n in 1..=f.group_count() {
        let Some(g) = f.view_mut(n - 1) else { continue };
        let keeper = g.path(0).to_path_buf();
        if let Some((_, old)) = changed.iter().find(|(new, _)| *new == keeper) {
            if let Some(i) = (1..g.len()).find(|&i| g.path(i) == old) {
                g.set_keeper(i);
            }
        }
    }
}

/// The listing's line for every row of the layout, rendered exactly as
/// `find --verbose` prints them (two cells narrower: the marks take them),
/// once per layout and width. The rows on screen pick from it at every
/// draw, so a filter or a sort shows its own groups — laid out once in the
/// full order, a filter to one group drew the first group's lines over
/// its rows (Phase 17; seen in the Windows run's view.png). Plain path
/// lines when the rendered listing does not line up with the rows.
struct Listing {
    by_row: std::collections::HashMap<Row, String>,
}

impl Listing {
    fn of(f: &Found, inv: &Invocation, r: &Renderer, st: &State, width_cells: usize) -> Listing {
        let captured = r.capture(width_cells.saturating_sub(2).max(40), |c| {
            c.listing(&f.groups, &f.near, &f.video, &inv.root);
        });
        let lines: Vec<String> = captured
            .into_iter()
            .filter(|l| !l.trim().is_empty())
            .collect();
        let all = state::all_rows(&st.layout);
        let lines_up = lines.len() == all.len()
            && all.iter().zip(&lines).all(|(row, l)| match row {
                Row::Head(_) => l.starts_with(' ') && !l.starts_with("  "),
                Row::Member(_, _) => l.starts_with("   "),
            });
        let by_row = if lines_up {
            all.into_iter().zip(lines).collect()
        } else {
            all.into_iter()
                .map(|row| {
                    let line = match row {
                        Row::Head(g) => format!(" GROUP {:03}", g + 1),
                        Row::Member(g, m) => {
                            let mi = &st.layout.groups[g].members[m];
                            let verb = if m == 0 { "keep" } else { "    " };
                            format!("   {verb}  {}", rel_display(&mi.path, &inv.root))
                        }
                    };
                    (row, line)
                })
                .collect()
        };
        Listing { by_row }
    }

    /// The lines of the rows shown, in the order shown.
    fn shown(&self, st: &State) -> Vec<String> {
        st.rows
            .iter()
            .map(|row| self.by_row.get(row).cloned().unwrap_or_default())
            .collect()
    }
}

/// The paths of `sel` as action items, by-hand ones marked.
fn chosen_items<'a>(f: &'a Found, sel: &HashSet<PathBuf>) -> Vec<actions::Item<'a>> {
    // the other shots a person chose with `c` come only from here
    actions::items(f)
        .into_iter()
        .chain(actions::shot_items(f))
        .filter(|it| sel.contains(it.path))
        .map(|mut it| {
            it.by_hand = !it.actionable();
            it
        })
        .collect()
}

// ---------------------------------------------------------------------------
// the terminal
// ---------------------------------------------------------------------------

/// A key, or a resize, from the terminal (raw mode held by the caller).
enum Input {
    Key(Key),
    Resize,
}

fn read_input() -> io::Result<Input> {
    loop {
        match event::read()? {
            Event::Resize(_, _) => return Ok(Input::Resize),
            Event::Key(k) if k.kind == KeyEventKind::Press => {
                let key = match k.code {
                    KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => Key::Quit,
                    KeyCode::Char(' ') => Key::Space,
                    KeyCode::Char(c) => Key::Char(c.to_ascii_lowercase()),
                    KeyCode::Up => Key::Up,
                    KeyCode::Down => Key::Down,
                    KeyCode::PageUp => Key::PageUp,
                    KeyCode::PageDown => Key::PageDown,
                    KeyCode::Home => Key::Home,
                    KeyCode::End => Key::End,
                    KeyCode::Enter => Key::Enter,
                    KeyCode::Esc => Key::Esc,
                    KeyCode::Backspace => Key::Backspace,
                    _ => continue,
                };
                return Ok(Input::Key(key));
            }
            _ => continue,
        }
    }
}

/// Columns and rows: the terminal's when it reports them (a pseudo-
/// terminal may report 0 × 0), else the theme's width and 24 rows.
fn term_size(r: &Renderer) -> (usize, usize) {
    let (c, rows) = terminal::size().unwrap_or((0, 0));
    let cols = if c == 0 {
        r.theme.width
    } else {
        (c as usize).min(r.theme.width.max(40))
    };
    let rows = if rows == 0 { 24 } else { rows as usize };
    (cols.max(40), rows.max(10))
}

fn enter_screen() -> io::Result<()> {
    terminal::enable_raw_mode()?;
    execute!(io::stdout(), terminal::EnterAlternateScreen, cursor::Hide)
}

fn leave_screen() {
    let _ = execute!(io::stdout(), cursor::Show, terminal::LeaveAlternateScreen);
    let _ = terminal::disable_raw_mode();
}

fn draw(lines: &[String]) -> io::Result<()> {
    let mut out = io::stdout().lock();
    queue!(out, cursor::MoveTo(0, 0))?;
    for (i, l) in lines.iter().enumerate() {
        queue!(out, cursor::MoveTo(0, i as u16))?;
        out.write_all(l.as_bytes())?;
        queue!(out, terminal::Clear(terminal::ClearType::UntilNewLine))?;
    }
    queue!(out, terminal::Clear(terminal::ClearType::FromCursorDown))?;
    out.flush()
}

/// A one-line bar on the main screen; waits for one key (raw mode only for
/// the wait) and erases the bar.
fn pause(r: &Renderer, parts: &[(&str, &str)]) -> Key {
    let t = &r.theme;
    let v: Vec<(String, String)> = parts
        .iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect();
    r.blank();
    print!("{}", screen::fit_parts(t, t.glyphs.pointer, &v, t.width));
    let _ = io::stdout().flush();
    let key = (|| -> io::Result<Key> {
        terminal::enable_raw_mode()?;
        let k = loop {
            if let Input::Key(k) = read_input()? {
                break k;
            }
        };
        terminal::disable_raw_mode()?;
        Ok(k)
    })()
    .unwrap_or(Key::Quit);
    let _ = execute!(
        io::stdout(),
        cursor::MoveToColumn(0),
        terminal::Clear(terminal::ClearType::CurrentLine)
    );
    key
}

/// A typed number on the main screen (the `--undo` picker): digits,
/// backspace, enter; enter alone is 1, esc or q is None.
pub(crate) fn pick_number(r: &Renderer, prompt: &str, max: usize) -> Option<usize> {
    let t = &r.theme;
    let mut typed = String::new();
    let result = (|| -> io::Result<Option<usize>> {
        terminal::enable_raw_mode()?;
        loop {
            let mut out = io::stdout();
            execute!(
                out,
                cursor::MoveToColumn(0),
                terminal::Clear(terminal::ClearType::CurrentLine)
            )?;
            print!(
                " {}  {}  {}",
                t.paint_bold(Paint::Keep, t.glyphs.pointer),
                t.paint_bold(Paint::Keep, &format!("{typed}_")),
                t.paint(Paint::Dupe, prompt)
            );
            out.flush()?;
            match read_input()? {
                Input::Key(Key::Char(c)) if c.is_ascii_digit() && typed.len() < 4 => typed.push(c),
                Input::Key(Key::Backspace) => {
                    typed.pop();
                }
                Input::Key(Key::Enter) => {
                    let n = if typed.is_empty() {
                        1
                    } else {
                        typed.parse().unwrap_or(0)
                    };
                    if (1..=max).contains(&n) {
                        return Ok(Some(n));
                    }
                    typed.clear();
                }
                Input::Key(Key::Esc | Key::Quit | Key::Char('q')) => return Ok(None),
                _ => {}
            }
        }
    })();
    let _ = terminal::disable_raw_mode();
    let _ = execute!(
        io::stdout(),
        cursor::MoveToColumn(0),
        terminal::Clear(terminal::ClearType::CurrentLine)
    );
    result.ok().flatten()
}

// ---------------------------------------------------------------------------
// the sitting
// ---------------------------------------------------------------------------

/// Run the workspace on `f`. Returns the exit code of the last action.
pub(crate) fn run(f: &mut Found, inv: &Invocation, r: &Renderer) -> i32 {
    let mut code = EXIT_OK;
    // (journal id, the groups before that change), newest last
    let mut undo_stack: Vec<(String, Found)> = Vec::new();
    let title = r
        .capture(r.theme.width, |c| c.header(inv))
        .into_iter()
        .next()
        .unwrap_or_default();
    let (mut cols, mut rows) = term_size(r);
    // the keepers `f` changed: (new, old), to take the rule back
    let mut preferred: Vec<(PathBuf, PathBuf)> = Vec::new();
    let mut st = State::new(layout_of(f, &inv.root), rows.saturating_sub(6));
    let mut lines = Listing::of(f, inv, r, &st, cols);
    let protocol = preview::detect();

    if enter_screen().is_err() {
        leave_screen();
        r.next_after_find(inv);
        return code;
    }
    loop {
        let mut t = r.theme;
        t.width = cols;
        let _ = draw(&st.screen(&t, &title, &lines.shown(&st), rows));
        let key = match read_input() {
            Ok(Input::Key(k)) => k,
            Ok(Input::Resize) => {
                (cols, rows) = term_size(r);
                st.set_height(rows.saturating_sub(6));
                lines = Listing::of(f, inv, r, &st, cols);
                continue;
            }
            Err(_) => Key::Quit,
        };
        match st.handle(key) {
            Effect::None => {}
            Effect::Quit => {
                leave_screen();
                if undo_stack.is_empty() {
                    // the typed forms, for the scrollback
                    r.next_after_find(inv);
                }
                return code;
            }
            Effect::Keeper(g, m) => {
                if f.set_keeper(g + 1, m) {
                    st.relayout(layout_of(f, &inv.root));
                    lines = Listing::of(f, inv, r, &st, cols);
                    st.message = Some("new keeper; the group re-related to it".into());
                }
            }
            Effect::Prefer(g, m) => {
                let shown = st.layout.groups[g].members[m].shown.clone();
                let folder = shown
                    .rfind(['/', '\\'])
                    .map_or(String::new(), |i| shown[..i].to_string());
                prefer_undo(f, &preferred);
                preferred = prefer_folder(f, &inv.root, &folder);
                st.relayout(layout_of(f, &inv.root));
                lines = Listing::of(f, inv, r, &st, cols);
                let place = if folder.is_empty() {
                    "."
                } else {
                    folder.as_str()
                };
                st.message = Some(format!(
                    "keeping in {place}: {} {} changed",
                    preferred.len(),
                    if preferred.len() == 1 {
                        "keeper"
                    } else {
                        "keepers"
                    }
                ));
                st.prefer = Some(folder);
            }
            Effect::PreferOff => {
                prefer_undo(f, &preferred);
                preferred.clear();
                st.prefer = None;
                st.relayout(layout_of(f, &inv.root));
                lines = Listing::of(f, inv, r, &st, cols);
                st.message = Some("no preferred folder; the keepers back".into());
            }
            Effect::Reveal(g, m) => {
                let path = st.layout.groups[g].members[m].path.clone();
                st.message = Some(match preview::reveal(&path) {
                    Ok(()) => format!("opening in {}", preview::file_manager()),
                    Err(e) => format!("could not open {}: {e}", preview::file_manager()),
                });
            }
            Effect::View(g, m) => {
                st.message = view(f, g, m, protocol, inv, r, cols, rows);
            }
            Effect::DryRun => {
                leave_screen();
                let mut dry = inv.clone();
                dry.dry_run = true;
                r.blank();
                r.note(Paint::Plate, &dry.command_line_as(Command::Milk));
                r.blank();
                let list = chosen_items(f, &st.selection());
                actions::act_milk(f, &list, inv, r, true);
                let k = pause(r, &[("enter", "back to the groups"), ("q", "quit")]);
                if matches!(k, Key::Char('q') | Key::Quit) {
                    if undo_stack.is_empty() {
                        r.next_after_find(inv);
                    }
                    return code;
                }
                if enter_screen().is_err() {
                    return code;
                }
            }
            Effect::Act(act) => {
                leave_screen();
                let sel: HashSet<PathBuf> = st.selection();
                let mut acted = act_on(act, f, &sel, inv, r, &mut undo_stack);
                let mut last = act;
                code = acted.code;
                // the bar under the action: back, a retry or a fallback for
                // what failed (by kind), undo, quit — as often as needed
                loop {
                    st.changes = undo_stack.len();
                    st.relayout(layout_of(f, &inv.root));
                    lines = Listing::of(f, inv, r, &st, cols);
                    match after_change(r, f, &acted, last) {
                        After::Back => {
                            if enter_screen().is_err() {
                                return code;
                            }
                            break;
                        }
                        After::Retry => {
                            let paths = failed_paths(&acted, ActError::retryable);
                            acted = act_on(last, f, &paths, inv, r, &mut undo_stack);
                            code = acted.code;
                        }
                        After::Fallback => {
                            let paths = failed_paths(&acted, |e| matches!(e, ActError::Bin(_)));
                            last = Act::Milk;
                            acted = act_on(Act::Milk, f, &paths, inv, r, &mut undo_stack);
                            code = acted.code;
                        }
                        After::Undo => {
                            code = undo_last(&mut undo_stack, f, inv, r);
                            st.changes = undo_stack.len();
                            st.relayout(layout_of(f, &inv.root));
                            lines = Listing::of(f, inv, r, &st, cols);
                            if !back_or_quit(r, f) || enter_screen().is_err() {
                                return code;
                            }
                            break;
                        }
                        After::Quit => return code,
                    }
                }
            }
            Effect::Undo => {
                leave_screen();
                code = undo_last(&mut undo_stack, f, inv, r);
                st.changes = undo_stack.len();
                st.relayout(layout_of(f, &inv.root));
                lines = Listing::of(f, inv, r, &st, cols);
                if !back_or_quit(r, f) || enter_screen().is_err() {
                    return code;
                }
            }
        }
        if f.empty() && undo_stack.is_empty() {
            leave_screen();
            return code;
        }
    }
}

enum After {
    Back,
    /// Act again on the files that were in use or refused.
    Retry,
    /// Move into `_milked` the files the Recycle Bin refused.
    Fallback,
    Undo,
    Quit,
}

/// One change on the main screen: the native command, the action on the
/// chosen files, its journal on the undo stack with the groups from
/// before it, then the groups pruned of what it moved.
fn act_on(
    act: Act,
    f: &mut Found,
    paths: &HashSet<PathBuf>,
    inv: &Invocation,
    r: &Renderer,
    stack: &mut Vec<(String, Found)>,
) -> actions::Acted {
    let before = f.clone();
    let list = chosen_items(f, paths);
    r.blank();
    let acted = match act {
        Act::Milk => {
            r.note(Paint::Plate, &inv.command_line_as(Command::Milk));
            r.blank();
            actions::act_milk(f, &list, inv, r, false)
        }
        Act::Purge => {
            r.note(Paint::Plate, &inv.command_line_as(Command::Purge));
            r.blank();
            let verified = actions::verify_items(&list, &f.pipe, r);
            actions::act_purge(f, &list, &verified, inv, r, false)
        }
    };
    drop(list);
    if let Some(id) = &acted.journal {
        stack.push((id.clone(), before));
    }
    f.prune_missing();
    acted
}

fn failed_paths(acted: &actions::Acted, keep: impl Fn(&ActError) -> bool) -> HashSet<PathBuf> {
    acted
        .failed
        .iter()
        .filter(|(_, e)| keep(e))
        .map(|(p, _)| p.clone())
        .collect()
}

/// The bar under an action's output: back to what is left; try again the
/// files that were in use; move into `_milked` what the bin refused; undo
/// it (when it changed anything); leave.
fn after_change(r: &Renderer, f: &Found, acted: &actions::Acted, act: Act) -> After {
    let left = f.group_count();
    let retry_n = acted.failed.iter().filter(|(_, e)| e.retryable()).count();
    let bin_n = acted
        .failed
        .iter()
        .filter(|(_, e)| matches!(e, ActError::Bin(_)))
        .count();
    let back = format!("back to the groups ({} left)", fmt_count(left as u64));
    let retry = format!(
        "retry {} (close what holds them first)",
        fmt_count(retry_n as u64)
    );
    let fallback = format!(
        "milk the {} the bin refused into {}",
        fmt_count(bin_n as u64),
        crate::MILKED_DIR
    );
    let mut parts: Vec<(&str, &str)> = Vec::new();
    if left > 0 {
        parts.push(("enter", &back));
    }
    if retry_n > 0 {
        parts.push(("r", &retry));
    }
    if bin_n > 0 && act == Act::Purge {
        parts.push(("m", &fallback));
    }
    if acted.journal.is_some() {
        parts.push(("u", "undo it"));
    }
    parts.push(("q", "quit"));
    loop {
        match pause(r, &parts) {
            Key::Enter if left > 0 => return After::Back,
            Key::Char('r') if retry_n > 0 => return After::Retry,
            Key::Char('m') if bin_n > 0 && act == Act::Purge => return After::Fallback,
            Key::Char('u') if acted.journal.is_some() => return After::Undo,
            Key::Char('q') | Key::Esc | Key::Quit => return After::Quit,
            _ => {}
        }
    }
}

/// After an undo on the main screen: true to go back to the groups.
fn back_or_quit(r: &Renderer, f: &Found) -> bool {
    if f.group_count() == 0 {
        return false;
    }
    loop {
        match pause(r, &[("enter", "back to the groups"), ("q", "quit")]) {
            Key::Enter => return true,
            Key::Char('q') | Key::Esc | Key::Quit => return false,
            _ => {}
        }
    }
}

/// Reverse the newest change of the sitting and put its groups back.
fn undo_last(
    stack: &mut Vec<(String, Found)>,
    f: &mut Found,
    inv: &Invocation,
    r: &Renderer,
) -> i32 {
    let Some((id, before)) = stack.pop() else {
        return EXIT_OK;
    };
    r.blank();
    r.note(Paint::Plate, &inv.command_line_as(Command::Undo));
    r.blank();
    let code = actions::act_undo_which(inv, r, Some(&id));
    *f = before;
    // anything the undo could not restore is not offered again
    f.prune_missing();
    code
}

/// `v`: the keeper beside the member, drawn inline when the terminal has a
/// graphics protocol, opened in the system viewer when it has none.
/// Returns the status note.
#[allow(clippy::too_many_arguments)]
fn view(
    f: &Found,
    g: usize,
    m: usize,
    protocol: Protocol,
    inv: &Invocation,
    r: &Renderer,
    cols: usize,
    rows: usize,
) -> Option<String> {
    let Some((keeper, member, words)) = visuals(f, g, m) else {
        return Some("nothing to look at: not a picture or a video".into());
    };
    if protocol == Protocol::None {
        let a = preview::open(keeper.path());
        let b = preview::open(member.path());
        return Some(match (a, b) {
            (Ok(()), Ok(())) => {
                "opening both in the system viewer (no inline graphics here)".into()
            }
            (Err(e), _) | (_, Err(e)) => format!("could not open the viewer: {e}"),
        });
    }
    let t = &r.theme;
    let half = cols / 2;
    let img_cols = half.saturating_sub(2).max(10);
    let img_rows = rows.saturating_sub(7).max(4);
    let (cw, ch) = preview::cell_pixels();
    let (max_w, max_h) = (img_cols as u32 * cw, img_rows as u32 * ch);
    let mut out = io::stdout();
    let _ = execute!(
        out,
        terminal::Clear(terminal::ClearType::All),
        cursor::MoveTo(0, 0)
    );
    let head = format!(
        " {}   {}",
        t.paint_bold(Paint::Plate, &format!("VIEW  group {}", g + 1)),
        t.paint(
            Paint::Dupe,
            &format!("keeper | member   {}", protocol.name())
        )
    );
    let _ = preview::emit(&head);
    for (col, v) in [(1usize, &keeper), (half + 1, &member)] {
        let _ = execute!(out, cursor::MoveTo(col as u16, 2));
        match v.load() {
            Some(img) => {
                let small = preview::fit_pixels(&img, max_w, max_h);
                let _ = preview::emit(&preview::encode(protocol, &small));
            }
            None => {
                let _ = preview::emit(&t.paint(Paint::Warn, "could not decode"));
            }
        }
    }
    // captions under the pictures
    let cap_row = (rows.saturating_sub(4)) as u16;
    let caption = |v: &Visual, label: &str| -> String {
        let rel = rel_display(v.path(), &inv.root);
        format!(
            "{}  {}",
            t.paint_bold(Paint::Keep, label),
            t.paint(
                Paint::Plate,
                &shorten_middle(&rel, img_cols.saturating_sub(6), t.glyphs.ellipsis)
            )
        )
    };
    let _ = execute!(out, cursor::MoveTo(1, cap_row));
    let _ = preview::emit(&caption(&keeper, "keep"));
    let _ = execute!(out, cursor::MoveTo((half + 1) as u16, cap_row));
    let _ = preview::emit(&caption(&member, "near"));
    let _ = execute!(out, cursor::MoveTo((half + 1) as u16, cap_row + 1));
    let _ = preview::emit(&t.paint(
        Paint::Dupe,
        &shorten_middle(&words, img_cols, t.glyphs.ellipsis),
    ));
    let _ = execute!(out, cursor::MoveTo(0, (rows - 1) as u16));
    let bar = screen::fit_parts(
        t,
        t.glyphs.pointer,
        &[
            ("any key".into(), "back".into()),
            ("o".into(), "reveal the member".into()),
        ],
        cols,
    );
    let _ = preview::emit(&bar);
    let key = loop {
        match read_input() {
            Ok(Input::Key(k)) => break k,
            Ok(Input::Resize) => continue,
            Err(_) => break Key::Esc,
        }
    };
    if protocol == Protocol::Kitty {
        // kitty keeps placed images across a clear; delete them
        let _ = preview::emit("\x1b_Ga=d\x1b\\");
    }
    let _ = execute!(out, terminal::Clear(terminal::ClearType::All));
    if key == Key::Char('o') {
        return Some(match preview::reveal(member.path()) {
            Ok(()) => format!("opening in {}", preview::file_manager()),
            Err(e) => format!("could not open {}: {e}", preview::file_manager()),
        });
    }
    None
}

/// The keeper and member `m` of on-screen group `g` as things to look at,
/// with the member's words.
fn visuals(f: &Found, g: usize, m: usize) -> Option<(Visual, Visual, String)> {
    let grp = f.view(g)?;
    if m >= grp.len() {
        return None;
    }
    Some((grp.visual(0)?, grp.visual(m)?, grp.words(m)))
}
