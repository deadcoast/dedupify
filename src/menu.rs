//! The menu (docs/CLI.md §4): `dedupe` alone. Two pipelines and a
//! folder, nothing else asked [the owner's direction of 2026-11-14: pick
//! the pipeline, give the path; anything more is what the in-depth output
//! is for]. `batch` takes the whole tree below the folder and deals with
//! it (§2b); `review` lists every group and opens the workspace (§2a).
//! `again`, `undo` and `doctor` sit under them. The state machine is pure
//! and tested; `run` wraps it in raw mode and returns the native argv,
//! which `main` parses and runs like any typed command.

use std::io::{self, Write};
use std::path::Path;

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::{cursor, execute, terminal};

use crate::render::fold::{DIM, PLATE};
use crate::theme::{Rgb, Role, Theme};
use crate::util::{fit, shorten_middle, visible_width, width};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pipeline {
    Batch,
    Review,
    Undo,
    Doctor,
}

/// The pipelines, then the tools: name, what it is, what it does.
pub const PIPELINES: [(Pipeline, &str, &str, &str); 4] = [
    (
        Pipeline::Batch,
        "batch",
        "a whole folder tree, done for you",
        "copies and confirmed lookalikes into _milked · said by folder · one key undoes it",
    ),
    (
        Pipeline::Review,
        "review",
        "every group side by side; you choose",
        "pictures, evidence and keys in the workspace",
    ),
    (Pipeline::Undo, "undo", "reverse the last change", ""),
    (
        Pipeline::Doctor,
        "doctor",
        "check this terminal, folder and tools",
        "",
    ),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Pick,
    Folder,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    Enter,
    Esc,
    Backspace,
    Char(char),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Continue,
    Quit,
    Run(Vec<String>),
}

/// The rows of the first step, in order on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Row {
    Pipe(usize),
    Again,
}

#[derive(Debug, Clone)]
pub struct Menu {
    pub step: Step,
    row: usize,
    /// The folder as typed; empty means the current one, shown dim until
    /// the first key replaces it.
    pub folder: String,
    pub folder_error: Option<String>,
    /// Where the user is; left out of the command when it is the folder.
    cwd: String,
    /// The last folder looked at, offered under the pipelines (`again`).
    again: Option<crate::recent::Recent>,
}

impl Menu {
    pub fn new(cwd: &str) -> Menu {
        Menu::with_recent(cwd, None)
    }

    /// The menu with the last run offered under the pipelines (QOL-03).
    pub fn with_recent(cwd: &str, again: Option<crate::recent::Recent>) -> Menu {
        Menu {
            step: Step::Pick,
            row: 0,
            folder: String::new(),
            folder_error: None,
            cwd: cwd.to_string(),
            again,
        }
    }

    /// batch, review, [again], undo, doctor
    fn rows(&self) -> Vec<Row> {
        let mut v = vec![Row::Pipe(0), Row::Pipe(1)];
        if self.again.is_some() {
            v.push(Row::Again);
        }
        v.push(Row::Pipe(2));
        v.push(Row::Pipe(3));
        v
    }

    fn current(&self) -> Row {
        let rows = self.rows();
        rows[self.row.min(rows.len() - 1)]
    }

    pub fn selected(&self) -> Option<Pipeline> {
        match self.current() {
            Row::Pipe(i) => Some(PIPELINES[i].0),
            Row::Again => None,
        }
    }

    /// The folder to run on: as typed (a pasted or dropped path loses its
    /// quotes), or the current one.
    fn folder_given(&self) -> String {
        let f = self
            .folder
            .trim()
            .trim_matches('"')
            .trim_matches('\'')
            .trim();
        if f.is_empty() {
            self.cwd.clone()
        } else {
            f.to_string()
        }
    }

    /// argv without the program name. The folder is passed only when it
    /// is not the current directory, so the composed command stays as
    /// short as the typed form.
    pub fn args(&self) -> Vec<String> {
        let mut v: Vec<String> = Vec::new();
        let Some(p) = self.selected() else {
            return self.again.as_ref().map(|a| a.args()).unwrap_or_default();
        };
        v.push(
            match p {
                Pipeline::Batch => "batch",
                Pipeline::Review => "find",
                Pipeline::Undo => "--undo",
                Pipeline::Doctor => "doctor",
            }
            .into(),
        );
        let folder = self.folder_given();
        if folder != self.cwd {
            v.push(folder);
        }
        // review is the whole tree too
        if p == Pipeline::Review {
            v.push(".".into());
        }
        v
    }

    pub fn command_line(&self) -> String {
        let mut parts = vec!["dedupe".to_string()];
        for a in self.args() {
            parts.push(if a.contains(' ') {
                format!("\"{a}\"")
            } else {
                a
            });
        }
        parts.join(" ")
    }

    pub fn handle(&mut self, key: Key) -> Outcome {
        let n = self.rows().len();
        match self.step {
            Step::Pick => match key {
                Key::Up => self.row = (self.row + n - 1) % n,
                Key::Down => self.row = (self.row + 1) % n,
                Key::Enter if self.current() == Row::Again => {
                    return Outcome::Run(self.args());
                }
                Key::Enter => {
                    self.step = Step::Folder;
                    self.folder_error = None;
                }
                Key::Esc | Key::Char('q') => return Outcome::Quit,
                _ => {}
            },
            Step::Folder => match key {
                Key::Char(c) => {
                    self.folder.push(c);
                    self.folder_error = None;
                }
                Key::Backspace => {
                    self.folder.pop();
                    self.folder_error = None;
                }
                Key::Enter => {
                    let f = self.folder_given();
                    if !Path::new(&f).is_dir() {
                        self.folder_error = Some(format!("not a folder  {f}"));
                    } else {
                        return Outcome::Run(self.args());
                    }
                }
                Key::Esc => {
                    self.step = Step::Pick;
                    self.folder.clear();
                    self.folder_error = None;
                }
                _ => {}
            },
        }
        Outcome::Continue
    }

    /// The screen as lines, every one inside `t.width`.
    pub fn view(&self, t: &Theme) -> Vec<String> {
        let g = t.glyphs;
        let w = t.width.saturating_sub(1).min(100);
        let amber = Rgb(0xFF, 0xB0, 0x00);
        let mut out: Vec<String> = Vec::new();
        let plate = if t.color {
            "\x1b[1;38;2;24;22;18;48;2;255;176;0m dedupe \x1b[0m".to_string()
        } else {
            "dedupe".to_string()
        };
        // the first line: the name on its plate, the step, the build
        let what = match (self.step, self.selected()) {
            (Step::Pick, _) => t.paint_rgb(DIM, "the deduplifier"),
            (Step::Folder, Some(p)) => t.paint_rgb_bold(
                amber,
                PIPELINES.iter().find(|x| x.0 == p).map_or("", |x| x.1),
            ),
            (Step::Folder, None) => String::new(),
        };
        let left = format!(" {plate}  {what}");
        let id = crate::build_id();
        let gap = w.saturating_sub(visible_width(&left) + width(&id)).max(2);
        out.push(if visible_width(&left) + gap + width(&id) <= w {
            format!("{left}{}{}", " ".repeat(gap), t.paint_rgb(DIM, &id))
        } else {
            left
        });
        out.push(String::new());
        let room = w.saturating_sub(16);
        match self.step {
            Step::Pick => {
                let rows = self.rows();
                for (i, r) in rows.iter().enumerate() {
                    let sel = i == self.row.min(rows.len() - 1);
                    let ptr = if sel {
                        t.paint_rgb_bold(amber, g.pointer)
                    } else {
                        " ".into()
                    };
                    let (name, line1, line2) = match r {
                        Row::Pipe(k) => (
                            PIPELINES[*k].1,
                            PIPELINES[*k].2.to_string(),
                            PIPELINES[*k].3,
                        ),
                        Row::Again => (
                            "again",
                            self.again.as_ref().map(|a| a.label()).unwrap_or_default(),
                            "",
                        ),
                    };
                    let big = matches!(r, Row::Pipe(0) | Row::Pipe(1));
                    let label = if big {
                        name.to_uppercase()
                    } else {
                        name.to_string()
                    };
                    let label = fit(&label, 8);
                    let label = if sel {
                        t.paint_rgb_bold(amber, &label)
                    } else if big {
                        t.paint_rgb_bold(PLATE, &label)
                    } else {
                        t.paint_rgb(PLATE, &label)
                    };
                    let line1 = shorten_middle(&line1, room, g.ellipsis);
                    out.push(format!(
                        "  {ptr} {label}  {}",
                        if sel || big {
                            t.paint_rgb(PLATE, &line1)
                        } else {
                            t.paint_rgb(DIM, &line1)
                        }
                    ));
                    if !line2.is_empty() {
                        out.push(format!(
                            "              {}",
                            t.paint_rgb(DIM, &fit(line2, room.min(width(line2))))
                        ));
                    }
                    if big {
                        out.push(String::new());
                    }
                }
                out.push(String::new());
                let keys: Vec<(String, String)> = vec![
                    (format!("{} {}", g.up, g.down), "choose".into()),
                    ("enter".into(), "next".into()),
                    ("q".into(), "quit".into()),
                ];
                out.push(format!(
                    " {}",
                    crate::workspace::fit_keys(t, &keys, w.saturating_sub(1))
                ));
            }
            Step::Folder => {
                out.push(format!("  {}", t.paint_rgb_bold(PLATE, "FOLDER")));
                let typed = self.folder.is_empty();
                let shown_text = if typed {
                    self.cwd.clone()
                } else {
                    self.folder.clone()
                };
                let shown_text = shorten_middle(&shown_text, room, g.ellipsis);
                out.push(format!(
                    "  {} {}{}",
                    t.paint_rgb_bold(amber, g.pointer),
                    if typed {
                        t.paint_rgb(DIM, &shown_text)
                    } else {
                        t.paint_rgb_bold(PLATE, &shown_text)
                    },
                    t.paint_rgb(amber, "_")
                ));
                out.push(format!(
                    "    {}",
                    t.paint_rgb(
                        DIM,
                        &fit(
                            if typed {
                                "this folder, or type, paste or drop another"
                            } else {
                                "type, paste or drop a folder"
                            },
                            room
                        )
                    )
                ));
                if let Some(e) = &self.folder_error {
                    out.push(format!("    {}", t.paint(Role::Danger, &fit(e, room))));
                } else {
                    out.push(String::new());
                }
                out.push(String::new());
                let go = match self.selected() {
                    Some(Pipeline::Batch) | Some(Pipeline::Review) => "run",
                    Some(Pipeline::Undo) => "undo",
                    Some(Pipeline::Doctor) => "check",
                    None => "run",
                };
                let keys: Vec<(String, String)> =
                    vec![("enter".into(), go.into()), ("esc".into(), "back".into())];
                out.push(format!(
                    " {}",
                    crate::workspace::fit_keys(t, &keys, w.saturating_sub(1))
                ));
            }
        }
        out
    }
}

/// Run the menu in raw mode. Returns the argv to execute, or `None` on quit.
pub fn run(theme: Theme, cwd: &str) -> io::Result<Option<Vec<String>>> {
    let mut menu = Menu::with_recent(cwd, crate::recent::load());
    let mut out = io::stdout();
    terminal::enable_raw_mode()?;
    execute!(out, terminal::EnterAlternateScreen, cursor::Hide)?;
    let result = (|| -> io::Result<Option<Vec<String>>> {
        loop {
            execute!(
                out,
                terminal::Clear(terminal::ClearType::All),
                cursor::MoveTo(0, 0)
            )?;
            let lines = menu.view(&theme);
            let mut buf = String::new();
            buf.push_str("\r\n");
            for l in lines {
                buf.push_str(&l);
                buf.push_str("\r\n");
            }
            out.write_all(buf.as_bytes())?;
            out.flush()?;
            let ev = event::read()?;
            let Event::Key(k) = ev else { continue };
            if k.kind != KeyEventKind::Press {
                continue;
            }
            let key = match k.code {
                KeyCode::Up => Key::Up,
                KeyCode::Down => Key::Down,
                KeyCode::Enter => Key::Enter,
                KeyCode::Esc => Key::Esc,
                KeyCode::Backspace => Key::Backspace,
                KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                    return Ok(None)
                }
                KeyCode::Char(c) => Key::Char(c),
                KeyCode::Tab => Key::Down,
                KeyCode::BackTab => Key::Up,
                _ => continue,
            };
            match menu.handle(key) {
                Outcome::Continue => {}
                Outcome::Quit => return Ok(None),
                Outcome::Run(args) => return Ok(Some(args)),
            }
        }
    })();
    let _ = execute!(out, cursor::Show, terminal::LeaveAlternateScreen);
    let _ = terminal::disable_raw_mode();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(m: &mut Menu, keys: &[Key]) -> Outcome {
        let mut last = Outcome::Continue;
        for k in keys {
            last = m.handle(*k);
        }
        last
    }

    fn typed(s: &str) -> Vec<Key> {
        s.chars().map(Key::Char).collect()
    }

    #[test]
    fn batch_is_first_and_two_enters_run_it_here() {
        let d = tempfile::tempdir().unwrap();
        let cwd = d.path().to_str().unwrap();
        let mut m = Menu::new(cwd);
        assert_eq!(m.selected(), Some(Pipeline::Batch));
        assert_eq!(m.handle(Key::Enter), Outcome::Continue);
        assert_eq!(m.step, Step::Folder);
        assert_eq!(m.handle(Key::Enter), Outcome::Run(vec!["batch".into()]));
    }

    #[test]
    fn a_folder_typed_pasted_or_dropped_is_the_one_run_on() {
        let d = tempfile::tempdir().unwrap();
        let photos = tempfile::tempdir().unwrap();
        let p = photos.path().to_str().unwrap().to_string();
        let mut m = Menu::new(d.path().to_str().unwrap());
        m.handle(Key::Enter);
        // a dropped folder arrives quoted
        let mut keys = typed(&format!("\"{p}\""));
        keys.push(Key::Enter);
        assert_eq!(
            press(&mut m, &keys),
            Outcome::Run(vec!["batch".into(), p.clone()])
        );
        // review: the same folder, the whole tree, the workspace after
        let mut m = Menu::new(d.path().to_str().unwrap());
        let mut keys = vec![Key::Down, Key::Enter];
        keys.extend(typed(&p));
        keys.push(Key::Enter);
        assert_eq!(
            press(&mut m, &keys),
            Outcome::Run(vec!["find".into(), p, ".".into()])
        );
    }

    #[test]
    fn a_folder_that_is_not_one_stays_and_says_so() {
        let d = tempfile::tempdir().unwrap();
        let mut m = Menu::new(d.path().to_str().unwrap());
        m.handle(Key::Enter);
        let mut keys = typed("/no/such/folder/here");
        keys.push(Key::Enter);
        assert_eq!(press(&mut m, &keys), Outcome::Continue);
        assert_eq!(m.step, Step::Folder);
        assert!(m
            .folder_error
            .as_deref()
            .unwrap_or("")
            .starts_with("not a folder"));
        // esc goes back to the pipelines with nothing typed
        m.handle(Key::Esc);
        assert_eq!(m.step, Step::Pick);
        assert!(m.folder.is_empty());
        assert_eq!(m.handle(Key::Esc), Outcome::Quit);
    }

    #[test]
    fn again_sits_under_the_pipelines_and_runs_at_once() {
        let d = tempfile::tempdir().unwrap();
        let cwd = d.path().to_str().unwrap();
        let photos = tempfile::tempdir().unwrap();
        let again = crate::recent::Recent {
            root: photos.path().to_path_buf(),
            recurse: true,
            kind: String::new(),
            min: None,
            batch: true,
        };
        let mut m = Menu::with_recent(cwd, Some(again.clone()));
        assert_eq!(m.selected(), Some(Pipeline::Batch));
        let out = press(&mut m, &[Key::Down, Key::Down, Key::Enter]);
        assert_eq!(out, Outcome::Run(again.args()));
        assert_eq!(again.args()[0], "batch");
        // without one, the third row is undo
        let mut m = Menu::new(cwd);
        press(&mut m, &[Key::Down, Key::Down]);
        assert_eq!(m.selected(), Some(Pipeline::Undo));
        let out = press(&mut m, &[Key::Enter, Key::Enter]);
        assert_eq!(out, Outcome::Run(vec!["--undo".into()]));
    }

    #[test]
    fn every_line_fits_and_the_steps_say_what_they_are() {
        let d = tempfile::tempdir().unwrap();
        let cwd = d.path().to_str().unwrap();
        for w in [40usize, 60, 80, 120, 200] {
            let t = Theme::custom(true, true, w, true);
            let mut m = Menu::new(cwd);
            for l in m.view(&t) {
                assert!(visible_width(&l) < w, "{w}: {l:?}");
            }
            let pick = crate::util::strip_sgr(&m.view(&t).join("\n"));
            assert!(pick.contains("BATCH") && pick.contains("REVIEW"), "{pick}");
            m.handle(Key::Enter);
            let folder = crate::util::strip_sgr(&m.view(&t).join("\n"));
            assert!(
                folder.contains("FOLDER") && folder.contains("batch"),
                "{folder}"
            );
            for l in m.view(&t) {
                assert!(visible_width(&l) < w, "{w}: {l:?}");
            }
        }
    }
}
