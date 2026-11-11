//! Where `ffmpeg` and `ffprobe` are.
//!
//! PATH first, as a shell would find them. When PATH does not name them —
//! a console started before an installer changed PATH, or an installer
//! that never added its folder — the places the common installers put
//! them, so an installed ffmpeg is used without the person editing PATH
//! (found on the owner's Windows machine, 2026-11-11: WinGet's
//! `Gyan.FFmpeg` 9.0.2 installed, PATH without it). [CLAUDE]
//!
//! - Windows: next to `dedupe.exe`; WinGet's `Links` folder and its
//!   `Packages\*FFmpeg*\*\bin`; Scoop's `shims`; Chocolatey's `bin`;
//!   `Program Files\ffmpeg\bin`; `C:\ffmpeg\bin`.
//! - macOS: Homebrew (`/opt/homebrew/bin`, `/usr/local/bin`), which a
//!   program started outside a login shell may not have on PATH.
//! - Elsewhere: `/usr/local/bin`, `/snap/bin`.
//!
//! `ffprobe` is looked for beside the `ffmpeg` found first, so the two
//! come from one build. Each is resolved once per run.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

/// One of the two programs, and where it was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Located {
    pub path: PathBuf,
    /// Found through PATH (the alternative: one of the install folders).
    pub on_path: bool,
}

fn exe(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

fn in_dir(dir: &Path, name: &str) -> Option<PathBuf> {
    let p = dir.join(exe(name));
    p.is_file().then_some(p)
}

fn on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).find_map(|d| in_dir(&d, name))
}

/// The install folders, in the order they are tried.
fn install_dirs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(d) = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(Path::to_path_buf))
    {
        v.push(d);
    }
    if cfg!(windows) {
        let env = |k: &str| std::env::var_os(k).map(PathBuf::from);
        if let Some(local) = env("LOCALAPPDATA") {
            let winget = local.join("Microsoft").join("WinGet");
            v.push(winget.join("Links"));
            // a portable package: Packages\<id>_<source>\<archive folder>\bin
            if let Ok(rd) = std::fs::read_dir(winget.join("Packages")) {
                let mut pkgs: Vec<PathBuf> = rd
                    .flatten()
                    .map(|e| e.path())
                    .filter(|p| {
                        p.file_name()
                            .is_some_and(|n| n.to_string_lossy().to_lowercase().contains("ffmpeg"))
                    })
                    .collect();
                pkgs.sort();
                for p in pkgs {
                    if let Ok(rd) = std::fs::read_dir(&p) {
                        let mut builds: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
                        // the newest build folder first (names carry the version)
                        builds.sort();
                        builds.reverse();
                        for b in builds {
                            v.push(b.join("bin"));
                            v.push(b);
                        }
                    }
                }
            }
        }
        if let Some(home) = env("USERPROFILE") {
            v.push(home.join("scoop").join("shims"));
        }
        if let Some(pd) = env("ProgramData") {
            v.push(pd.join("chocolatey").join("bin"));
        }
        if let Some(pf) = env("ProgramFiles") {
            v.push(pf.join("ffmpeg").join("bin"));
        }
        v.push(PathBuf::from(r"C:\ffmpeg\bin"));
    } else if cfg!(target_os = "macos") {
        v.push(PathBuf::from("/opt/homebrew/bin"));
        v.push(PathBuf::from("/usr/local/bin"));
    } else {
        v.push(PathBuf::from("/usr/local/bin"));
        v.push(PathBuf::from("/snap/bin"));
    }
    v
}

fn locate(name: &str, beside: Option<&Located>) -> Option<Located> {
    if let Some(p) = on_path(name) {
        return Some(Located {
            path: p,
            on_path: true,
        });
    }
    if let Some(b) = beside.and_then(|b| b.path.parent()) {
        if let Some(p) = in_dir(b, name) {
            return Some(Located {
                path: p,
                on_path: false,
            });
        }
    }
    install_dirs().iter().find_map(|d| {
        in_dir(d, name).map(|p| Located {
            path: p,
            on_path: false,
        })
    })
}

static FFMPEG: OnceLock<Option<Located>> = OnceLock::new();
static FFPROBE: OnceLock<Option<Located>> = OnceLock::new();

/// Where `ffmpeg` is, if anywhere.
pub fn ffmpeg_at() -> Option<&'static Located> {
    FFMPEG.get_or_init(|| locate("ffmpeg", None)).as_ref()
}

/// Where `ffprobe` is, if anywhere: beside the `ffmpeg` found first.
pub fn ffprobe_at() -> Option<&'static Located> {
    FFPROBE
        .get_or_init(|| locate("ffprobe", ffmpeg_at()))
        .as_ref()
}

/// A command for `ffmpeg`: the one found, or the bare name (which then
/// fails to start with the system's own error).
pub fn ffmpeg() -> Command {
    Command::new(ffmpeg_at().map_or(PathBuf::from("ffmpeg"), |l| l.path.clone()))
}

/// A command for `ffprobe`, as `ffmpeg()`.
pub fn ffprobe() -> Command {
    Command::new(ffprobe_at().map_or(PathBuf::from("ffprobe"), |l| l.path.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_program_in_a_folder_is_found_and_one_elsewhere_is_not() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(in_dir(d.path(), "ffmpeg"), None);
        std::fs::write(d.path().join(exe("ffmpeg")), b"").unwrap();
        assert_eq!(
            in_dir(d.path(), "ffmpeg"),
            Some(d.path().join(exe("ffmpeg")))
        );
        // ffprobe beside the ffmpeg that was found, before any install folder
        std::fs::write(d.path().join(exe("ffprobe")), b"").unwrap();
        let ff = Located {
            path: d.path().join(exe("ffmpeg")),
            on_path: false,
        };
        let probe = locate("ffprobe", Some(&ff)).unwrap();
        // PATH may hold an ffprobe of its own on this machine; then it wins
        if !probe.on_path {
            assert_eq!(probe.path, d.path().join(exe("ffprobe")));
        }
        // the install folders end where they should
        let dirs = install_dirs();
        assert!(!dirs.is_empty());
        if cfg!(windows) {
            assert!(dirs.iter().any(|p| p.ends_with(r"WinGet\Links")));
        }
    }
}
