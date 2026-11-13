//! A file fetched and checked before it is kept (Phase 16, NF-09): the
//! deep tier's model, installed from the doctor. The transfer is curl's —
//! on Windows 10 and later, macOS and Linux alike, so `dedupe` carries no
//! TLS of its own and follows the system's proxies and redirects — into a
//! `.part` beside the destination; the `.part`'s growth is the progress
//! and the stall clock (`tools::stall()`); its size and SHA-256 are
//! compared with the published file's, and only then is it renamed into
//! place. Anything else removes the `.part`: nothing is left behind but
//! the right file. [CLAUDE]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Debug)]
pub enum FetchError {
    /// No curl on PATH.
    NoCurl,
    /// curl ended with this status (its words, when it gave any).
    Transfer(String),
    /// Nothing arrived for `tools::stall()`.
    Stalled,
    /// The file arrived at another size.
    Size {
        got: u64,
        want: u64,
    },
    /// The file arrived with another SHA-256.
    Hash {
        got: String,
    },
    Io(String),
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FetchError::NoCurl => write!(f, "curl not found on PATH"),
            FetchError::Transfer(s) => write!(f, "the download failed: {s}"),
            FetchError::Stalled => write!(
                f,
                "nothing arrived for {} s; stopped",
                crate::tools::stall().as_secs()
            ),
            FetchError::Size { got, want } => write!(
                f,
                "{} arrived, {} expected",
                crate::util::fmt_bytes(*got),
                crate::util::fmt_bytes(*want)
            ),
            FetchError::Hash { got } => {
                write!(
                    f,
                    "SHA-256 {}… is not the published file's",
                    &got[..12.min(got.len())]
                )
            }
            FetchError::Io(s) => write!(f, "{s}"),
        }
    }
}

/// The SHA-256 of a file, lowercase hex.
pub fn sha256_file(p: &Path) -> Option<String> {
    use sha2::{Digest, Sha256};
    let mut f = fs::File::open(p).ok()?;
    let mut h = Sha256::new();
    std::io::copy(&mut f, &mut h).ok()?;
    Some(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// The `.part` a download of `dest` is written to.
pub fn part_of(dest: &Path) -> PathBuf {
    let mut name = dest.file_name().unwrap_or_default().to_os_string();
    name.push(".part");
    dest.with_file_name(name)
}

/// `url` into `dest`, kept only at `size` bytes with SHA-256 `sha256`;
/// `progress` hears the bytes arrived so far, five times a second.
pub fn fetch_verified(
    url: &str,
    dest: &Path,
    size: u64,
    sha256: &str,
    progress: &mut dyn FnMut(u64),
) -> Result<(), FetchError> {
    let curl = crate::tools::on_path_pub("curl").ok_or(FetchError::NoCurl)?;
    if let Some(dir) = dest.parent() {
        fs::create_dir_all(dir).map_err(|e| FetchError::Io(format!("{}: {e}", dir.display())))?;
    }
    let part = part_of(dest);
    let _ = fs::remove_file(&part);
    let result = transfer(&curl, url, &part, progress).and_then(|()| {
        let got = fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
        if got != size {
            return Err(FetchError::Size { got, want: size });
        }
        match sha256_file(&part) {
            Some(h) if h == sha256 => Ok(()),
            Some(h) => Err(FetchError::Hash { got: h }),
            None => Err(FetchError::Io(format!("{}: unreadable", part.display()))),
        }
    });
    match result {
        Ok(()) => fs::rename(&part, dest).map_err(|e| {
            let _ = fs::remove_file(&part);
            FetchError::Io(format!("{}: {e}", dest.display()))
        }),
        Err(e) => {
            let _ = fs::remove_file(&part);
            Err(e)
        }
    }
}

/// curl writing `part`, watched by its size: stopped when it stops
/// growing for `tools::stall()`.
fn transfer(
    curl: &Path,
    url: &str,
    part: &Path,
    progress: &mut dyn FnMut(u64),
) -> Result<(), FetchError> {
    let mut child = Command::new(curl)
        // fail on an HTTP error, follow redirects, no meter of its own;
        // a host on this machine is never sent through a proxy
        .args([
            "-fsSL",
            "--retry",
            "2",
            "--noproxy",
            "localhost,127.0.0.1,::1",
            "-o",
        ])
        .arg(part)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| FetchError::Transfer(e.to_string()))?;
    let mut last = (0u64, Instant::now());
    let status = loop {
        if let Some(st) = child
            .try_wait()
            .map_err(|e| FetchError::Io(e.to_string()))?
        {
            break st;
        }
        let now = fs::metadata(part).map(|m| m.len()).unwrap_or(0);
        progress(now);
        if now != last.0 {
            last = (now, Instant::now());
        } else if last.1.elapsed() >= crate::tools::stall() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(FetchError::Stalled);
        }
        std::thread::sleep(Duration::from_millis(200));
    };
    progress(fs::metadata(part).map(|m| m.len()).unwrap_or(0));
    if status.success() {
        return Ok(());
    }
    let mut said = String::new();
    if let Some(mut e) = child.stderr.take() {
        use std::io::Read;
        let _ = e.read_to_string(&mut said);
    }
    let said = said.trim().trim_start_matches("curl: ").to_string();
    Err(FetchError::Transfer(if said.is_empty() {
        format!("curl {status}")
    } else {
        said
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// A server on this machine answering every request with `body`
    /// (`Content-Length` as given), `n` times.
    fn serve(body: Vec<u8>, n: usize) -> String {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!(
            "http://127.0.0.1:{}/sscd.onnx",
            l.local_addr().unwrap().port()
        );
        std::thread::spawn(move || {
            for s in l.incoming().take(n) {
                let Ok(mut s) = s else { continue };
                let mut req = [0u8; 4096];
                let _ = s.read(&mut req);
                let _ = s.write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    )
                    .as_bytes(),
                );
                let _ = s.write_all(&body);
            }
        });
        url
    }

    #[test]
    fn the_right_file_is_kept_and_a_short_or_wrong_one_leaves_nothing() {
        if crate::tools::on_path_pub("curl").is_none() {
            eprintln!("no curl on PATH; skipped");
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("models").join("sscd.onnx");
        let right: Vec<u8> = (0..300_000u32)
            .map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8)
            .collect();
        let sha = {
            use sha2::{Digest, Sha256};
            Sha256::digest(&right)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        };
        let size = right.len() as u64;
        let leftovers = |dir: &Path| -> Vec<String> {
            fs::read_dir(dir)
                .map(|r| {
                    r.flatten()
                        .map(|e| e.file_name().to_string_lossy().into_owned())
                        .collect()
                })
                .unwrap_or_default()
        };
        // truncated: refused, nothing left
        let short = serve(right[..100_000].to_vec(), 4);
        let e = fetch_verified(&short, &dest, size, &sha, &mut |_| {}).unwrap_err();
        assert!(matches!(e, FetchError::Size { got: 100_000, .. }), "{e}");
        assert!(leftovers(dest.parent().unwrap()).is_empty());
        // the same size, other bytes: refused, nothing left
        let mut other = right.clone();
        other[150_000] ^= 0xff;
        let wrong = serve(other, 4);
        let e = fetch_verified(&wrong, &dest, size, &sha, &mut |_| {}).unwrap_err();
        assert!(matches!(e, FetchError::Hash { .. }), "{e}");
        assert!(leftovers(dest.parent().unwrap()).is_empty());
        // the right file: kept, the .part gone, progress heard
        let good = serve(right.clone(), 4);
        let mut heard = 0;
        fetch_verified(&good, &dest, size, &sha, &mut |n| heard = heard.max(n)).unwrap();
        assert_eq!(fs::read(&dest).unwrap(), right);
        assert_eq!(heard, size);
        assert_eq!(
            leftovers(dest.parent().unwrap()),
            vec!["sscd.onnx".to_string()]
        );
        // a server that is not there: refused, nothing left
        let gone = serve(Vec::new(), 0);
        std::thread::sleep(Duration::from_millis(100));
        let dest2 = tmp.path().join("other").join("m.onnx");
        assert!(fetch_verified(&gone, &dest2, size, &sha, &mut |_| {}).is_err());
        assert!(leftovers(dest2.parent().unwrap()).is_empty());
    }
}
