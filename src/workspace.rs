//! The result is the workspace (docs/CLI.md §2, Phase 8). After `find`
//! lists its groups on a terminal, a one-line bar offers the decisions a
//! person makes at that point — move these, bin these, preview, change a
//! keeper, undo, quit — and acts on the groups already on screen. Nothing
//! is scanned twice. On quit the native commands are printed, so every
//! session still teaches the typed form. [CLAUDE, under the brief's FUOE
//! rule: the operational experience is the design]
//!
//! Raw mode is held only while a key is awaited; everything printed goes
//! through the ordinary renderer between keys, so the listing above the
//! bar is the same listing a pipe would see.

use std::io::{self, Write};

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::{cursor, execute, terminal};

use crate::actions::{self, Found, EXIT_OK};
use crate::cli::{Command, Invocation};
use crate::render::Renderer;
use crate::theme::Role;
use crate::util::{fmt_bytes, fmt_count, rel_display};
use crate::MILKED_DIR;

/// One key from the terminal, raw mode held only for the wait.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Key {
    Char(char),
    Enter,
    Esc,
    Backspace,
    Up,
    Down,
    Quit, // Ctrl-C
}

fn read_key() -> io::Result<Key> {
    terminal::enable_raw_mode()?;
    let k = loop {
        let ev = event::read()?;
        let Event::Key(k) = ev else { continue };
        if k.kind != KeyEventKind::Press {
            continue;
        }
        break match k.code {
            KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => Key::Quit,
            KeyCode::Char(c) => Key::Char(c.to_ascii_lowercase()),
            KeyCode::Enter => Key::Enter,
            KeyCode::Esc => Key::Esc,
            KeyCode::Backspace => Key::Backspace,
            KeyCode::Up => Key::Up,
            KeyCode::Down => Key::Down,
            _ => continue,
        };
    };
    terminal::disable_raw_mode()?;
    Ok(k)
}

/// Erase the bar line the cursor sits on, so what comes next replaces it.
fn clear_bar() {
    let mut out = io::stdout();
    let _ = execute!(
        out,
        cursor::MoveToColumn(0),
        terminal::Clear(terminal::ClearType::CurrentLine)
    );
    let _ = out.flush();
}

/// Print a bar without a newline and leave the cursor on it. The bar is
/// one line: when the parts do not fit the width, each part's text is cut
/// at its first two words (`milk 12 → _milked` becomes `milk 12`), then
/// at its first.
fn bar(r: &Renderer, parts: &[(&str, String)]) {
    let t = &r.theme;
    let render = |take: usize| -> (String, String) {
        let mut plain = format!(" {}", t.glyphs.pointer);
        let mut painted = format!(" {}", t.paint_bold(Role::Keep, t.glyphs.pointer));
        for (key, text) in parts {
            let text: String = if take == 0 {
                text.clone()
            } else {
                text.split_whitespace()
                    .take(take)
                    .collect::<Vec<_>>()
                    .join(" ")
            };
            plain.push_str(&format!("  {key} {text}"));
            painted.push_str(&format!(
                "  {} {}",
                t.paint_bold(Role::Keep, key),
                t.paint(Role::Plate, &text)
            ));
        }
        (plain, painted)
    };
    let mut chosen = render(0);
    for take in [2usize, 1] {
        if crate::util::width(&chosen.0) < t.width {
            break;
        }
        chosen = render(take);
    }
    print!("{}", chosen.1);
    let _ = io::stdout().flush();
}

/// A typed number on the bar: digits, backspace, enter; esc gives None.
fn read_number(r: &Renderer, prompt: &str) -> Option<usize> {
    let t = &r.theme;
    let mut typed = String::new();
    loop {
        clear_bar();
        print!(
            " {}  {} {}",
            t.paint_bold(Role::Keep, t.glyphs.pointer),
            t.paint(Role::Plate, prompt),
            t.paint_bold(Role::Keep, &typed)
        );
        let _ = io::stdout().flush();
        match read_key().ok()? {
            Key::Char(c) if c.is_ascii_digit() && typed.len() < 6 => typed.push(c),
            Key::Backspace => {
                typed.pop();
            }
            Key::Enter => return typed.parse().ok(),
            Key::Esc | Key::Quit => return None,
            _ => {}
        }
    }
}

/// What the bar offers; counts are of the files that would move or bin.
fn actionable(f: &Found) -> (u64, u64) {
    let mut n = 0u64;
    let mut bytes = 0u64;
    for it in actions::items(f) {
        if it.actionable() {
            n += 1;
            bytes += it.size;
        }
    }
    (n, bytes)
}

/// Run the workspace on `f`. Returns the exit code of the last action.
pub(crate) fn run(f: &mut Found, inv: &Invocation, r: &Renderer) -> i32 {
    let mut code = EXIT_OK;
    let mut last_journal: Option<String> = None;
    let mut acted = false;
    loop {
        let (n, bytes) = actionable(f);
        let mut parts: Vec<(&str, String)> = Vec::new();
        if !acted && n > 0 {
            parts.push((
                "m",
                format!(
                    "milk {} {} {MILKED_DIR}",
                    fmt_count(n),
                    r.theme.glyphs.arrow
                ),
            ));
            parts.push((
                "p",
                format!(
                    "purge {} {} Recycle Bin",
                    fmt_count(n),
                    r.theme.glyphs.arrow
                ),
            ));
            parts.push(("n", "preview".to_string()));
            parts.push(("k", "keeper".to_string()));
        }
        if last_journal.is_some() {
            parts.push(("u", "undo".to_string()));
        }
        parts.push(("q", "quit".to_string()));
        r.blank();
        bar(r, &parts);
        let key = match read_key() {
            Ok(k) => k,
            Err(_) => Key::Quit,
        };
        match key {
            Key::Char('m') if !acted && n > 0 => {
                clear_bar();
                bar(
                    r,
                    &[
                        (
                            "milk",
                            format!(
                                "{} {} {MILKED_DIR}, {}",
                                fmt_count(n),
                                r.theme.glyphs.arrow,
                                fmt_bytes(bytes)
                            ),
                        ),
                        ("enter", "go".to_string()),
                        ("esc", "back".to_string()),
                    ],
                );
                if matches!(read_key(), Ok(Key::Enter)) {
                    clear_bar();
                    r.note(Role::Plate, &inv.command_line_as(Command::Milk));
                    r.blank();
                    let (c, j) = actions::act_milk(f, inv, r, false);
                    code = c;
                    last_journal = j;
                    acted = true;
                } else {
                    clear_bar();
                }
            }
            Key::Char('p') if !acted && n > 0 => {
                clear_bar();
                bar(
                    r,
                    &[
                        (
                            "purge",
                            format!(
                                "{} {} Recycle Bin, {}",
                                fmt_count(n),
                                r.theme.glyphs.arrow,
                                fmt_bytes(bytes)
                            ),
                        ),
                        ("enter", "go".to_string()),
                        ("esc", "back".to_string()),
                    ],
                );
                if matches!(read_key(), Ok(Key::Enter)) {
                    clear_bar();
                    r.note(Role::Plate, &inv.command_line_as(Command::Purge));
                    r.blank();
                    let list = actions::items(f);
                    let verified = actions::verify_items(&list, &f.pipe, r);
                    let (c, j) = actions::act_purge(f, &list, &verified, inv, r, false);
                    code = c;
                    last_journal = j;
                    acted = true;
                } else {
                    clear_bar();
                }
            }
            Key::Char('n') if !acted && n > 0 => {
                clear_bar();
                let mut dry = inv.clone();
                dry.dry_run = true;
                r.note(Role::Plate, &dry.command_line_as(Command::Milk));
                r.blank();
                actions::act_milk(f, inv, r, true);
            }
            Key::Char('k') if !acted && n > 0 => {
                change_keeper(f, inv, r);
            }
            Key::Char('u') if last_journal.is_some() => {
                clear_bar();
                r.note(Role::Plate, &inv.command_line_as(Command::Undo));
                r.blank();
                code = actions::act_undo(inv, r);
                last_journal = None;
            }
            Key::Char('q') | Key::Esc | Key::Quit => {
                clear_bar();
                if !acted {
                    // the typed forms, for the scrollback
                    r.next_after_find(inv);
                }
                return code;
            }
            _ => {
                clear_bar();
            }
        }
    }
}

/// `k`: pick a group by its on-screen number, then a member by position;
/// the group is reprinted with the new keeper and the new relations.
fn change_keeper(f: &mut Found, inv: &Invocation, r: &Renderer) {
    let t = &r.theme;
    let total = f.group_count();
    let Some(n) = read_number(r, &format!("keeper of group (1-{total})")) else {
        clear_bar();
        return;
    };
    let paths = f.group_paths(n);
    if paths.is_empty() {
        clear_bar();
        r.note(Role::Warn, &format!("no group {n}"));
        return;
    }
    clear_bar();
    r.blank();
    for (i, p) in paths.iter().enumerate() {
        let rel = rel_display(p, &inv.root);
        let mark = if i == 0 { "keep" } else { "    " };
        r.line(&format!(
            "   {}  {}  {}",
            t.paint_bold(Role::Keep, &format!("{:>2}", i + 1)),
            t.paint(if i == 0 { Role::Keep } else { Role::Dupe }, mark),
            t.paint(Role::Plate, &rel)
        ));
    }
    let Some(m) = read_number(r, "new keeper") else {
        clear_bar();
        return;
    };
    clear_bar();
    if m < 2 || m > paths.len() {
        r.note(Role::Warn, &format!("no member {m} in group {n}"));
        return;
    }
    if !f.set_keeper(n, m - 1) {
        r.note(Role::Warn, "keeper unchanged");
        return;
    }
    // the group, as the listing prints it, with its new keeper
    let mut i = n - 1;
    if i < f.groups.len() {
        r.groups_from(&f.groups[i..i + 1], &inv.root, i);
        return;
    }
    i -= f.groups.len();
    if i < f.near.len() {
        r.near_groups(&f.near[i..i + 1], &inv.root, f.groups.len() + i);
        return;
    }
    i -= f.near.len();
    r.video_groups(
        &f.video[i..i + 1],
        &inv.root,
        f.groups.len() + f.near.len() + i,
    );
}
