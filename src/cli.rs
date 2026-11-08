//! Grammar (docs/CLI.md §1):
//!
//! ```text
//! dedupe                                   menu
//! dedupe <find|milk|purge> [PATH] [.] [img|vid] [-n] [-q]
//! dedupe --undo | -u [PATH] [-n]
//! dedupe doctor | --doctor | -dr [PATH]
//! dedupe --help | -h
//! ```
//!
//! Hand-written: the grammar has a bare `.` with a meaning, a two-letter
//! short flag `-dr`, and order-independent word tokens. A POSIX-shaped
//! parser would fight all three.

use std::fmt;
use std::path::{Path, PathBuf};

use crate::kind::Kind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Find,
    Milk,
    Purge,
    Doctor,
    Undo,
    Help,
    Menu,
}

impl Command {
    pub fn word(self) -> &'static str {
        match self {
            Command::Find => "find",
            Command::Milk => "milk",
            Command::Purge => "purge",
            Command::Doctor => "doctor",
            Command::Undo => "--undo",
            Command::Help => "--help",
            Command::Menu => "",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invocation {
    pub command: Command,
    /// Absolute root. For `Help` and `Menu` it is the current directory.
    pub root: PathBuf,
    /// The root as the user typed it (for echoing in output); `None` means
    /// "current directory".
    pub root_display: Option<String>,
    pub recurse: bool,
    pub kind: Kind,
    pub dry_run: bool,
    pub quick: bool,
}

impl Invocation {
    /// The native command line that reproduces this invocation.
    /// Used by the menu's confirm step and by the "next" lines.
    pub fn command_line(&self) -> String {
        self.command_line_as(self.command)
    }

    pub fn command_line_as(&self, command: Command) -> String {
        let mut parts: Vec<String> = vec!["dedupe".into()];
        match command {
            Command::Menu => {}
            Command::Help => parts.push("--help".into()),
            c => parts.push(c.word().into()),
        }
        if let Some(r) = &self.root_display {
            parts.push(quote_if_needed(r));
        }
        if matches!(command, Command::Find | Command::Milk | Command::Purge) {
            if self.recurse {
                parts.push(".".into());
            }
            let k = self.kind.token();
            if !k.is_empty() {
                parts.push(k.into());
            }
            if self.quick {
                parts.push("--quick-search".into());
            }
        }
        if self.dry_run && matches!(command, Command::Milk | Command::Purge | Command::Undo) {
            parts.push("--dry-run".into());
        }
        parts.join(" ")
    }
}

fn quote_if_needed(s: &str) -> String {
    if s.contains(' ') {
        format!("\"{s}\"")
    } else {
        s.to_string()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliError {
    UnknownFlag(String),
    UnknownToken(String),
    MissingCommand(String),
    SecondPath { first: String, second: String },
    SecondKind { first: String, second: String },
    RootMissing(String),
    RootNotDir(String),
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CliError::UnknownFlag(s) => write!(f, "unknown flag  {s}"),
            CliError::UnknownToken(s) => write!(f, "unknown token  {s}"),
            CliError::MissingCommand(s) => {
                write!(f, "no command before  {s}   (find, milk, purge, doctor)")
            }
            CliError::SecondPath { first, second } => {
                write!(f, "two paths given  {first}  and  {second}")
            }
            CliError::SecondKind { first, second } => {
                write!(
                    f,
                    "one kind at a time  {first}  and  {second}   (leave both out for everything)"
                )
            }
            CliError::RootMissing(s) => write!(f, "folder not found  {s}"),
            CliError::RootNotDir(s) => write!(f, "not a folder  {s}"),
        }
    }
}

impl std::error::Error for CliError {}

/// Rewrite spec aliases that a flag parser cannot carry: `-dr` and
/// `--doctor` become the `doctor` command word.
fn rewrite(args: &[String]) -> Vec<String> {
    args.iter()
        .map(|a| match a.as_str() {
            "-dr" | "--doctor" => "doctor".to_string(),
            other => other.to_string(),
        })
        .collect()
}

/// Parse `args` (without the program name). `cwd` is the directory the
/// user is in; it becomes the root when no PATH is given.
pub fn parse(args: &[String], cwd: &Path) -> Result<Invocation, CliError> {
    let args = rewrite(args);

    let mut command: Option<Command> = None;
    let mut path: Option<String> = None;
    let mut recurse = false;
    let mut kind = Kind::All;
    let mut dry_run = false;
    let mut quick = false;
    let mut undo = false;
    let mut help = false;

    for tok in &args {
        let t = tok.as_str();
        if t.starts_with('-') && t != "-" {
            match t {
                "--help" | "-h" => help = true,
                "--dry-run" | "-n" => dry_run = true,
                "--quick-search" | "-q" => quick = true,
                "--undo" | "-u" => undo = true,
                other => return Err(CliError::UnknownFlag(other.to_string())),
            }
            continue;
        }
        if command.is_none() && path.is_none() {
            let c = match t {
                "find" => Some(Command::Find),
                "milk" => Some(Command::Milk),
                "purge" => Some(Command::Purge),
                "doctor" => Some(Command::Doctor),
                _ => None,
            };
            if let Some(c) = c {
                command = Some(c);
                continue;
            }
        } else if command.is_none() {
            // a path came first and then a command word — accept it too
            let c = match t {
                "find" => Some(Command::Find),
                "milk" => Some(Command::Milk),
                "purge" => Some(Command::Purge),
                "doctor" => Some(Command::Doctor),
                _ => None,
            };
            if let Some(c) = c {
                command = Some(c);
                continue;
            }
        }
        if t == "." {
            recurse = true;
            continue;
        }
        if let Some(k) = Kind::from_token(t) {
            if kind != Kind::All {
                return Err(CliError::SecondKind {
                    first: kind.token().to_string(),
                    second: t.to_string(),
                });
            }
            kind = k;
            continue;
        }
        if t == "-" {
            return Err(CliError::UnknownToken(t.to_string()));
        }
        match &path {
            None => path = Some(t.to_string()),
            Some(first) => {
                return Err(CliError::SecondPath {
                    first: first.clone(),
                    second: t.to_string(),
                })
            }
        }
    }

    let command = if help {
        Command::Help
    } else if undo {
        Command::Undo
    } else if let Some(c) = command {
        c
    } else if path.is_none() && !recurse && kind == Kind::All && !dry_run && !quick {
        Command::Menu
    } else {
        let culprit = path
            .clone()
            .or(if recurse { Some(".".into()) } else { None })
            .unwrap_or_else(|| kind.token().to_string());
        let culprit = if culprit.is_empty() {
            if dry_run {
                "--dry-run".into()
            } else {
                "--quick-search".into()
            }
        } else {
            culprit
        };
        return Err(CliError::MissingCommand(culprit));
    };

    let root = resolve_root(path.as_deref(), cwd, command)?;

    Ok(Invocation {
        command,
        root,
        root_display: path,
        recurse,
        kind,
        dry_run,
        quick,
    })
}

fn resolve_root(path: Option<&str>, cwd: &Path, command: Command) -> Result<PathBuf, CliError> {
    let candidate = match path {
        None => cwd.to_path_buf(),
        Some(p) => {
            let pb = PathBuf::from(p);
            if pb.is_absolute() {
                pb
            } else {
                cwd.join(pb)
            }
        }
    };
    if matches!(command, Command::Help | Command::Menu) {
        return Ok(candidate);
    }
    if !candidate.exists() {
        return Err(CliError::RootMissing(path.unwrap_or(".").to_string()));
    }
    if !candidate.is_dir() {
        return Err(CliError::RootNotDir(path.unwrap_or(".").to_string()));
    }
    // canonicalize for a stable journal key; fall back to the joined path.
    // On Windows canonicalize answers with a `\\?\` verbatim path; the
    // prefix is stripped so every printed and journaled path reads as
    // Explorer shows it (std re-adds it for long paths internally).
    Ok(crate::util::strip_verbatim(
        &std::fs::canonicalize(&candidate).unwrap_or(candidate),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn cwd() -> PathBuf {
        std::env::temp_dir()
    }

    #[test]
    fn nothing_opens_menu() {
        let inv = parse(&a(&[]), &cwd()).unwrap();
        assert_eq!(inv.command, Command::Menu);
    }

    #[test]
    fn find_dot_recurses_in_cwd() {
        let inv = parse(&a(&["find", "."]), &cwd()).unwrap();
        assert_eq!(inv.command, Command::Find);
        assert!(inv.recurse);
        assert_eq!(inv.root_display, None);
    }

    #[test]
    fn find_without_dot_is_top_level() {
        let inv = parse(&a(&["find"]), &cwd()).unwrap();
        assert!(!inv.recurse);
    }

    #[test]
    fn path_then_dot_then_kind_in_any_order() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().to_str().unwrap().to_string();
        let i1 = parse(&a(&["milk", &p, ".", "img"]), &cwd()).unwrap();
        let i2 = parse(&a(&["milk", "img", ".", &p]), &cwd()).unwrap();
        let i3 = parse(&a(&[&p, "milk", ".", "img"]), &cwd()).unwrap();
        assert_eq!(i1.command, Command::Milk);
        assert!(i1.recurse && i2.recurse && i3.recurse);
        assert_eq!(i1.kind, Kind::Image);
        assert_eq!(i1.root, i2.root);
        assert_eq!(i1.root, i3.root);
    }

    #[test]
    fn doctor_aliases() {
        for form in [&["doctor"][..], &["--doctor"], &["-dr"]] {
            let inv = parse(&a(form), &cwd()).unwrap();
            assert_eq!(inv.command, Command::Doctor);
        }
    }

    #[test]
    fn undo_overrides_command() {
        let inv = parse(&a(&["find", "--undo"]), &cwd()).unwrap();
        assert_eq!(inv.command, Command::Undo);
        let inv = parse(&a(&["-u"]), &cwd()).unwrap();
        assert_eq!(inv.command, Command::Undo);
    }

    #[test]
    fn help_wins() {
        let inv = parse(&a(&["purge", ".", "-h"]), &cwd()).unwrap();
        assert_eq!(inv.command, Command::Help);
    }

    #[test]
    fn flags_short_and_long() {
        let inv = parse(&a(&["purge", "-n", "-q"]), &cwd()).unwrap();
        assert!(inv.dry_run && inv.quick);
        let inv = parse(&a(&["purge", "--dry-run", "--quick-search"]), &cwd()).unwrap();
        assert!(inv.dry_run && inv.quick);
    }

    #[test]
    fn errors() {
        assert!(matches!(
            parse(&a(&["find", "--nope"]), &cwd()),
            Err(CliError::UnknownFlag(_))
        ));
        assert!(matches!(
            parse(&a(&["."]), &cwd()),
            Err(CliError::MissingCommand(_))
        ));
        assert!(matches!(
            parse(&a(&["find", "a", "b"]), &cwd()),
            Err(CliError::SecondPath { .. })
        ));
        assert!(matches!(
            parse(&a(&["find", ".", "img", "vid"]), &cwd()),
            Err(CliError::SecondKind { .. })
        ));
        assert!(matches!(
            parse(&a(&["find", ".", "img", "image"]), &cwd()),
            Err(CliError::SecondKind { .. })
        ));
        assert!(matches!(
            parse(&a(&["find", "/definitely/not/here/xyz"]), &cwd()),
            Err(CliError::RootMissing(_))
        ));
    }

    #[test]
    fn command_line_round_trip() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().to_str().unwrap().to_string();
        let inv = parse(&a(&["milk", &p, ".", "vid", "-n"]), &cwd()).unwrap();
        assert_eq!(
            inv.command_line(),
            format!("dedupe milk {p} . vid --dry-run")
        );
        assert_eq!(
            inv.command_line_as(Command::Purge),
            format!("dedupe purge {p} . vid --dry-run")
        );
        let inv = parse(&a(&["find", "."]), &cwd()).unwrap();
        assert_eq!(inv.command_line(), "dedupe find .");
        assert_eq!(inv.command_line_as(Command::Milk), "dedupe milk .");
    }
}
