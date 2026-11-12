//! Into the Recycle Bin, and never past it (Phase 13, found by TECH-11).
//!
//! On Windows the `trash` crate asks the shell with `FOF_WANTNUKEWARNING`:
//! a file the bin cannot hold — larger than its cap, on a volume whose bin
//! is set to delete at once — brings up the shell's "Delete File" dialog,
//! which asks to delete it permanently (CI run 43226616276: a 5 MB file
//! against a 1 MB bin; `purge` held there, the dialog its only window).
//! Here the shell is asked without that flag and with a progress sink:
//! before each file, the shell says whether the file will be recycled
//! (`TSF_DELETE_RECYCLE_IF_POSSIBLE`); when it will not, the sink stops it
//! (`E_ABORT`), and the file stays where it is, refused — the technique
//! Chromium's `MoveItemToTrash` uses. An abort cancels the operations
//! after it as well, so what is left is asked again, without the refused
//! file, until nothing is.
//!
//! Elsewhere the `trash` crate as before.

use std::path::{Path, PathBuf};

use crate::fsops::ActError;

/// Send `paths` to the bin; for each one still on disk afterwards, why.
pub fn to_bin(paths: &[&Path]) -> Vec<(PathBuf, ActError)> {
    imp::to_bin(paths)
}

#[cfg(not(windows))]
mod imp {
    use super::*;

    pub fn to_bin(paths: &[&Path]) -> Vec<(PathBuf, ActError)> {
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
    use windows::Win32::Foundation::E_ABORT;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_ALL, COINIT_APARTMENTTHREADED,
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
        // SAFETY: COM calls on this thread, initialised (apartment) here;
        // every interface lives to the end of the block
        unsafe {
            // S_FALSE (already initialised) and a different mode are both
            // fine: the thread has COM either way
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let op: IFileOperation = CoCreateInstance(&FileOperation, None, CLSCTX_ALL)?;
            // no FOF_WANTNUKEWARNING: the sink decides, not a dialog
            op.SetOperationFlags(FOF_NO_UI | FOF_ALLOWUNDO | FOFX_RECYCLEONDELETE)?;
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

    pub fn to_bin(paths: &[&Path]) -> Vec<(PathBuf, ActError)> {
        let mut out: Vec<(PathBuf, ActError)> = Vec::new();
        let mut pending: Vec<&Path> = paths.to_vec();
        // each round either ends the list or takes at least one file off it
        while !pending.is_empty() {
            let said = match once(&pending) {
                Ok(s) => s,
                Err(e) => {
                    for p in pending.iter().filter(|p| p.exists()) {
                        out.push((p.to_path_buf(), ActError::Bin(e.to_string())));
                    }
                    break;
                }
            };
            let mut settled = 0;
            let mut next: Vec<&Path> = Vec::new();
            for &p in &pending {
                let k = lower(p);
                if !p.exists() {
                    if said.nuked.contains(&k) {
                        // the sink exists so this cannot happen; if it ever
                        // does, the line says so rather than "binned"
                        out.push((
                            p.to_path_buf(),
                            ActError::Other("deleted without the Recycle Bin".into()),
                        ));
                    }
                    settled += 1;
                } else if said.refused.contains(&k) {
                    out.push((
                        p.to_path_buf(),
                        ActError::Bin(
                            "it cannot hold this file (too large for it, or set to delete at once): left in place"
                                .into(),
                        ),
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
