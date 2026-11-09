//! The file operations a change is made of, and the ways they fail
//! (CORE-05, SC-07; Phase 10). [CLAUDE]
//!
//! A move is a rename. When the rename crosses volumes — a folder of the
//! root that is a mount point, or `_milked` made a link to another drive
//! to stage duplicates there — the system refuses it (EXDEV on Unix,
//! ERROR_NOT_SAME_DEVICE on Windows; both are `CrossesDevices` to std),
//! and the move becomes: copy, flush to disk, read the copy back and
//! compare its BLAKE3 with the original's, carry the times and the
//! read-only bit over, then delete the original. If the original cannot be
//! deleted, the copy is removed again and the original stays where it
//! was: a failed move never leaves two files or none.
//!
//! Failures are typed by what a person can do about them: a file in use
//! or refused by permission can be tried again after closing the program
//! holding it; a file the Recycle Bin refused can be moved into `_milked`
//! instead; a file that vanished needs nothing.

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::Path;

/// Why one file could not be acted on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActError {
    /// Gone since the scan.
    Vanished,
    /// Held open by another program (Windows sharing or lock violation).
    InUse,
    /// The system refused access.
    Permission,
    /// The move crossed volumes and the copy could not be completed or
    /// verified; the original is untouched.
    CrossDevice(String),
    /// The Recycle Bin refused it (no bin on that volume, a network drive,
    /// a path it cannot take).
    Bin(String),
    /// Anything else, in the system's words.
    Other(String),
}

impl ActError {
    pub fn from_io(e: &io::Error) -> ActError {
        // Windows: 32 sharing violation, 33 lock violation
        if matches!(e.raw_os_error(), Some(32) | Some(33)) && cfg!(windows) {
            return ActError::InUse;
        }
        match e.kind() {
            io::ErrorKind::NotFound => ActError::Vanished,
            io::ErrorKind::PermissionDenied => ActError::Permission,
            io::ErrorKind::CrossesDevices => ActError::CrossDevice(os_text(e)),
            _ => ActError::Other(os_text(e)),
        }
    }

    /// The words on the `failed` line.
    pub fn words(&self) -> String {
        match self {
            ActError::Vanished => "gone since the scan".into(),
            ActError::InUse => "in use by another program".into(),
            ActError::Permission => "permission denied".into(),
            ActError::CrossDevice(e) => format!("copy to the other volume failed: {e}"),
            ActError::Bin(e) => format!("the Recycle Bin refused it: {e}"),
            ActError::Other(e) => e.clone(),
        }
    }

    /// Worth trying again once whatever held the file lets go.
    pub fn retryable(&self) -> bool {
        matches!(self, ActError::InUse | ActError::Permission)
    }
}

/// The system's message without std's ` (os error N)` suffix.
pub fn os_text(e: &io::Error) -> String {
    let s = e.to_string();
    match s.find(" (os error") {
        Some(i) => s[..i].to_string(),
        None => s,
    }
}

/// Move `src` to `dst` (which must not exist). `Ok(true)` when the move
/// crossed volumes and was copied, verified and the original deleted.
pub fn move_file(src: &Path, dst: &Path) -> Result<bool, ActError> {
    if let Some(p) = dst.parent() {
        fs::create_dir_all(p).map_err(|e| ActError::from_io(&e))?;
    }
    match fs::rename(src, dst) {
        Ok(()) => Ok(false),
        Err(e) if e.kind() == io::ErrorKind::CrossesDevices => copy_move(src, dst).map(|_| true),
        Err(e) => Err(ActError::from_io(&e)),
    }
}

/// Copy, flush, verify, keep the times, delete the original.
fn copy_move(src: &Path, dst: &Path) -> Result<(), ActError> {
    let md = fs::metadata(src).map_err(|e| ActError::from_io(&e))?;
    let mut input = File::open(src).map_err(|e| ActError::from_io(&e))?;
    let mut out = File::options()
        .write(true)
        .create_new(true)
        .open(dst)
        .map_err(|e| ActError::CrossDevice(os_text(&e)))?;
    let fail = |msg: String| {
        let _ = fs::remove_file(dst);
        ActError::CrossDevice(msg)
    };
    // copy, hashing what was read
    let mut h = blake3::Hasher::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut n: u64 = 0;
    loop {
        let k = match input.read(&mut buf) {
            Ok(0) => break,
            Ok(k) => k,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => {
                drop(out);
                return Err(fail(os_text(&e)));
            }
        };
        h.update(&buf[..k]);
        if let Err(e) = out.write_all(&buf[..k]) {
            drop(out);
            return Err(fail(os_text(&e)));
        }
        n += k as u64;
    }
    if let Err(e) = out.sync_all() {
        drop(out);
        return Err(fail(os_text(&e)));
    }
    // the times as they were: the keeper policy reads them, and a person
    // expects the file unchanged
    let mut times = fs::FileTimes::new();
    if let Ok(t) = md.modified() {
        times = times.set_modified(t);
    }
    if let Ok(t) = md.accessed() {
        times = times.set_accessed(t);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::FileTimesExt;
        if let Ok(t) = md.created() {
            times = times.set_created(t);
        }
    }
    let _ = out.set_times(times);
    drop(out);
    let _ = fs::set_permissions(dst, md.permissions());
    // read the copy back
    let want = h.finalize();
    let got = match File::open(dst).and_then(|mut f| {
        let mut h2 = blake3::Hasher::new();
        let mut m: u64 = 0;
        loop {
            let k = f.read(&mut buf)?;
            if k == 0 {
                break;
            }
            h2.update(&buf[..k]);
            m += k as u64;
        }
        Ok((h2.finalize(), m))
    }) {
        Ok(x) => x,
        Err(e) => return Err(fail(os_text(&e))),
    };
    if got != (want, n) || n != md.len() {
        return Err(fail("the copy did not read back the same".into()));
    }
    // only now the original goes; if it cannot, the copy goes instead
    if let Err(e) = fs::remove_file(src) {
        let _ = fs::set_permissions(dst, {
            let mut p = md.permissions();
            #[allow(clippy::permissions_set_readonly_false)]
            p.set_readonly(false);
            p
        });
        let _ = fs::remove_file(dst);
        return Err(ActError::from_io(&e));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_are_typed_by_what_can_be_done() {
        let e = io::Error::from(io::ErrorKind::NotFound);
        assert_eq!(ActError::from_io(&e), ActError::Vanished);
        let e = io::Error::from(io::ErrorKind::PermissionDenied);
        assert_eq!(ActError::from_io(&e), ActError::Permission);
        assert!(ActError::Permission.retryable());
        assert!(ActError::InUse.retryable());
        assert!(!ActError::Vanished.retryable());
        assert!(!ActError::Bin("x".into()).retryable());
        let e = io::Error::from(io::ErrorKind::CrossesDevices);
        assert!(matches!(ActError::from_io(&e), ActError::CrossDevice(_)));
        if cfg!(windows) {
            assert_eq!(
                ActError::from_io(&io::Error::from_raw_os_error(32)),
                ActError::InUse
            );
        }
    }

    #[test]
    fn copy_move_verifies_keeps_times_and_removes_the_original() {
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("a.jpg");
        let dst = d.path().join("sub/a.jpg");
        let data: Vec<u8> = (0..3_000_000u32).map(|i| (i * 7 % 251) as u8).collect();
        fs::write(&src, &data).unwrap();
        let t = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_500_000_000);
        File::options()
            .write(true)
            .open(&src)
            .unwrap()
            .set_modified(t)
            .unwrap();
        fs::create_dir_all(dst.parent().unwrap()).unwrap();
        copy_move(&src, &dst).unwrap();
        assert!(!src.exists());
        assert_eq!(fs::read(&dst).unwrap(), data);
        assert_eq!(fs::metadata(&dst).unwrap().modified().unwrap(), t);
    }

    #[test]
    fn copy_move_never_overwrites_and_leaves_the_original() {
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("a.jpg");
        let dst = d.path().join("b.jpg");
        fs::write(&src, b"one").unwrap();
        fs::write(&dst, b"two").unwrap();
        assert!(copy_move(&src, &dst).is_err());
        assert_eq!(fs::read(&src).unwrap(), b"one");
        assert_eq!(fs::read(&dst).unwrap(), b"two");
        // a missing source is Vanished
        let gone = d.path().join("gone.jpg");
        assert_eq!(
            move_file(&gone, &d.path().join("x.jpg")),
            Err(ActError::Vanished)
        );
    }
}
