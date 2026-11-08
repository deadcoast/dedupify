//! The workflow menu (docs/CLI.md §4). The state machine is pure and
//! tested; `run` wraps it in crossterm raw mode and returns the native
//! argv the user composed, which `main` then parses and runs like any
//! typed command.

use std::io::{self, Write};
use std::path::Path;

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::{cursor, execute, terminal};

use crate::theme::{Role, Theme};
use crate::util::{fit, shorten_middle, width};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Workflow {
    Find,
    Milk,
    Purge,
    Undo,
    Doctor,
}

pub const WORKFLOWS: [(Workflow, &str, &str); 5] = [
    (Workflow::Find, "find", "see duplicates, change nothing"),
    (Workflow::Milk, "milk", "move duplicates into _milked"),
    (
        Workflow::Purge,
        "purge",
        "send duplicates to the Recycle Bin",
    ),
    (Workflow::Undo, "undo", "reverse the last change"),
    (Workflow::Doctor, "doctor", "check this terminal and folder"),
];

pub const SCOPES: [(&str, &str); 2] = [("this folder only", ""), ("include subfolders", ".")];
pub const KINDS: [(&str, &str); 3] = [("everything", ""), ("images", "img"), ("videos", "vid")];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Workflow,
    Folder,
    Scope,
    Kind,
    Confirm,
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

#[derive(Debug, Clone)]
pub struct Menu {
    pub step: Step,
    pub workflow: usize,
    pub folder: String,
    pub folder_error: Option<String>,
    pub scope: usize,
    pub kind: usize,
}

impl Menu {
    pub fn new(cwd: &str) -> Menu {
        Menu {
            step: Step::Workflow,
            workflow: 0,
            folder: cwd.to_string(),
            folder_error: None,
            scope: 0,
            kind: 0,
        }
    }

    fn selected(&self) -> Workflow {
        WORKFLOWS[self.workflow].0
    }

    fn needs_scope(&self) -> bool {
        matches!(
            self.selected(),
            Workflow::Find | Workflow::Milk | Workflow::Purge
        )
    }

    /// argv without the program name.
    pub fn args(&self) -> Vec<String> {
        let mut v: Vec<String> = Vec::new();
        match self.selected() {
            Workflow::Find => v.push("find".into()),
            Workflow::Milk => v.push("milk".into()),
            Workflow::Purge => v.push("purge".into()),
            Workflow::Undo => v.push("--undo".into()),
            Workflow::Doctor => v.push("doctor".into()),
        }
        v.push(self.folder.clone());
        if self.needs_scope() {
            let s = SCOPES[self.scope].1;
            if !s.is_empty() {
                v.push(s.into());
            }
            let k = KINDS[self.kind].1;
            if !k.is_empty() {
                v.push(k.into());
            }
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
        match self.step {
            Step::Workflow => match key {
                Key::Up => self.workflow = (self.workflow + WORKFLOWS.len() - 1) % WORKFLOWS.len(),
                Key::Down => self.workflow = (self.workflow + 1) % WORKFLOWS.len(),
                Key::Enter => self.step = Step::Folder,
                Key::Esc => return Outcome::Quit,
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
                    let p = Path::new(self.folder.trim());
                    if self.folder.trim().is_empty() {
                        self.folder_error = Some("type a folder".into());
                    } else if !p.is_dir() {
                        self.folder_error = Some("not a folder".into());
                    } else {
                        self.folder = self.folder.trim().to_string();
                        self.step = if self.needs_scope() {
                            Step::Scope
                        } else {
                            Step::Confirm
                        };
                    }
                }
                Key::Esc => self.step = Step::Workflow,
                _ => {}
            },
            Step::Scope => match key {
                Key::Up | Key::Down => self.scope = 1 - self.scope,
                Key::Enter => self.step = Step::Kind,
                Key::Esc => self.step = Step::Folder,
                _ => {}
            },
            Step::Kind => match key {
                Key::Up => self.kind = (self.kind + KINDS.len() - 1) % KINDS.len(),
                Key::Down => self.kind = (self.kind + 1) % KINDS.len(),
                Key::Enter => self.step = Step::Confirm,
                Key::Esc => self.step = Step::Scope,
                _ => {}
            },
            Step::Confirm => match key {
                Key::Enter => return Outcome::Run(self.args()),
                Key::Esc => {
                    self.step = if self.needs_scope() {
                        Step::Kind
                    } else {
                        Step::Folder
                    }
                }
                _ => {}
            },
        }
        Outcome::Continue
    }

    /// The frame as lines. `inner` cells between the two verticals.
    pub fn view(&self, t: &Theme) -> Vec<String> {
        let g = t.glyphs;
        let total = t.width.clamp(44, 64);
        let inner = total - 2;
        let mut out: Vec<String> = Vec::new();

        let title = format!("{} dedupe ", g.h);
        let top = format!(
            "{}{}{}{}",
            g.tl,
            title,
            g.h.repeat(inner.saturating_sub(width(&title))),
            g.tr
        );
        out.push(format!(" {}", t.paint(Role::Rule, &top)));

        let mut body: Vec<Vec<(String, Role, bool)>> = Vec::new(); // (text, role, bold)
        let push = |body: &mut Vec<Vec<(String, Role, bool)>>, segs: Vec<(String, Role, bool)>| {
            body.push(segs)
        };
        let plain = |s: &str| (s.to_string(), Role::Plate, false);
        let dim = |s: &str| (s.to_string(), Role::Dupe, false);
        let lit = |s: &str| (s.to_string(), Role::Keep, true);

        let heading = match self.step {
            Step::Workflow => "WORKFLOW",
            Step::Folder => "FOLDER",
            Step::Scope => "SCOPE",
            Step::Kind => "KIND",
            Step::Confirm => "CONFIRM",
        };
        push(&mut body, vec![(format!("  {heading}"), Role::Plate, true)]);

        match self.step {
            Step::Workflow => {
                for (i, (_, name, desc)) in WORKFLOWS.iter().enumerate() {
                    let sel = i == self.workflow;
                    let ptr = if sel { g.pointer } else { " " };
                    let mut segs = vec![(format!("  {ptr} "), Role::Keep, true)];
                    segs.push(if sel {
                        lit(&fit(name, 8))
                    } else {
                        plain(&fit(name, 8))
                    });
                    segs.push(dim(desc));
                    push(&mut body, segs);
                }
                push(&mut body, vec![plain("")]);
                push(
                    &mut body,
                    vec![dim(&format!(
                        "  {}{} move   enter select   esc quit",
                        g.up, g.down
                    ))],
                );
            }
            Step::Folder => {
                let avail = inner.saturating_sub(6);
                let shown = shorten_middle(&self.folder, avail, g.ellipsis);
                push(
                    &mut body,
                    vec![
                        (format!("  {} ", g.pointer), Role::Keep, true),
                        lit(&shown),
                        plain("_"),
                    ],
                );
                match &self.folder_error {
                    Some(e) => push(&mut body, vec![(format!("    {e}"), Role::Danger, false)]),
                    None => push(&mut body, vec![plain("")]),
                }
                push(
                    &mut body,
                    vec![dim("  type a path   enter accept   esc back")],
                );
            }
            Step::Scope => {
                for (i, (name, _)) in SCOPES.iter().enumerate() {
                    let sel = i == self.scope;
                    let ptr = if sel { g.pointer } else { " " };
                    push(
                        &mut body,
                        vec![
                            (format!("  {ptr} "), Role::Keep, true),
                            if sel { lit(name) } else { plain(name) },
                        ],
                    );
                }
                push(&mut body, vec![plain("")]);
                push(
                    &mut body,
                    vec![dim(&format!(
                        "  {}{} move   enter select   esc back",
                        g.up, g.down
                    ))],
                );
            }
            Step::Kind => {
                for (i, (name, _)) in KINDS.iter().enumerate() {
                    let sel = i == self.kind;
                    let ptr = if sel { g.pointer } else { " " };
                    push(
                        &mut body,
                        vec![
                            (format!("  {ptr} "), Role::Keep, true),
                            if sel { lit(name) } else { plain(name) },
                        ],
                    );
                }
                push(&mut body, vec![plain("")]);
                push(
                    &mut body,
                    vec![dim(&format!(
                        "  {}{} move   enter select   esc back",
                        g.up, g.down
                    ))],
                );
            }
            Step::Confirm => {
                let avail = inner.saturating_sub(6);
                let cmd = shorten_middle(&self.command_line(), avail, g.ellipsis);
                push(
                    &mut body,
                    vec![(format!("  {} ", g.pointer), Role::Keep, true), lit(&cmd)],
                );
                let what = match self.selected() {
                    Workflow::Find => "prints groups, changes nothing",
                    Workflow::Milk => "moves duplicates; dedupe --undo reverses it",
                    Workflow::Purge => {
                        "verifies bytes, then Recycle Bin; dedupe --undo reverses it"
                    }
                    Workflow::Undo => "reverses the newest journal for this folder",
                    Workflow::Doctor => "runs the checks, changes nothing",
                };
                push(&mut body, vec![dim(&format!("    {what}"))]);
                push(&mut body, vec![plain("")]);
                push(&mut body, vec![dim("  enter run   esc back")]);
            }
        }

        for segs in body {
            let plain_text: String = segs.iter().map(|(s, _, _)| s.as_str()).collect();
            let w = width(&plain_text);
            let mut line = String::new();
            if w > inner {
                // cut the last segment to fit
                let mut used = 0;
                for (s, role, bold) in &segs {
                    let room = inner.saturating_sub(used);
                    let piece = fit(s, room.min(width(s)));
                    used += width(&piece);
                    line.push_str(&if *bold {
                        t.paint_bold(*role, &piece)
                    } else {
                        t.paint(*role, &piece)
                    });
                    if used >= inner {
                        break;
                    }
                }
                let pad = inner.saturating_sub(used);
                line.push_str(&" ".repeat(pad));
            } else {
                for (s, role, bold) in &segs {
                    line.push_str(&if *bold {
                        t.paint_bold(*role, s)
                    } else {
                        t.paint(*role, s)
                    });
                }
                line.push_str(&" ".repeat(inner - w));
            }
            out.push(format!(
                " {}{}{}",
                t.paint(Role::Rule, g.v),
                line,
                t.paint(Role::Rule, g.v)
            ));
        }

        let bottom = format!("{}{}{}", g.bl, g.h.repeat(inner), g.br);
        out.push(format!(" {}", t.paint(Role::Rule, &bottom)));
        out
    }
}

/// Run the menu in raw mode. Returns the argv to execute, or `None` on quit.
pub fn run(theme: Theme, cwd: &str) -> io::Result<Option<Vec<String>>> {
    let mut menu = Menu::new(cwd);
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

    #[test]
    fn find_recursive_images_composes_native_command() {
        let d = tempfile::tempdir().unwrap();
        let cwd = d.path().to_str().unwrap();
        let mut m = Menu::new(cwd);
        // find → folder (accept cwd) → scope: subfolders → kind: images → confirm
        let out = press(
            &mut m,
            &[
                Key::Enter,
                Key::Enter,
                Key::Down,
                Key::Enter,
                Key::Down,
                Key::Enter,
                Key::Enter,
            ],
        );
        assert_eq!(
            out,
            Outcome::Run(vec!["find".into(), cwd.into(), ".".into(), "img".into()])
        );
        assert_eq!(m.command_line(), format!("dedupe find {cwd} . img"));
    }

    #[test]
    fn undo_skips_scope_and_kind() {
        let d = tempfile::tempdir().unwrap();
        let cwd = d.path().to_str().unwrap();
        let mut m = Menu::new(cwd);
        let out = press(
            &mut m,
            &[
                Key::Down,
                Key::Down,
                Key::Down,
                Key::Enter,
                Key::Enter,
                Key::Enter,
            ],
        );
        assert_eq!(out, Outcome::Run(vec!["--undo".into(), cwd.into()]));
    }

    #[test]
    fn bad_folder_stays_on_folder_step() {
        let mut m = Menu::new("/definitely/not/a/folder/xyz");
        press(&mut m, &[Key::Enter, Key::Enter]);
        assert_eq!(m.step, Step::Folder);
        assert!(m.folder_error.is_some());
    }

    #[test]
    fn esc_walks_back_and_quits_at_top() {
        let d = tempfile::tempdir().unwrap();
        let cwd = d.path().to_str().unwrap();
        let mut m = Menu::new(cwd);
        press(&mut m, &[Key::Enter, Key::Enter, Key::Enter, Key::Enter]);
        assert_eq!(m.step, Step::Confirm);
        press(&mut m, &[Key::Esc, Key::Esc, Key::Esc, Key::Esc]);
        assert_eq!(m.step, Step::Workflow);
        assert_eq!(m.handle(Key::Esc), Outcome::Quit);
    }

    #[test]
    fn frame_lines_are_exactly_frame_width() {
        let d = tempfile::tempdir().unwrap();
        let cwd = d.path().to_str().unwrap();
        let mut m = Menu::new(cwd);
        let t = Theme::custom(false, true, 80, false);
        for step in [
            Step::Workflow,
            Step::Folder,
            Step::Scope,
            Step::Kind,
            Step::Confirm,
        ] {
            m.step = step;
            let lines = m.view(&t);
            let w0 = width(&lines[0]);
            for l in &lines {
                assert_eq!(width(l), w0, "{step:?}: {l:?}");
            }
            assert!(lines[0].ends_with('┓'));
            assert!(lines.last().unwrap().ends_with('┛'));
        }
    }
}
