//! Into the Recycle Bin, and never past it (Phase 13, found by TECH-11).
//!
//! What CI measured on Windows Server 2025, a 5 MB file against a bin
//! capped at 1 MB (VERIFICATION § Phase 13): with the `trash` crate's flags
//! (`FOF_WANTNUKEWARNING`) the shell's "Delete File" dialog asks to delete
//! it permanently and `purge` waits on it; without that flag the shell
//! deletes it permanently and says so only afterwards (no item in the
//! bin), having said beforehand that it would recycle "if possible"
//! (`TSF_DELETE_RECYCLE_IF_POSSIBLE` set either way). So the shell cannot
//! be asked to recycle or fail. Here, on Windows:
//!
//! - a file on a drive with no Recycle Bin (removable, network, optical),
//!   on a drive whose bin is set to delete at once (`NukeOnDelete`, or the
//!   `NoRecycleFiles` policy), or larger than the drive's bin cap
//!   (`MaxCapacity`) is refused before the shell sees it;
//! - every other file gets a hard link in `_milked/.purging` before the
//!   shell's operation (no dialog asked for). If the file left without a
//!   bin item — its link count back to one — the link is renamed back to
//!   the file's place: refused, put back. A file that went to the bin has
//!   its link removed.
//!
//! Elsewhere the `trash` crate, whose freedesktop and macOS bins never
//! delete outright.

use std::path::{Path, PathBuf};

use crate::fsops::ActError;

/// Send `paths` to the bin; for each one not binned, why (still on disk,
/// or, where nothing could guard it, gone without the bin). `guard`: a
/// folder on the files' volume for the hard links (Windows; created and
/// removed here, its parent too when created here).
pub fn to_bin(paths: &[&Path], guard: Option<&Path>) -> Vec<(PathBuf, ActError)> {
    imp::to_bin(paths, guard)
}

/// `to_bin` without the checks before the shell (drive, settings, cap):
/// the hard-link guard alone, for the CI test that drives a file the bin
/// lets go. Not used by `purge`.
#[cfg(windows)]
#[doc(hidden)]
pub fn to_bin_guard_only(paths: &[&Path], guard: &Path) -> Vec<(PathBuf, ActError)> {
    imp::to_bin_with(paths, Some(guard), false)
}

/// What the shell says while it deletes `path` with `flags`
/// (`FILEOPERATION_FLAGS`), one line per callback: for the CI test that
/// measures how the bin treats a file it cannot hold. Not used by `purge`.
#[cfg(windows)]
#[doc(hidden)]
pub fn trace(path: &Path, flags: u32) -> Vec<String> {
    imp::trace(path, flags)
}

#[cfg(not(windows))]
mod imp {
    use super::*;

    pub fn to_bin(paths: &[&Path], _guard: Option<&Path>) -> Vec<(PathBuf, ActError)> {
        let res = trash::delete_all(paths);
        paths
            .iter()
            .filter(|p| p.exists())
            .map(|p| (p.to_path_buf(), error_of(&res)))
            .collect()
    }

    /// Why the bin left a file: a file held open is in use (and worth a
    /// retry); anything else is the bin refusing (and `_milked` is the
    /// fallback).
    fn error_of(res: &Result<(), trash::Error>) -> ActError {
        match res {
            // the freedesktop bin's file-system errors carry the io error
            #[cfg(all(unix, not(target_os = "macos"), not(target_os = "ios")))]
            Err(trash::Error::FileSystem { source, .. }) => ActError::from_io(source),
            Err(e) => ActError::Bin(e.to_string()),
            Ok(()) => ActError::Bin("the file is still there".into()),
        }
    }
}

#[cfg(windows)]
mod imp {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::os::windows::ffi::OsStrExt;

    use windows::core::{implement, ComObject, Ref, HRESULT, PCWSTR};
    use windows::Win32::Foundation::{ERROR_SUCCESS, E_ABORT, HANDLE};
    use windows::Win32::Storage::FileSystem::{
        GetDriveTypeW, GetFileInformationByHandle, GetVolumeNameForVolumeMountPointW,
        GetVolumePathNameW, BY_HANDLE_FILE_INFORMATION,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_ALL, COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::System::Registry::{
        RegGetValueW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD,
    };
    use windows::Win32::UI::Shell::{
        FileOperation, IFileOperation, IFileOperationProgressSink, IFileOperationProgressSink_Impl,
        IShellItem, SHCreateItemFromParsingName, FOFX_RECYCLEONDELETE, FOF_ALLOWUNDO, FOF_NO_UI,
        SIGDN_FILESYSPATH, TSF_DELETE_RECYCLE_IF_POSSIBLE,
    };

    /// What the shell said about each file, by its path in lower case.
    #[derive(Default)]
    struct Said {
        /// Would not be recycled: stopped before the shell touched it.
        refused: Vec<String>,
        /// The shell's own failure for the file (in use, access denied …).
        failed: HashMap<String, HRESULT>,
        /// Deleted, and no item in the bin for it. Never expected: the
        /// sink stops a file the bin will not take before its delete.
        nuked: Vec<String>,
        /// Every callback, for `trace`.
        events: Vec<String>,
    }

    #[implement(IFileOperationProgressSink)]
    struct Sink {
        said: RefCell<Said>,
    }

    fn key(item: Ref<'_, IShellItem>) -> String {
        item.as_ref()
            .and_then(|i| {
                // SAFETY: a live shell item; the returned string is freed here
                unsafe {
                    let name = i.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
                    let s = name.to_string().ok();
                    CoTaskMemFree(Some(name.0 as *const _));
                    s
                }
            })
            .map(|s| s.to_lowercase())
            .unwrap_or_default()
    }

    #[allow(non_snake_case)] // COM's method names
    impl IFileOperationProgressSink_Impl for Sink_Impl {
        fn StartOperations(&self) -> windows::core::Result<()> {
            Ok(())
        }
        fn FinishOperations(&self, _: HRESULT) -> windows::core::Result<()> {
            Ok(())
        }
        fn PreRenameItem(
            &self,
            _: u32,
            _: Ref<'_, IShellItem>,
            _: &PCWSTR,
        ) -> windows::core::Result<()> {
            Ok(())
        }
        fn PostRenameItem(
            &self,
            _: u32,
            _: Ref<'_, IShellItem>,
            _: &PCWSTR,
            _: HRESULT,
            _: Ref<'_, IShellItem>,
        ) -> windows::core::Result<()> {
            Ok(())
        }
        fn PreMoveItem(
            &self,
            _: u32,
            _: Ref<'_, IShellItem>,
            _: Ref<'_, IShellItem>,
            _: &PCWSTR,
        ) -> windows::core::Result<()> {
            Ok(())
        }
        fn PostMoveItem(
            &self,
            _: u32,
            _: Ref<'_, IShellItem>,
            _: Ref<'_, IShellItem>,
            _: &PCWSTR,
            _: HRESULT,
            _: Ref<'_, IShellItem>,
        ) -> windows::core::Result<()> {
            Ok(())
        }
        fn PreCopyItem(
            &self,
            _: u32,
            _: Ref<'_, IShellItem>,
            _: Ref<'_, IShellItem>,
            _: &PCWSTR,
        ) -> windows::core::Result<()> {
            Ok(())
        }
        fn PostCopyItem(
            &self,
            _: u32,
            _: Ref<'_, IShellItem>,
            _: Ref<'_, IShellItem>,
            _: &PCWSTR,
            _: HRESULT,
            _: Ref<'_, IShellItem>,
        ) -> windows::core::Result<()> {
            Ok(())
        }
        fn PreDeleteItem(
            &self,
            flags: u32,
            item: Ref<'_, IShellItem>,
        ) -> windows::core::Result<()> {
            self.said
                .borrow_mut()
                .events
                .push(format!("PreDeleteItem flags {flags:#x}"));
            // the shell says here whether this delete would be a recycle
            if flags & TSF_DELETE_RECYCLE_IF_POSSIBLE.0 as u32 == 0 {
                self.said.borrow_mut().refused.push(key(item));
                return Err(E_ABORT.into());
            }
            Ok(())
        }
        fn PostDeleteItem(
            &self,
            _: u32,
            item: Ref<'_, IShellItem>,
            result: HRESULT,
            recycled: Ref<'_, IShellItem>,
        ) -> windows::core::Result<()> {
            let mut said = self.said.borrow_mut();
            said.events.push(format!(
                "PostDeleteItem hr {:#010x} item in the bin {}",
                result.0 as u32,
                !recycled.is_null()
            ));
            // an abort cancels the files after it: not failures, they are
            // asked again (E_ABORT, COPYENGINE_E_USER_CANCELLED,
            // ERROR_CANCELLED)
            let cancelled = matches!(result.0 as u32, 0x8000_4004 | 0x8027_0000 | 0x8007_04C7);
            if cancelled {
                // asked again in the next round
            } else if result.is_err() {
                said.failed.insert(key(item), result);
            } else if recycled.is_null() {
                said.nuked.push(key(item));
            }
            Ok(())
        }
        fn PreNewItem(
            &self,
            _: u32,
            _: Ref<'_, IShellItem>,
            _: &PCWSTR,
        ) -> windows::core::Result<()> {
            Ok(())
        }
        fn PostNewItem(
            &self,
            _: u32,
            _: Ref<'_, IShellItem>,
            _: &PCWSTR,
            _: &PCWSTR,
            _: u32,
            _: HRESULT,
            _: Ref<'_, IShellItem>,
        ) -> windows::core::Result<()> {
            Ok(())
        }
        fn UpdateProgress(&self, _: u32, _: u32) -> windows::core::Result<()> {
            Ok(())
        }
        fn ResetTimer(&self) -> windows::core::Result<()> {
            Ok(())
        }
        fn PauseTimer(&self) -> windows::core::Result<()> {
            Ok(())
        }
        fn ResumeTimer(&self) -> windows::core::Result<()> {
            Ok(())
        }
    }

    /// The path as the shell parses it (no `\\?\`), NUL-terminated.
    fn shell_name(p: &Path) -> Vec<u16> {
        crate::util::strip_verbatim(p)
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect()
    }

    fn lower(p: &Path) -> String {
        crate::util::strip_verbatim(p)
            .to_string_lossy()
            .to_lowercase()
    }

    /// One shell operation over `paths`; what it said.
    fn once(paths: &[&Path]) -> windows::core::Result<Said> {
        // no FOF_WANTNUKEWARNING: the sink decides, not a dialog
        once_with(paths, (FOF_NO_UI | FOF_ALLOWUNDO | FOFX_RECYCLEONDELETE).0)
    }

    pub fn trace(path: &Path, flags: u32) -> Vec<String> {
        match once_with(&[path], flags) {
            Ok(said) => said.events,
            Err(e) => vec![format!("the operation failed: {e}")],
        }
    }

    fn once_with(paths: &[&Path], flags: u32) -> windows::core::Result<Said> {
        // SAFETY: COM calls on this thread, initialised (apartment) here;
        // every interface lives to the end of the block
        unsafe {
            // S_FALSE (already initialised) and a different mode are both
            // fine: the thread has COM either way
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let op: IFileOperation = CoCreateInstance(&FileOperation, None, CLSCTX_ALL)?;
            op.SetOperationFlags(windows::Win32::UI::Shell::FILEOPERATION_FLAGS(flags))?;
            let mut queued = 0;
            for p in paths {
                let item: IShellItem =
                    match SHCreateItemFromParsingName(PCWSTR(shell_name(p).as_ptr()), None) {
                        Ok(i) => i,
                        Err(_) => continue, // gone, or not a file the shell knows
                    };
                op.DeleteItem(&item, None)?;
                queued += 1;
            }
            if queued == 0 {
                return Ok(Said::default());
            }
            let sink = ComObject::new(Sink {
                said: RefCell::new(Said::default()),
            });
            let iface: IFileOperationProgressSink = sink.to_interface();
            let cookie = op.Advise(&iface)?;
            // an abort makes this an error; the sink says which files
            let _ = op.PerformOperations();
            let _ = op.Unadvise(cookie);
            Ok(sink.said.take())
        }
    }

    /// How one volume's bin is set.
    #[derive(Clone, Copy)]
    struct Bin {
        /// `GetDriveTypeW`: 3 fixed; 2 removable, 4 network, 5 optical,
        /// 6 RAM disk and 0/1 unknown have no bin.
        drive: u32,
        nuke: bool,
        cap_mb: Option<u32>,
    }

    fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
        s.encode_wide().chain(Some(0)).collect()
    }

    fn reg_dword(root: HKEY, key: &str, value: &str) -> Option<u32> {
        let (k, v) = (wide(key.as_ref()), wide(value.as_ref()));
        let mut data = 0u32;
        let mut len = 4u32;
        // SAFETY: NUL-terminated names; a 4-byte buffer for a DWORD
        let r = unsafe {
            RegGetValueW(
                root,
                PCWSTR(k.as_ptr()),
                PCWSTR(v.as_ptr()),
                RRF_RT_REG_DWORD,
                None,
                Some(&mut data as *mut u32 as *mut std::ffi::c_void),
                Some(&mut len),
            )
        };
        (r == ERROR_SUCCESS).then_some(data)
    }

    /// The volume `p` is on (its mount point) and how its bin is set.
    fn bin_of(p: &Path) -> Option<(String, Bin)> {
        let mut mount = [0u16; 512];
        let mut name = [0u16; 64];
        // SAFETY: NUL-terminated input, output buffers of the sizes given
        unsafe {
            GetVolumePathNameW(PCWSTR(shell_name(p).as_ptr()), &mut mount).ok()?;
            let drive = GetDriveTypeW(PCWSTR(mount.as_ptr()));
            let root = String::from_utf16_lossy(&mount[..mount.iter().position(|&c| c == 0)?]);
            let guid = GetVolumeNameForVolumeMountPointW(PCWSTR(mount.as_ptr()), &mut name)
                .ok()
                .map(|()| String::from_utf16_lossy(&name))
                .and_then(|n| Some(n[n.find('{')?..=n.find('}')?].to_string()));
            let key = guid.map(|g| {
                format!("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\BitBucket\\Volume\\{g}")
            });
            let get = |v: &str| {
                key.as_deref()
                    .and_then(|k| reg_dword(HKEY_CURRENT_USER, k, v))
            };
            Some((
                root,
                Bin {
                    drive,
                    nuke: get("NukeOnDelete").is_some_and(|n| n != 0),
                    cap_mb: get("MaxCapacity"),
                },
            ))
        }
    }

    /// Why the bin will not keep `p`, before the shell is asked.
    fn will_not_keep(p: &Path, bins: &mut HashMap<String, Bin>) -> Option<String> {
        const POLICY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Policies\\Explorer";
        if [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE]
            .iter()
            .any(|&h| reg_dword(h, POLICY, "NoRecycleFiles").is_some_and(|n| n != 0))
        {
            return Some("deleted files skip it on this computer (a policy): left in place".into());
        }
        let (root, bin) = bin_of(p)?;
        let bin = *bins.entry(root).or_insert(bin);
        if bin.drive != 3 {
            return Some("this drive has none: left in place".into());
        }
        if bin.nuke {
            return Some("this drive's bin is set to delete at once: left in place".into());
        }
        let size = std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
        match bin.cap_mb {
            Some(cap) if size > u64::from(cap) * 1024 * 1024 => Some(format!(
                "the file is larger than this drive's bin ({} MB): left in place",
                crate::util::fmt_count(u64::from(cap))
            )),
            _ => None,
        }
    }

    /// The names the file `p` has (its hard links), or None if unreadable.
    fn links(p: &Path) -> Option<u32> {
        use std::os::windows::io::AsRawHandle;
        let f = std::fs::File::open(p).ok()?;
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        // SAFETY: an open file's handle; a struct the call fills
        unsafe { GetFileInformationByHandle(HANDLE(f.as_raw_handle()), &mut info).ok()? };
        Some(info.nNumberOfLinks)
    }

    pub fn to_bin(paths: &[&Path], guard: Option<&Path>) -> Vec<(PathBuf, ActError)> {
        to_bin_with(paths, guard, true)
    }

    pub fn to_bin_with(
        paths: &[&Path],
        guard: Option<&Path>,
        check: bool,
    ) -> Vec<(PathBuf, ActError)> {
        let mut out: Vec<(PathBuf, ActError)> = Vec::new();
        let mut bins: HashMap<String, Bin> = HashMap::new();
        // the guard folder, and what of it this call made
        let mut made: Vec<PathBuf> = Vec::new();
        if let Some(g) = guard {
            for d in [g.parent(), Some(g)].into_iter().flatten() {
                if !d.exists() && std::fs::create_dir(d).is_ok() {
                    made.push(d.to_path_buf());
                }
            }
        }
        let mut pending: Vec<&Path> = Vec::new();
        for &p in paths {
            match will_not_keep(p, &mut bins).filter(|_| check) {
                Some(why) => out.push((p.to_path_buf(), ActError::Bin(why))),
                None => pending.push(p),
            }
        }
        let mut serial = 0u64;
        // each round either ends the list or takes at least one file off it
        while !pending.is_empty() {
            // a second name for each file while the shell works on it
            let linked: HashMap<&Path, PathBuf> = pending
                .iter()
                .filter_map(|&p| {
                    let g = guard?;
                    serial += 1;
                    let name = p.file_name()?.to_string_lossy();
                    let link = g.join(format!("{serial}-{name}"));
                    std::fs::hard_link(p, &link).ok().map(|()| (p, link))
                })
                .collect();
            let said = match once(&pending) {
                Ok(s) => s,
                Err(e) => {
                    for &p in &pending {
                        if let Some(l) = linked.get(p) {
                            let _ = std::fs::remove_file(l);
                        }
                        if p.exists() {
                            out.push((p.to_path_buf(), ActError::Bin(e.to_string())));
                        }
                    }
                    break;
                }
            };
            let mut settled = 0;
            let mut next: Vec<&Path> = Vec::new();
            for &p in &pending {
                let k = lower(p);
                let link = linked.get(p);
                if !p.exists() {
                    settled += 1;
                    match link {
                        // the bin has it, under its own name: our name goes
                        Some(l) if links(l).is_some_and(|n| n >= 2) => {
                            let _ = std::fs::remove_file(l);
                        }
                        // the shell let it go: our name puts it back
                        Some(l) => match std::fs::rename(l, p) {
                            Ok(()) => out.push((
                                p.to_path_buf(),
                                ActError::Bin(
                                    "it would not keep the file and Windows deleted it instead: put back where it was"
                                        .into(),
                                ),
                            )),
                            Err(e) => out.push((
                                p.to_path_buf(),
                                ActError::Other(format!(
                                    "the bin did not keep it; its bytes are at {} ({e})",
                                    crate::util::strip_verbatim(l).display()
                                )),
                            )),
                        },
                        None if said.nuked.contains(&k) => out.push((
                            p.to_path_buf(),
                            ActError::Other("deleted without the Recycle Bin".into()),
                        )),
                        None => {}
                    }
                    continue;
                }
                if let Some(l) = link {
                    let _ = std::fs::remove_file(l);
                }
                if said.refused.contains(&k) {
                    out.push((
                        p.to_path_buf(),
                        ActError::Bin("it would not take the file: left in place".into()),
                    ));
                    settled += 1;
                } else if let Some(hr) = said.failed.get(&k) {
                    out.push((p.to_path_buf(), error_of(*hr)));
                    settled += 1;
                } else {
                    next.push(p);
                }
            }
            if settled == 0 {
                // nothing moved and nothing said why: the shell gave up
                for p in next {
                    out.push((
                        p.to_path_buf(),
                        ActError::Bin("the shell left it without a reason".into()),
                    ));
                }
                break;
            }
            pending = next;
        }
        // the guard folder goes if empty, and its parent if made here
        for d in made.iter().rev() {
            let _ = std::fs::remove_dir(d);
        }
        if let Some(g) = guard {
            if !made.iter().any(|d| d == g) {
                let _ = std::fs::remove_dir(g);
            }
        }
        out
    }

    /// The shell's failure for one file, in the words `failed` lines use.
    fn error_of(hr: HRESULT) -> ActError {
        // sharing violation, lock violation: in use, worth a retry
        match hr.0 as u32 {
            0x8007_0020 | 0x8007_0021 => ActError::InUse,
            _ => ActError::Bin(windows::core::Error::from(hr).message()),
        }
    }
}
