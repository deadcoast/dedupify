//! `ffprobe`: a file's video stream, size, rate, duration and rotation,
//! and whether the two programs answer at all.

use std::io;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, PartialEq)]
pub struct Meta {
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    pub codec: String,
    pub fps: f64,
}

#[derive(Debug)]
pub enum VideoError {
    Spawn(io::Error),
    Probe(String),
    NoVideoStream,
    NoFrames,
    Io(io::Error),
    /// ffmpeg or ffprobe sent nothing for `tools::stall()` and was stopped
    /// (Phase 13, CORE-11).
    Stalled,
}

impl VideoError {
    /// The same error again, for a probe's error read by the hash pass
    /// (an `io::Error` keeps its kind and words).
    pub fn again(&self) -> VideoError {
        let io = |e: &io::Error| io::Error::new(e.kind(), e.to_string());
        match self {
            VideoError::Spawn(e) => VideoError::Spawn(io(e)),
            VideoError::Probe(s) => VideoError::Probe(s.clone()),
            VideoError::NoVideoStream => VideoError::NoVideoStream,
            VideoError::NoFrames => VideoError::NoFrames,
            VideoError::Io(e) => VideoError::Io(io(e)),
            VideoError::Stalled => VideoError::Stalled,
        }
    }

    /// From a clocked run: a stall, or the child would not start.
    pub fn from_run(e: io::Error) -> VideoError {
        if e.kind() == io::ErrorKind::TimedOut {
            VideoError::Stalled
        } else {
            VideoError::Spawn(e)
        }
    }
}

impl std::fmt::Display for VideoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VideoError::Spawn(e) => write!(f, "cannot start ffmpeg/ffprobe: {e}"),
            VideoError::Probe(s) => write!(f, "ffprobe: {s}"),
            VideoError::NoVideoStream => write!(f, "no video stream"),
            VideoError::NoFrames => write!(f, "no frames decoded"),
            VideoError::Io(e) => write!(f, "{e}"),
            VideoError::Stalled => write!(
                f,
                "ffmpeg sent nothing for {} s and was stopped",
                crate::tools::stall().as_secs()
            ),
        }
    }
}

/// Whether `ffmpeg` and `ffprobe` start and answer `-version`, wherever
/// `tools` found them (PATH, or where an installer put them).
pub fn tools_present() -> (bool, bool) {
    let ok = |mut c: Command| {
        crate::tools::run(c.arg("-version"), None).is_ok_and(|r| r.status.success())
    };
    (ok(crate::tools::ffmpeg()), ok(crate::tools::ffprobe()))
}

fn json_str<'a>(v: &'a serde_json::Value, keys: &[&str]) -> Option<&'a str> {
    let mut cur = v;
    for k in keys {
        cur = cur.get(*k)?;
    }
    cur.as_str()
}

fn json_u64(v: &serde_json::Value, keys: &[&str]) -> Option<u64> {
    let mut cur = v;
    for k in keys {
        cur = cur.get(*k)?;
    }
    cur.as_u64()
}

fn parse_rate(s: &str) -> f64 {
    match s.split_once('/') {
        Some((a, b)) => {
            let a: f64 = a.parse().unwrap_or(0.0);
            let b: f64 = b.parse().unwrap_or(1.0);
            if b == 0.0 {
                0.0
            } else {
                a / b
            }
        }
        None => s.parse().unwrap_or(0.0),
    }
}

/// `ffprobe` the first video stream.
pub fn probe(path: &Path) -> Result<Meta, VideoError> {
    let mut out = crate::tools::ffprobe();
    out.args([
        "-v",
        "error",
        "-select_streams",
        "v:0",
        "-show_entries",
        "stream=codec_name,width,height,r_frame_rate,avg_frame_rate,duration:format=duration",
        "-of",
        "json",
    ])
    .arg(path);
    let out = crate::tools::run(&mut out, None).map_err(VideoError::from_run)?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(VideoError::Probe(if err.is_empty() {
            "exit status".into()
        } else {
            err
        }));
    }
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).map_err(|e| VideoError::Probe(e.to_string()))?;
    let stream = v
        .get("streams")
        .and_then(|s| s.as_array())
        .and_then(|a| a.first())
        .ok_or(VideoError::NoVideoStream)?;
    let width = json_u64(stream, &["width"]).unwrap_or(0) as u32;
    let height = json_u64(stream, &["height"]).unwrap_or(0) as u32;
    if width == 0 || height == 0 {
        return Err(VideoError::NoVideoStream);
    }
    let codec = json_str(stream, &["codec_name"]).unwrap_or("?").to_string();
    let fps = json_str(stream, &["avg_frame_rate"])
        .map(parse_rate)
        .filter(|f| *f > 0.0)
        .or_else(|| json_str(stream, &["r_frame_rate"]).map(parse_rate))
        .unwrap_or(0.0);
    let duration = json_str(&v, &["format", "duration"])
        .and_then(|s| s.parse::<f64>().ok())
        .or_else(|| json_str(stream, &["duration"]).and_then(|s| s.parse::<f64>().ok()))
        .unwrap_or(0.0);
    Ok(Meta {
        duration,
        width,
        height,
        codec,
        fps,
    })
}

// ---------------------------------------------------------------------------
// TMK descriptor

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_rates() {
        assert!((parse_rate("30000/1001") - 29.97).abs() < 0.01);
        assert_eq!(parse_rate("25"), 25.0);
        assert_eq!(parse_rate("0/0"), 0.0);
    }
}
