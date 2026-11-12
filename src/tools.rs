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

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

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

// ---------------------------------------------------------------------------
// a clock on every child (Phase 13, CORE-11)
// ---------------------------------------------------------------------------

/// How long a child may go without sending a byte, on stdout or stderr,
/// before it is taken for stalled and stopped. Every call here makes
/// ffmpeg or ffprobe write as it works (frames, cropdetect lines, a
/// decoded picture) or answer within a second (a probe), so a minute of
/// silence is a stream that will not end or a share that went away — one
/// such file used to hold the whole run. `DEDUPE_STALL` (seconds) sets it,
/// for a slow network share or a test. [CLAUDE]
pub fn stall() -> Duration {
    static STALL: OnceLock<Duration> = OnceLock::new();
    *STALL.get_or_init(|| {
        std::env::var("DEDUPE_STALL")
            .ok()
            .and_then(|v| v.trim().parse::<f64>().ok())
            .filter(|s| s.is_finite() && *s > 0.0)
            .map_or(Duration::from_secs(60), Duration::from_secs_f64)
    })
}

/// What a child sent, and how it ended.
pub struct Ran {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// A child watched for silence: `touch` when a byte arrives; a thread
/// kills the child when none has for `stall()`.
pub struct Watch {
    child: Arc<Mutex<Child>>,
    t0: Instant,
    last: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    fired: Arc<AtomicBool>,
    thread: Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl Watch {
    pub fn new(child: Child) -> Watch {
        Watch::with(child, stall())
    }

    fn with(child: Child, limit: Duration) -> Watch {
        let child = Arc::new(Mutex::new(child));
        let t0 = Instant::now();
        let last = Arc::new(AtomicU64::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let fired = Arc::new(AtomicBool::new(false));
        let limit = limit.as_millis() as u64;
        let thread = {
            let (child, last, stop, fired) =
                (child.clone(), last.clone(), stop.clone(), fired.clone());
            std::thread::spawn(move || {
                let tick = Duration::from_millis((limit / 8).clamp(10, 250));
                while !stop.load(Ordering::Relaxed) {
                    std::thread::sleep(tick);
                    let now = t0.elapsed().as_millis() as u64;
                    if now.saturating_sub(last.load(Ordering::Relaxed)) > limit {
                        fired.store(true, Ordering::Relaxed);
                        let _ = lock(&child).kill();
                        break;
                    }
                }
            })
        };
        Watch {
            child,
            t0,
            last,
            stop,
            fired,
            thread: Mutex::new(Some(thread)),
        }
    }

    /// A byte arrived.
    pub fn touch(&self) {
        self.last
            .store(self.t0.elapsed().as_millis() as u64, Ordering::Relaxed);
    }

    /// The child was stopped for its silence.
    pub fn stalled(&self) -> bool {
        self.fired.load(Ordering::Relaxed)
    }

    /// Stop the child now (an error on our side).
    pub fn kill(&self) {
        let _ = lock(&self.child).kill();
    }

    /// Reap a child the clock stopped, so it leaves no zombie behind.
    fn kill_if(&self, stalled: bool) {
        if stalled {
            let mut c = lock(&self.child);
            let _ = c.kill();
            let _ = c.wait();
        }
    }

    /// Stop watching and wait for the child's end.
    pub fn finish(&self) -> io::Result<ExitStatus> {
        self.unwatch();
        let status = lock(&self.child).wait();
        status
    }

    fn unwatch(&self) {
        self.stop.store(true, Ordering::Relaxed);
        let t = self.thread.lock().unwrap_or_else(|p| p.into_inner()).take();
        if let Some(t) = t {
            let _ = t.join();
        }
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        self.unwatch();
    }
}

fn lock(m: &Mutex<Child>) -> std::sync::MutexGuard<'_, Child> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// Run `cmd` to its end: `input` written to its stdin from a thread, stdout
/// and stderr read as they come, and the child stopped when neither has
/// sent a byte for `stall()` — then an `io::ErrorKind::TimedOut` error.
pub fn run(cmd: &mut Command, input: Option<Vec<u8>>) -> io::Result<Ran> {
    run_with(cmd, input, stall())
}

fn run_with(cmd: &mut Command, input: Option<Vec<u8>>, limit: Duration) -> io::Result<Ran> {
    let mut child = cmd
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let writer = input.and_then(|data| {
        child.stdin.take().map(|mut stdin| {
            std::thread::spawn(move || {
                // a decoder that stops early closes the pipe; the exit
                // status says what happened
                let _ = stdin.write_all(&data);
            })
        })
    });
    let out = child.stdout.take();
    let err = child.stderr.take();
    let watch = Arc::new(Watch::with(child, limit));
    // each pipe read on a thread of its own, its bytes sent back at its end
    let (tx, rx) = std::sync::mpsc::channel::<(usize, Vec<u8>)>();
    let reader = |i: usize, pipe: Option<Box<dyn Read + Send>>| {
        let (watch, tx) = (watch.clone(), tx.clone());
        std::thread::spawn(move || {
            let mut v = Vec::new();
            if let Some(mut p) = pipe {
                let mut buf = [0u8; 16 * 1024];
                while let Ok(n) = p.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    watch.touch();
                    v.extend_from_slice(&buf[..n]);
                }
            }
            let _ = tx.send((i, v));
        });
    };
    reader(0, out.map(|p| Box::new(p) as Box<dyn Read + Send>));
    reader(1, err.map(|p| Box::new(p) as Box<dyn Read + Send>));
    drop(tx);
    let mut got: [Option<Vec<u8>>; 2] = [None, None];
    while got.iter().any(Option::is_none) {
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok((i, v)) => got[i] = Some(v),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
        // stopped for its silence: a pipe a process of its own still holds
        // open must not hold this run too; its reader ends when it closes
        if watch.stalled() {
            break;
        }
    }
    if let Some(w) = writer {
        if !watch.stalled() {
            let _ = w.join();
        }
    }
    let stalled = watch.stalled();
    watch.kill_if(stalled);
    if stalled {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            format!("sent nothing for {} s", limit.as_secs_f64()),
        ));
    }
    let status = watch.finish()?;
    let [stdout, stderr] = got.map(Option::unwrap_or_default);
    Ok(Ran {
        status,
        stdout,
        stderr,
    })
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

    fn shell(script: &str) -> Command {
        if cfg!(windows) {
            let mut c = Command::new("cmd");
            c.args(["/C", script]);
            c
        } else {
            let mut c = Command::new("sh");
            c.args(["-c", script]);
            c
        }
    }

    #[test]
    fn a_child_that_talks_runs_to_its_end_and_a_silent_one_is_stopped() {
        let limit = Duration::from_millis(1500);
        // talks, then ends: everything it sent, in order
        let r = run_with(&mut shell("echo one&& echo two 1>&2"), None, limit).unwrap();
        assert!(r.status.success());
        assert!(String::from_utf8_lossy(&r.stdout).contains("one"));
        assert!(String::from_utf8_lossy(&r.stderr).contains("two"));
        // input reaches it
        let cat = if cfg!(windows) { "findstr x" } else { "cat" };
        let r = run_with(&mut shell(cat), Some(b"x marks\n".to_vec()), limit).unwrap();
        assert!(String::from_utf8_lossy(&r.stdout).contains("x marks"));
        // silent for longer than the limit: stopped, an error that says so
        let mut sleep = if cfg!(windows) {
            let mut c = Command::new("powershell");
            c.args(["-NoProfile", "-Command", "Start-Sleep 5"]);
            c
        } else {
            shell("exec sleep 5")
        };
        let t = Instant::now();
        let e = run_with(&mut sleep, None, limit).err().unwrap();
        assert_eq!(e.kind(), io::ErrorKind::TimedOut, "{e}");
        assert!(t.elapsed() < Duration::from_secs(4), "{:?}", t.elapsed());
        // a silent child whose own child holds the pipes open past it:
        // stopped all the same, the run not held by the grandchild
        if !cfg!(windows) {
            let t = Instant::now();
            let e = run_with(&mut shell("sleep 5; echo late"), None, limit)
                .err()
                .unwrap();
            assert_eq!(e.kind(), io::ErrorKind::TimedOut, "{e}");
            assert!(t.elapsed() < Duration::from_secs(4), "{:?}", t.elapsed());
        }
    }
}
