//! End-to-end: the built binary on a generated fixture tree.
//!
//! Runs `find`, `milk`, `--undo`, `purge --dry-run` and the error paths,
//! and checks the tree after each step. The real `purge` + `--undo` through
//! the OS bin is `#[ignore]`d so an ordinary `cargo test` never touches the
//! machine's Recycle Bin; run it with `cargo test -- --ignored`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_dedupe"))
}

fn put(root: &Path, rel: &str, bytes: &[u8]) {
    put_at(root, rel, bytes, 1_622_505_600); // 2021-06-01
}

fn put_at(root: &Path, rel: &str, bytes: &[u8], mtime_unix: u64) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(&p, bytes).unwrap();
    let f = fs::File::options().write(true).open(&p).unwrap();
    f.set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(mtime_unix))
        .unwrap();
}

fn blob(seed: u8, len: usize) -> Vec<u8> {
    (0..len)
        .map(|i| ((i as u64 * 2_654_435_761u64) >> 7) as u8 ^ seed)
        .collect()
}

struct Fx {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    journal: PathBuf,
}

fn fixture() -> Fx {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("fx");
    let journal = tmp.path().join("journal");
    fs::create_dir_all(&root).unwrap();
    let img = blob(1, 300 * 1024);
    put_at(&root, "2019/trip/IMG_0041.jpg", &img, 1_560_297_600); // 2019-06-12, the original
    put(&root, "backup/IMG_0041.jpg", &img);
    put(&root, "backup/old/IMG_0041 (1).jpg", &img);
    let vid = blob(2, 2 * 1024 * 1024 + 777);
    put(&root, "clips/a.mp4", &vid);
    put(&root, "clips/copy/a.mp4", &vid);
    let note = b"meeting notes\n".repeat(40);
    put(&root, "notes.txt", &note);
    put(&root, "notes (copy).txt", &note);
    let base = blob(3, 300 * 1024);
    let mut mid = base.clone();
    mid[150 * 1024] ^= 0xff;
    put(&root, "decoys/base.bin", &base);
    put(&root, "decoys/mid_differs.bin", &mid);
    put(&root, "unique/one.png", &blob(4, 50_000));
    put(&root, "empty.txt", b"");
    Fx {
        _tmp: tmp,
        root,
        journal,
    }
}

// ---------------------------------------------------------------------------
// near-duplicate fixture: procedural photographs
// ---------------------------------------------------------------------------

fn photo(seed: u32, w: u32, h: u32) -> image::RgbImage {
    let mut state = (seed as u64 * 0x9E37_79B9_7F4A_7C15) | 1;
    let mut rnd = move || {
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        (state.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40) as f32 / (1u64 << 24) as f32
    };
    // two octaves of smooth value noise plus a few shapes
    let cells = [4u32, 12];
    let grids: Vec<(u32, Vec<f32>)> = cells
        .iter()
        .map(|&c| (c, (0..(c + 1) * (c + 1)).map(|_| rnd()).collect()))
        .collect();
    let shapes: Vec<(f32, f32, f32, f32, [f32; 3])> = (0..4)
        .map(|_| {
            (
                rnd(),
                rnd(),
                0.1 + 0.2 * rnd(),
                0.1 + 0.2 * rnd(),
                [rnd(), rnd(), rnd()],
            )
        })
        .collect();
    let tint = [0.6 + 0.4 * rnd(), 0.6 + 0.4 * rnd(), 0.6 + 0.4 * rnd()];
    image::ImageBuffer::from_fn(w, h, |x, y| {
        let fx = x as f32 / w as f32;
        let fy = y as f32 / h as f32;
        let mut v = 0.0;
        let mut amp = 1.0;
        let mut tot = 0.0;
        for (c, g) in &grids {
            let gx = fx * *c as f32;
            let gy = fy * *c as f32;
            let (x0, y0) = (gx.floor() as u32, gy.floor() as u32);
            let (tx, ty) = (gx - x0 as f32, gy - y0 as f32);
            let at = |xx: u32, yy: u32| g[(yy.min(*c) * (c + 1) + xx.min(*c)) as usize];
            v += amp
                * (at(x0, y0) * (1.0 - tx) * (1.0 - ty)
                    + at(x0 + 1, y0) * tx * (1.0 - ty)
                    + at(x0, y0 + 1) * (1.0 - tx) * ty
                    + at(x0 + 1, y0 + 1) * tx * ty);
            tot += amp;
            amp *= 0.5;
        }
        v /= tot;
        let mut px = [v * tint[0], v * tint[1], v * tint[2]];
        for (cx, cy, rx, ry, col) in &shapes {
            let dx = (fx - cx) / rx;
            let dy = (fy - cy) / ry;
            let d = dx * dx + dy * dy;
            if d < 1.0 {
                let a = 0.7 * (1.0 - d).sqrt();
                for k in 0..3 {
                    px[k] = px[k] * (1.0 - a) + col[k] * a;
                }
            }
        }
        image::Rgb([
            (px[0] * 255.0) as u8,
            (px[1] * 255.0) as u8,
            (px[2] * 255.0) as u8,
        ])
    })
}

fn png_bytes(img: &image::RgbImage) -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(img.clone())
        .write_to(&mut buf, image::ImageFormat::Png)
        .unwrap();
    buf.into_inner()
}

fn jpeg_bytes(img: &image::RgbImage, q: u8) -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, q)
        .encode_image(img)
        .unwrap();
    buf.into_inner()
}

/// The same JPEG with an EXIF APP1 segment carrying `DateTimeOriginal`
/// (the camera's exposure time) inserted after the SOI marker.
fn with_capture_time(jpeg: &[u8], datetime: &str) -> Vec<u8> {
    assert_eq!(&jpeg[..2], &[0xFF, 0xD8]);
    let mut tiff: Vec<u8> = Vec::new();
    tiff.extend_from_slice(b"MM\x00\x2A\x00\x00\x00\x08");
    let exif_ifd_off: u32 = 8 + 2 + 12 + 4;
    tiff.extend_from_slice(&1u16.to_be_bytes());
    tiff.extend_from_slice(&0x8769u16.to_be_bytes());
    tiff.extend_from_slice(&4u16.to_be_bytes());
    tiff.extend_from_slice(&1u32.to_be_bytes());
    tiff.extend_from_slice(&exif_ifd_off.to_be_bytes());
    tiff.extend_from_slice(&0u32.to_be_bytes());
    let dt = format!("{datetime}\0");
    let data_off = exif_ifd_off + 2 + 12 + 4;
    tiff.extend_from_slice(&1u16.to_be_bytes());
    tiff.extend_from_slice(&0x9003u16.to_be_bytes());
    tiff.extend_from_slice(&2u16.to_be_bytes());
    tiff.extend_from_slice(&(dt.len() as u32).to_be_bytes());
    tiff.extend_from_slice(&data_off.to_be_bytes());
    tiff.extend_from_slice(&0u32.to_be_bytes());
    tiff.extend_from_slice(dt.as_bytes());
    let mut app1 = b"Exif\0\0".to_vec();
    app1.extend_from_slice(&tiff);
    let mut out = vec![0xFF, 0xD8, 0xFF, 0xE1];
    out.extend_from_slice(&((app1.len() + 2) as u16).to_be_bytes());
    out.extend_from_slice(&app1);
    out.extend_from_slice(&jpeg[2..]);
    out
}

/// Adds the photo set to a fixture root. Returns the exact copy's bytes so a
/// test can check what moved.
fn add_photos(root: &Path) {
    let beach = photo(21, 1000, 750);
    put_at(root, "photos/beach.png", &png_bytes(&beach), 1_560_297_600); // 2019: the original
    let small = image::imageops::resize(&beach, 500, 375, image::imageops::FilterType::Triangle);
    put(root, "photos/beach_small.jpg", &jpeg_bytes(&small, 75));
    let export = jpeg_bytes(&beach, 90);
    put(root, "photos/export/beach.jpg", &export);
    put(root, "photos/export/beach (copy).jpg", &export); // byte-identical to export/beach.jpg
    put(
        root,
        "photos/export/beach_mirror.png",
        &png_bytes(&image::imageops::flip_horizontal(&beach)),
    );
    // a 5 % border crop: the two 64-bit hashes group it (d 10, p 6), PDQ does not confirm it (q 32)
    let cropped = image::imageops::crop_imm(&beach, 25, 19, 950, 712).to_image();
    put(
        root,
        "photos/export/beach_crop.jpg",
        &jpeg_bytes(&cropped, 90),
    );
    put(root, "photos/forest.png", &png_bytes(&photo(22, 800, 600)));
    let flat: image::RgbImage =
        image::ImageBuffer::from_pixel(300, 200, image::Rgb([128, 128, 128]));
    put(root, "photos/flat.png", &png_bytes(&flat));
}

fn fixture_with_photos() -> Fx {
    let fx = fixture();
    add_photos(&fx.root);
    fx
}

/// Writes the photo fixture into `$DEDUPE_FIXTURE_DIR` for looking at the
/// output by hand: `DEDUPE_FIXTURE_DIR=/some/dir cargo test --test e2e -- --ignored write_fixture`.
#[test]
#[ignore]
fn write_fixture() {
    let dir = std::env::var("DEDUPE_FIXTURE_DIR").expect("DEDUPE_FIXTURE_DIR");
    let root = PathBuf::from(dir);
    fs::create_dir_all(&root).unwrap();
    add_photos(&root);
}

/// Larger photo set for timing the near pass: 300 originals (1600×1200
/// JPEG), 60 downscaled copies, 40 re-encoded copies, 20 mirrors.
/// `DEDUPE_FIXTURE_DIR=/some/dir cargo test --release --test e2e -- --ignored write_big_photo_fixture`
#[test]
#[ignore]
fn write_big_photo_fixture() {
    let dir = std::env::var("DEDUPE_FIXTURE_DIR").expect("DEDUPE_FIXTURE_DIR");
    let root = PathBuf::from(dir);
    fs::create_dir_all(&root).unwrap();
    for i in 0..300u32 {
        let img = photo(1000 + i, 1600, 1200);
        put(
            &root,
            &format!("cam/{:02}/DSC_{:04}.jpg", i % 12, i),
            &jpeg_bytes(&img, 88),
        );
        if i % 5 == 0 {
            let small =
                image::imageops::resize(&img, 800, 600, image::imageops::FilterType::Triangle);
            put(
                &root,
                &format!("phone/DSC_{:04}.jpg", i),
                &jpeg_bytes(&small, 70),
            );
        }
        if i % 7 == 0 {
            put(
                &root,
                &format!("exports/DSC_{:04}.jpg", i),
                &jpeg_bytes(&img, 60),
            );
        }
        if i % 15 == 0 {
            put(
                &root,
                &format!("exports/DSC_{:04}_m.jpg", i),
                &jpeg_bytes(&image::imageops::flip_horizontal(&img), 88),
            );
        }
    }
}

fn run(fx: &Fx, args: &[&str]) -> (i32, String, String) {
    let out = Command::new(bin())
        .args(args)
        .current_dir(&fx.root)
        .env("DEDUPE_JOURNAL_DIR", &fx.journal)
        .env("NO_COLOR", "1")
        // wide enough that no path is shortened; layout at narrow widths is
        // the audit's business (VERIFICATION.md, Phase 5), not these tests'
        .env("COLUMNS", "200")
        .output()
        .expect("spawn dedupe");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// Run on a terminal without colour and return the text a person saw.
#[cfg(any(unix, windows))]
fn run_tty(fx: &Fx, args: &[&str], keys: &[&str], columns: u32) -> Option<String> {
    run_tty_raw(fx, args, keys, columns, false).map(|raw| plain_text(&raw))
}

/// Run on a pseudo-terminal through `script(1)` with `keys` typed after
/// `delay`, so the workspace bar appears and answers; Unix only, and only
/// where `script` exists. Returns what the terminal received, escapes and
/// all, or None when it cannot run. `colour` leaves `NO_COLOR` unset.
#[cfg(unix)]
fn run_tty_raw(
    fx: &Fx,
    args: &[&str],
    keys: &[&str],
    columns: u32,
    colour: bool,
) -> Option<String> {
    use std::io::Write;
    use std::process::Stdio;
    if Command::new("script").arg("--version").output().is_err() {
        return None;
    }
    let cmd = std::iter::once(bin().to_string_lossy().into_owned())
        .chain(args.iter().map(|a| a.to_string()))
        .map(|a| format!("'{}'", a.replace('\'', "'\\''")))
        .collect::<Vec<_>>()
        .join(" ");
    let mut script = Command::new("script");
    if colour {
        script.env_remove("NO_COLOR");
    } else {
        script.env("NO_COLOR", "1");
    }
    let mut child = script
        .args(["-qfec", &cmd, "/dev/null"])
        .current_dir(&fx.root)
        .env("DEDUPE_JOURNAL_DIR", &fx.journal)
        .env("TERM", "xterm")
        .env("COLUMNS", columns.to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdin = child.stdin.take()?;
    let keys: Vec<String> = keys.iter().map(|k| k.to_string()).collect();
    std::thread::spawn(move || {
        // the result takes a moment to print; each key a moment to act on
        std::thread::sleep(std::time::Duration::from_millis(2500));
        for k in keys {
            let _ = stdin.write_all(k.as_bytes());
            let _ = stdin.flush();
            std::thread::sleep(std::time::Duration::from_millis(1500));
        }
    });
    let out = child.wait_with_output().ok()?;
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The same on Windows (Phase 12, TECH-04): `dedupe` started in a pseudo
/// console (`CreatePseudoConsole`, Windows 10 1809 and later) of
/// `columns` × 50 cells, `keys` written to its input after the same
/// delays, everything it draws read from its output. The environment is
/// this process's with the fixture's journal folder, `NO_COLOR` (unless
/// `colour`) and
/// `COLUMNS` added, passed whole so no other test sees them. Returns None
/// when the pseudo console cannot be made.
#[cfg(windows)]
fn run_tty_raw(
    fx: &Fx,
    args: &[&str],
    keys: &[&str],
    columns: u32,
    colour: bool,
) -> Option<String> {
    use std::ffi::c_void;
    use std::fs::File;
    use std::io::{Read, Write};
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::{FromRawHandle, RawHandle};
    use std::ptr::{null, null_mut};

    type Handle = *mut c_void;
    #[repr(C)]
    struct Coord {
        x: i16,
        y: i16,
    }
    #[repr(C)]
    struct StartupInfoW {
        cb: u32,
        reserved: *mut u16,
        desktop: *mut u16,
        title: *mut u16,
        x: u32,
        y: u32,
        x_size: u32,
        y_size: u32,
        x_count_chars: u32,
        y_count_chars: u32,
        fill_attribute: u32,
        flags: u32,
        show_window: u16,
        reserved2_len: u16,
        reserved2: *mut u8,
        std_input: Handle,
        std_output: Handle,
        std_error: Handle,
    }
    #[repr(C)]
    struct StartupInfoExW {
        startup_info: StartupInfoW,
        attribute_list: *mut c_void,
    }
    #[repr(C)]
    struct ProcessInformation {
        process: Handle,
        thread: Handle,
        process_id: u32,
        thread_id: u32,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn CreatePipe(
            read: *mut Handle,
            write: *mut Handle,
            attrs: *const c_void,
            size: u32,
        ) -> i32;
        fn CreatePseudoConsole(
            size: Coord,
            input: Handle,
            output: Handle,
            flags: u32,
            console: *mut Handle,
        ) -> i32;
        fn ClosePseudoConsole(console: Handle);
        fn InitializeProcThreadAttributeList(
            list: *mut c_void,
            count: u32,
            flags: u32,
            size: *mut usize,
        ) -> i32;
        fn UpdateProcThreadAttribute(
            list: *mut c_void,
            flags: u32,
            attribute: usize,
            value: *const c_void,
            size: usize,
            previous: *mut c_void,
            returned: *mut usize,
        ) -> i32;
        fn DeleteProcThreadAttributeList(list: *mut c_void);
        fn CreateProcessW(
            application: *const u16,
            command_line: *mut u16,
            process_attrs: *const c_void,
            thread_attrs: *const c_void,
            inherit: i32,
            flags: u32,
            environment: *const c_void,
            directory: *const u16,
            startup: *const StartupInfoExW,
            info: *mut ProcessInformation,
        ) -> i32;
        fn WaitForSingleObject(handle: Handle, ms: u32) -> u32;
        fn TerminateProcess(handle: Handle, code: u32) -> i32;
        fn CloseHandle(handle: Handle) -> i32;
    }
    const PSEUDOCONSOLE: usize = 0x0002_0016;
    const EXTENDED_STARTUPINFO_PRESENT: u32 = 0x0008_0000;
    const CREATE_UNICODE_ENVIRONMENT: u32 = 0x0000_0400;
    const WAIT_OBJECT_0: u32 = 0;
    const STARTF_USESTDHANDLES: u32 = 0x0000_0100;
    const INVALID_HANDLE: Handle = -1isize as Handle;

    let wide = |s: &std::ffi::OsStr| {
        s.encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<u16>>()
    };
    // the command line: every argument quoted the way CreateProcess parses it
    let quote = |a: &str| format!("\"{}\"", a.replace('"', "\\\""));
    let mut line: Vec<u16> = std::iter::once(bin().to_string_lossy().into_owned())
        .chain(args.iter().map(|a| a.to_string()))
        .map(|a| quote(&a))
        .collect::<Vec<_>>()
        .join(" ")
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    // the environment block: KEY=VALUE\0 … \0, this process's plus three
    let mut env: Vec<(String, String)> = std::env::vars()
        .filter(|(k, _)| {
            !["DEDUPE_JOURNAL_DIR", "NO_COLOR", "COLUMNS"].contains(&k.to_uppercase().as_str())
        })
        .collect();
    env.push((
        "DEDUPE_JOURNAL_DIR".into(),
        fx.journal.to_string_lossy().into_owned(),
    ));
    if !colour {
        env.push(("NO_COLOR".into(), "1".into()));
    }
    env.push(("COLUMNS".into(), columns.to_string()));
    env.sort_by_key(|(k, _)| k.to_uppercase());
    let mut block: Vec<u16> = Vec::new();
    for (k, v) in &env {
        block.extend(format!("{k}={v}").encode_utf16());
        block.push(0);
    }
    block.push(0);
    let dir = wide(fx.root.as_os_str());

    // SAFETY: plain Win32 calls on handles this function creates and
    // closes; buffers outlive the calls that read them; the attribute
    // list is sized by the first call and freed after CreateProcessW.
    unsafe {
        let (mut in_read, mut in_write, mut out_read, mut out_write) =
            (null_mut(), null_mut(), null_mut(), null_mut());
        if CreatePipe(&mut in_read, &mut in_write, null(), 0) == 0
            || CreatePipe(&mut out_read, &mut out_write, null(), 0) == 0
        {
            return None;
        }
        let mut console: Handle = null_mut();
        let size = Coord {
            x: columns as i16,
            y: 50,
        };
        if CreatePseudoConsole(size, in_read, out_write, 0, &mut console) != 0 {
            return None;
        }
        // the pseudo console holds its own copies of its two ends
        CloseHandle(in_read);
        CloseHandle(out_write);
        let mut bytes = 0usize;
        InitializeProcThreadAttributeList(null_mut(), 1, 0, &mut bytes);
        let mut list = vec![0u8; bytes];
        let list_ptr = list.as_mut_ptr() as *mut c_void;
        if InitializeProcThreadAttributeList(list_ptr, 1, 0, &mut bytes) == 0
            || UpdateProcThreadAttribute(
                list_ptr,
                0,
                PSEUDOCONSOLE,
                console,
                std::mem::size_of::<Handle>(),
                null_mut(),
                null_mut(),
            ) == 0
        {
            ClosePseudoConsole(console);
            return None;
        }
        let mut startup: StartupInfoExW = std::mem::zeroed();
        startup.startup_info.cb = std::mem::size_of::<StartupInfoExW>() as u32;
        startup.attribute_list = list_ptr;
        // no standard handles of ours: without this the child inherits
        // this process's when they are redirected (a pipe under cargo in
        // CI, a log under the launcher) and draws there, not in the
        // pseudo console (found in the first CI run, 2026-11-11)
        startup.startup_info.flags = STARTF_USESTDHANDLES;
        startup.startup_info.std_input = INVALID_HANDLE;
        startup.startup_info.std_output = INVALID_HANDLE;
        startup.startup_info.std_error = INVALID_HANDLE;
        let mut info: ProcessInformation = std::mem::zeroed();
        let started = CreateProcessW(
            null(),
            line.as_mut_ptr(),
            null(),
            null(),
            0,
            EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT,
            block.as_ptr() as *const c_void,
            dir.as_ptr(),
            &startup,
            &mut info,
        );
        DeleteProcThreadAttributeList(list_ptr);
        if started == 0 {
            ClosePseudoConsole(console);
            CloseHandle(in_write);
            CloseHandle(out_read);
            return None;
        }
        CloseHandle(info.thread);
        // read everything it draws, on a thread of its own: a pseudo
        // console whose output is not drained stops drawing
        let mut output = File::from_raw_handle(out_read as RawHandle);
        let reader = std::thread::spawn(move || {
            let mut all = Vec::new();
            let _ = output.read_to_end(&mut all);
            all
        });
        let mut input = File::from_raw_handle(in_write as RawHandle);
        let keys: Vec<String> = keys.iter().map(|k| k.to_string()).collect();
        let writer = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(2500));
            for k in keys {
                let _ = input.write_all(k.as_bytes());
                let _ = input.flush();
                std::thread::sleep(std::time::Duration::from_millis(1500));
            }
            input
        });
        if WaitForSingleObject(info.process, 120_000) != WAIT_OBJECT_0 {
            TerminateProcess(info.process, 1);
            WaitForSingleObject(info.process, 10_000);
        }
        CloseHandle(info.process);
        let input = writer.join().ok();
        // closing the pseudo console ends its output, and the reader
        ClosePseudoConsole(console);
        drop(input);
        let all = reader.join().ok()?;
        Some(String::from_utf8_lossy(&all).into_owned())
    }
}

/// Terminal output without its control sequences and carriage returns:
/// the text a person saw, in the order it was drawn. CSI sequences end at
/// their final byte (`@` to `~`); OSC ones (a window title, which the
/// Windows pseudo console sends) at BEL or ESC `\`. Two CSI sequences
/// stand for what they show: a cursor moved forward (`CSI n C`) is that
/// many blanks, and one moved to a row and column (`CSI r;c H`) starts a
/// line — the Windows pseudo console draws a run of blanks and a new row
/// that way rather than with spaces and line ends.
fn plain_text(text: &str) -> String {
    let mut plain = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            match chars.peek() {
                Some('[') => {
                    chars.next();
                    let mut params = String::new();
                    for n in chars.by_ref() {
                        if ('@'..='~').contains(&n) {
                            match n {
                                'C' => {
                                    let k = params.parse::<usize>().unwrap_or(1).max(1);
                                    plain.extend(std::iter::repeat_n(' ', k));
                                }
                                'H' | 'f' if !plain.is_empty() && !plain.ends_with('\n') => {
                                    plain.push('\n')
                                }
                                _ => {}
                            }
                            break;
                        }
                        params.push(n);
                    }
                }
                Some(']') => {
                    chars.next();
                    while let Some(n) = chars.next() {
                        if n == '\u{7}' {
                            break;
                        }
                        if n == '\u{1b}' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                }
                Some(_) => {
                    chars.next(); // a two-byte sequence (ESC 7, ESC =)
                }
                None => {}
            }
            continue;
        }
        if c != '\r' {
            plain.push(c);
        }
    }
    plain
}

fn files(root: &Path) -> Vec<String> {
    let mut v: Vec<String> = walkdir::WalkDir::new(root)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file())
        .map(|e| {
            e.path()
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect();
    v.sort();
    v
}

/// Journals written under `dir` (one subfolder per root). `recent.json`,
/// the last folder looked at (QOL-03), sits beside the subfolders and is
/// not one.
fn journals(dir: &Path) -> usize {
    walkdir::WalkDir::new(dir)
        .min_depth(2)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file() && e.path().extension().is_some_and(|x| x == "json"))
        .count()
}

#[test]
fn find_reports_groups_and_stages() {
    let fx = fixture();
    let (code, out, _) = run(&fx, &["find", "."]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("3 groups   4 duplicates"), "{out}");
    assert!(out.contains("GROUP 001   2 copies   2.00 MB each"), "{out}");
    assert!(
        out.contains("keep  2019/trip/IMG_0041.jpg")
            || out.contains("keep  2019\\trip\\IMG_0041.jpg"),
        "{out}"
    );
    assert!(out.contains("9 share a size"), "{out}");
    assert!(out.contains("9 share a sample"), "{out}");
    assert!(out.contains("7 confirmed"), "{out}");
    assert!(
        !out.contains("mid_differs"),
        "sample-equal, content-different file must not be a duplicate\n{out}"
    );
    assert!(out.contains("next   dedupe milk . "), "{out}");
    // nothing written to the tree, no journal
    assert_eq!(journals(&fx.journal), 0, "find journals nothing");
    assert!(!fx.root.join("_milked").exists());
    // the one thing find keeps: the folder it looked at, in the app
    // directory, for the bare menu's `again` line (QOL-03)
    let recent: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(fx.journal.join("recent.json")).unwrap()).unwrap();
    assert_eq!(
        Path::new(recent["root"].as_str().unwrap()).file_name(),
        fx.root.file_name(),
        "{recent}"
    );
    assert_eq!(recent["recurse"], serde_json::Value::Bool(true), "{recent}");
    assert_eq!(recent["kind"], "", "{recent}");
}

#[test]
fn find_top_level_only_without_dot() {
    let fx = fixture();
    let (code, out, _) = run(&fx, &["find"]);
    assert_eq!(code, 0);
    assert!(
        out.contains("1 group   1 duplicate   560 B reclaimable"),
        "{out}"
    );
}

#[test]
fn quick_search_reports_sample_tier() {
    let fx = fixture();
    let (code, out, _) = run(&fx, &["find", ".", "-q"]);
    assert_eq!(code, 0);
    assert!(out.contains("LIKELY 00"), "{out}");
    assert!(
        out.contains("mid_differs"),
        "quick search is sample-level by design\n{out}"
    );
    assert!(!out.contains("HASH "), "{out}");
    // the follow-up commands are the full form: moving and binning run on
    // full evidence, the quick flag is for looking
    assert!(out.contains("next   dedupe milk . "), "{out}");
    assert!(!out.contains("milk . --quick-search"), "{out}");
}

#[test]
fn quick_purge_still_verifies_bytes() {
    let fx = fixture();
    let before = files(&fx.root);
    let (code, out, _) = run(&fx, &["purge", ".", "-q", "-n"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("VERIFY"), "{out}");
    assert!(
        out.contains("kept      decoys/mid_differs.bin")
            || out.contains("kept      decoys\\mid_differs.bin"),
        "the sample-only match must be caught by the byte check\n{out}"
    );
    assert!(out.contains("bytes differ from keeper"), "{out}");
    assert!(out.contains("4 would bin"), "{out}");
    assert_eq!(files(&fx.root), before);
}

#[test]
fn one_kind_at_a_time() {
    let fx = fixture();
    let (code, _, err) = run(&fx, &["find", ".", "img", "vid"]);
    assert_eq!(code, 1);
    assert!(err.contains("one kind at a time"), "{err}");
}

#[test]
fn purge_runs_only_the_pass_its_kind_can_use() {
    let fx = fixture_with_photos();
    let (code, out, _) = run(&fx, &["purge", ".", "img", "-n"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("DECODE"), "{out}");
    assert!(
        !out.contains("FRAMES"),
        "no video pass over an image-only scan\n{out}"
    );
    let (code, out, _) = run(&fx, &["purge", ".", "vid", "-n"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        !out.contains("DECODE"),
        "no image pass over a video-only scan\n{out}"
    );
}

#[test]
fn empty_scan_prints_no_empty_stages() {
    let fx = fixture();
    fs::create_dir_all(fx.root.join("nothing")).unwrap();
    let (code, out, _) = run(&fx, &["find", "nothing", "."]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("SCAN"), "{out}");
    assert!(!out.contains("SIZE"), "{out}");
    assert!(out.contains("no duplicates   0 files"), "{out}");
    // one file: SIZE says nothing shares, and the sample/hash lines are not printed
    put(&fx.root, "nothing/only.bin", &blob(9, 4096));
    let (_, out, _) = run(&fx, &["find", "nothing", "."]);
    assert!(out.contains("0 share a size"), "{out}");
    assert!(!out.contains("SAMPLE"), "{out}");
    assert!(out.contains("1 file   4.00 KB checked"), "{out}");
}

#[test]
fn undo_elsewhere_is_named() {
    let fx = fixture();
    let (code, out, _) = run(&fx, &["milk", "."]);
    assert_eq!(code, 0, "{out}");
    // from a root with no journal of its own, the pending one is pointed at
    let other = fx.root.join("unique");
    let (code, out, _) = run(&fx, &["--undo", "unique"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("nothing to undo under unique"), "{out}");
    assert!(out.contains("undo   dedupe --undo "), "{out}");
    assert!(out.contains("dedupe milk .   4 entries"), "{out}");
    let _ = other;
    let (code, _, _) = run(&fx, &["--undo"]);
    assert_eq!(code, 0);
}

/// Ctrl-C in the middle of a milk: the moves made so far are journaled
/// and undo restores every one of them. Unix: SIGINT by `kill`.
#[cfg(unix)]
#[test]
fn interrupted_milk_leaves_a_journal_undo_can_use() {
    interrupted_milk_case(
        |c| c,
        |child| {
            // SAFETY: the pid is our own child's, still running (we hold its stdout)
            unsafe { libc::kill(child.id() as i32, libc::SIGINT) == 0 }
        },
    );
}

/// The same on Windows (TECH-07): the child runs in a process group of
/// its own and gets CTRL_BREAK from `GenerateConsoleCtrlEvent` (a
/// CTRL_C cannot be aimed at one process group). The handler treats it
/// as Ctrl-C. Skipped only when this test has no console at all (a
/// service); with one, an event that cannot be sent fails the test, so a
/// pass under `runs/win` is the interrupt itself.
#[cfg(windows)]
#[test]
fn interrupted_milk_leaves_a_journal_undo_can_use() {
    use std::os::windows::process::CommandExt;
    #[link(name = "kernel32")]
    extern "system" {
        fn GenerateConsoleCtrlEvent(event: u32, group: u32) -> i32;
        fn GetConsoleWindow() -> *mut std::ffi::c_void;
    }
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CTRL_BREAK_EVENT: u32 = 1;
    interrupted_milk_case(
        |c| {
            c.creation_flags(CREATE_NEW_PROCESS_GROUP);
            c
        },
        |child| {
            // SAFETY: plain Win32 calls: no arguments, or our own child's
            // group id
            if unsafe { GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, child.id()) } != 0 {
                // shown by `--nocapture`: the launcher's summary reads it
                eprintln!("CTRL_BREAK sent to the milk's process group {}", child.id());
                return true;
            }
            let err = std::io::Error::last_os_error();
            assert!(
                unsafe { GetConsoleWindow() }.is_null(),
                "CTRL_BREAK could not be sent with a console present: {err}"
            );
            false
        },
    );
}

fn interrupted_milk_case(
    spawn: impl Fn(&mut Command) -> &mut Command,
    interrupt: impl Fn(&std::process::Child) -> bool,
) {
    use std::io::Read as _;
    use std::process::Stdio;
    use std::time::{Duration, Instant};
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("fx");
    let journal = tmp.path().join("journal");
    fs::create_dir_all(&root).unwrap();
    // many small duplicate pairs so the move loop runs long enough to stop
    let n = 6000;
    for i in 0..n {
        let data = blob(i as u8, 256 + (i % 7) * 64);
        put(&root, &format!("a/f{i:04}.bin"), &data);
        put(&root, &format!("b/f{i:04}.bin"), &data);
    }
    let before = files(&root);
    let mut cmd = Command::new(bin());
    cmd.args(["milk", "."])
        .current_dir(&root)
        .env("DEDUPE_JOURNAL_DIR", &journal)
        .env("NO_COLOR", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = spawn(&mut cmd).spawn().unwrap();
    // wait until the first move is reported, then interrupt
    let mut out = child.stdout.take().unwrap();
    let mut seen = String::new();
    let t0 = Instant::now();
    let mut buf = [0u8; 4096];
    while !seen.contains(" moved     ") && t0.elapsed() < Duration::from_secs(20) {
        let k = out.read(&mut buf).unwrap();
        if k == 0 {
            break;
        }
        seen.push_str(&String::from_utf8_lossy(&buf[..k]));
    }
    assert!(
        seen.contains(" moved     "),
        "no move seen before the deadline\n{seen}"
    );
    if !interrupt(&child) {
        let _ = child.kill();
        let _ = child.wait();
        eprintln!("the interrupt could not be sent (no console); skipped");
        return;
    }
    let mut rest = String::new();
    out.read_to_string(&mut rest).unwrap();
    seen.push_str(&rest);
    let st = child.wait().unwrap();
    assert_eq!(st.code(), Some(130), "{seen}");
    assert!(seen.contains("interrupted after"), "{seen}");
    assert!(seen.contains("the journal covers them"), "{seen}");
    let moved = seen.matches(" moved     ").count();
    assert!(
        moved < n,
        "the run must have stopped early ({moved} of {n})\n{seen}"
    );
    let after = files(&root);
    assert_eq!(
        after.iter().filter(|p| p.starts_with("_milked/")).count(),
        moved
    );

    let fx = Fx {
        _tmp: tmp,
        root: root.clone(),
        journal,
    };
    let (code, out, _) = run(&fx, &["--undo"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains(&format!("{moved} restored")), "{out}");
    assert_eq!(
        files(&fx.root),
        before,
        "every interrupted move is reversed"
    );
}

#[test]
fn milk_then_undo_round_trip() {
    let fx = fixture();
    let before = files(&fx.root);

    let (code, out, _) = run(&fx, &["milk", ".", "--dry-run"]);
    assert_eq!(code, 0, "{out}");
    assert_eq!(files(&fx.root), before, "dry run must not touch the tree");
    assert_eq!(journals(&fx.journal), 0, "dry run must not journal");

    let (code, out, _) = run(&fx, &["milk", "."]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("4 moved"), "{out}");
    let after = files(&fx.root);
    assert!(
        after.contains(&"_milked/clips/copy/a.mp4".to_string()),
        "{after:?}"
    );
    assert!(
        after.contains(&"_milked/backup/old/IMG_0041 (1).jpg".to_string()),
        "{after:?}"
    );
    assert!(after.contains(&"2019/trip/IMG_0041.jpg".to_string()));
    assert!(!after.contains(&"backup/IMG_0041.jpg".to_string()));

    // idempotent
    let (code, out, _) = run(&fx, &["find", "."]);
    assert_eq!(code, 0);
    assert!(out.contains("no duplicates"), "{out}");

    let (code, out, _) = run(&fx, &["--undo"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("4 restored"), "{out}");
    assert_eq!(files(&fx.root), before, "undo must restore the exact tree");
    assert!(!fx.root.join("_milked").exists(), "empty _milked is pruned");

    let (code, out, _) = run(&fx, &["--undo"]);
    assert_eq!(code, 0);
    assert!(out.contains("nothing to undo"), "{out}");
}

#[test]
fn milk_collision_gets_suffix() {
    let fx = fixture();
    put(&fx.root, "_milked/notes (copy).txt", b"occupied");
    let (code, out, _) = run(&fx, &["milk"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        fx.root.join("_milked/notes (copy) (2).txt").exists(),
        "{out}"
    );
    assert!(out.contains("as notes (copy) (2).txt"), "{out}");
}

#[test]
fn purge_dry_run_verifies_and_changes_nothing() {
    let fx = fixture();
    let before = files(&fx.root);
    let (code, out, _) = run(&fx, &["purge", ".", "-n"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("4 verified byte for byte"), "{out}");
    assert!(out.contains("4 would bin"), "{out}");
    assert_eq!(files(&fx.root), before);
    assert_eq!(journals(&fx.journal), 0, "a dry run journals nothing");
}

#[test]
fn image_filter_only_sees_images() {
    let fx = fixture();
    let (code, out, _) = run(&fx, &["find", ".", "img"]);
    assert_eq!(code, 0);
    assert!(out.contains("1 group   2 duplicates"), "{out}");
    assert!(!out.contains("a.mp4"), "{out}");
}

#[test]
fn error_paths_exit_one_with_help_hint() {
    let fx = fixture();
    for args in [
        &["find", "--nope"][..],
        &["."],
        &["find", "a", "b"],
        &["find", "nowhere-xyz"],
    ] {
        let (code, _, err) = run(&fx, args);
        assert_eq!(code, 1, "{args:?}");
        assert!(err.contains("dedupe --help"), "{args:?}: {err}");
    }
}

#[test]
fn help_and_doctor() {
    let fx = fixture();
    let (code, out, _) = run(&fx, &["-h"]);
    assert_eq!(code, 0);
    assert!(
        out.contains("the deduplifier") && out.contains("--quick-search"),
        "{out}"
    );
    let (code, out, _) = run(&fx, &["-dr"]);
    assert!(code == 0 || code == 2, "{out}");
    assert!(out.contains("PASS  folder"), "{out}");
    assert!(out.contains("journal"), "{out}");
}

#[test]
fn near_pass_runs_for_img_and_groups_variants() {
    let fx = fixture_with_photos();
    let (code, out, _) = run(&fx, &["find", ".", "img"]);
    assert_eq!(code, 0, "{out}");
    // exact: the IMG_0041 trio from the base fixture, then export/beach.jpg + its byte copy
    assert!(out.contains("GROUP 001   3 copies"), "{out}");
    assert!(out.contains("GROUP 002   2 copies"), "{out}");
    assert!(
        out.contains("keep  photos/export/beach.jpg")
            || out.contains("keep  photos\\export\\beach.jpg"),
        "{out}"
    );
    // near: original, small jpeg, export jpeg (keeper of GROUP 001), mirror
    // the header counts members beside the keeper, as the totals do
    assert!(out.contains("NEAR 003   4 similar"), "{out}");
    assert!(out.contains("q "), "PDQ distance column\n{out}");
    assert!(
        out.contains("beach_crop"),
        "the 5 % crop groups on the two 64-bit hashes\n{out}"
    );
    assert!(
        out.contains("cropped 95%") && out.contains("q  32"),
        "PDQ missed the crop by one bit; the geometric fit confirms it\n{out}"
    );
    assert!(
        out.contains("keep  photos/beach.png") || out.contains("keep  photos\\beach.png"),
        "{out}"
    );
    assert!(out.contains("mirror"), "{out}");
    assert!(out.contains("+GROUP 002"), "{out}");
    assert!(out.contains("1 near group   4 similar"), "{out}");
    assert!(out.contains("1 flat, skipped"), "{out}");
    if dedupe::deep::installed().is_some() {
        // the procedural forest scores 0.76 against beach on the model
        // (VERIFICATION.md, Phase 4): it may be listed by the deep tier,
        // never as a near duplicate
        assert!(
            !out.contains("near  photos/forest") && !out.contains("near  photos\\forest"),
            "a different picture must not join the group as near\n{out}"
        );
    } else {
        assert!(
            !out.contains("forest"),
            "a different picture must not join the group\n{out}"
        );
    }
    assert!(out.contains("DECODE") && out.contains("MATCH"), "{out}");
}

#[test]
fn one_rule_for_depth_quick_is_the_exception() {
    // find, milk and purge run the near pass alike; --quick-search skips it
    // and names the full run (Phase 8)
    let fx = fixture_with_photos();
    let (code, out, _) = run(&fx, &["find", "."]);
    assert_eq!(code, 0);
    assert!(out.contains("NEAR"), "plain find runs the near pass\n{out}");
    assert!(
        !out.contains("near   dedupe"),
        "no hint when the pass ran\n{out}"
    );
    let (code, out, _) = run(&fx, &["find", ".", "-q"]);
    assert_eq!(code, 0);
    assert!(!out.contains("NEAR"), "{out}");
    assert!(
        out.contains("full   dedupe find ."),
        "the full run is named\n{out}"
    );
    assert!(
        out.contains("the full run matches near duplicates"),
        "{out}"
    );
    let (code, out, _) = run(&fx, &["milk", ".", "-n"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("would~"),
        "milk acts on what find lists\n{out}"
    );
    let (code, out, _) = run(&fx, &["purge", ".", "-n"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("NEAR"), "{out}");
    assert!(out.contains("would~"), "{out}");
    assert!(out.contains("not a byte copy"), "{out}");
}

#[test]
fn milk_img_moves_near_duplicates_and_undo_restores() {
    let fx = fixture_with_photos();
    let before = files(&fx.root);
    let (code, out, _) = run(&fx, &["milk", ".", "img"]);
    assert_eq!(code, 0, "{out}");
    // 2 + 1 exact, 3 PDQ-confirmed near, plus the crop the geometric fit confirms
    assert!(out.contains("7 moved"), "{out}");
    assert!(
        out.contains("moved~    photos/export/beach_crop.jpg")
            || out.contains("moved~    photos\\export\\beach_crop.jpg"),
        "{out}"
    );
    assert!(
        out.contains("cropped 95%") && out.contains("fit 1.00"),
        "{out}"
    );
    let after = files(&fx.root);
    assert!(after.contains(&"photos/beach.png".to_string()));
    assert!(
        after.contains(&"_milked/photos/export/beach_mirror.png".to_string()),
        "{after:?}"
    );
    assert!(
        after.contains(&"_milked/photos/beach_small.jpg".to_string()),
        "{after:?}"
    );
    assert!(
        after.contains(&"_milked/photos/export/beach.jpg".to_string()),
        "{after:?}"
    );
    assert!(
        after.contains(&"_milked/photos/export/beach (copy).jpg".to_string()),
        "{after:?}"
    );
    assert!(after.contains(&"photos/forest.png".to_string()));
    let (code, out, _) = run(&fx, &["--undo"]);
    assert_eq!(code, 0, "{out}");
    assert_eq!(files(&fx.root), before);
}

/// The beach with a dark caption bar over its lower `share`.
fn with_caption_bar(fx: &Fx, share: f32, name: &str) {
    let mut edited = image::open(fx.root.join("photos/beach.png"))
        .unwrap()
        .to_rgb8();
    let (w, h) = (edited.width(), edited.height());
    for y in (h as f32 * (1.0 - share)) as u32..h {
        for x in 0..w {
            edited.put_pixel(x, y, image::Rgb([40, 40, 48]));
        }
    }
    put(&fx.root, name, &png_bytes(&edited));
}

/// A near duplicate the two hashes group but nothing confirms stays where
/// it is for `milk` as well as `purge`: a photograph of the same scene
/// from another position is the measured case (VERIFICATION.md, Phase 6).
#[test]
fn milk_leaves_unconfirmed_near_duplicates_in_place() {
    let fx = fixture_with_photos();
    // the beach with a caption bar over its lower 35 %: the two hashes
    // still land inside their thresholds (d 7, p 14), PDQ (q 48) and the
    // fit (0.90, its cells included) do not. Until Phase 11 a 15 % bar
    // was this test's case; the fit's cells now confirm that one (below).
    with_caption_bar(&fx, 0.35, "photos/beach_bar.png");
    let (code, out, _) = run(&fx, &["find", ".", "img"]);
    assert_eq!(code, 0, "{out}");
    let line = out
        .lines()
        .find(|l| l.contains("beach_bar.png"))
        .unwrap_or_else(|| panic!("the hashes group it\n{out}"));
    assert!(
        line.contains("similar, unconfirmed") && line.contains("q  48") && line.contains("fit 0.9"),
        "PDQ over, fit under\n{out}"
    );
    let (code, out, _) = run(&fx, &["milk", ".", "img"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("kept~     photos/beach_bar.png")
            || out.contains("kept~     photos\\beach_bar.png"),
        "{out}"
    );
    assert!(
        out.contains("1 kept~: similar on two hashes, confirmed by nothing else"),
        "{out}"
    );
    assert!(fx.root.join("photos/beach_bar.png").exists());
    assert!(
        !fx.root.join("photos/beach_small.jpg").exists(),
        "confirmed members still move\n{out}"
    );
}

/// A caption bar over a fifth of the picture leaves the rest of a true copy
/// identical: the fit's cells, the worst quarter set aside, confirm it
/// where the whole picture's correlation could not (Phase 11, CORE-01).
#[test]
fn a_caption_bar_is_confirmed_by_the_fits_cells() {
    let fx = fixture_with_photos();
    with_caption_bar(&fx, 0.15, "photos/beach_bar.png");
    let (code, out, _) = run(&fx, &["find", ".", "img"]);
    assert_eq!(code, 0, "{out}");
    let line = out
        .lines()
        .find(|l| l.contains("beach_bar.png"))
        .unwrap_or_else(|| panic!("the hashes group it\n{out}"));
    // PDQ is over (q 36); the plain correlation was 0.78
    assert!(
        line.contains("near copy") && line.contains("q  36") && line.contains("fit 1.00"),
        "{out}"
    );
    let (code, out, _) = run(&fx, &["milk", ".", "img"]);
    assert_eq!(code, 0, "{out}");
    assert!(!fx.root.join("photos/beach_bar.png").exists(), "{out}");
    assert!(
        fx.root.join("_milked/photos/beach_bar.png").exists(),
        "{out}"
    );
}

/// Two files that state different capture moments are two photographs:
/// `kin`, shown in the group, never moved or binned. The same moment, or
/// no moment on either side, leaves the pictures to decide.
#[test]
fn different_capture_moments_are_kin() {
    let fx = fixture();
    let beach = photo(31, 1000, 750);
    let original = jpeg_bytes(&beach, 92);
    put(
        &fx.root,
        "shots/a.jpg",
        &with_capture_time(&original, "2021:03:04 05:06:07"),
    );
    // a re-export of the same exposure keeps its capture time
    put(
        &fx.root,
        "shots/a_export.jpg",
        &with_capture_time(&jpeg_bytes(&beach, 70), "2021:03:04 05:06:07"),
    );
    // the next frame of a burst: nearly the same picture, one second later
    let mut next = beach.clone();
    for p in next.pixels_mut() {
        p.0[0] = p.0[0].saturating_add(6);
    }
    put(
        &fx.root,
        "shots/a_next.jpg",
        &with_capture_time(&jpeg_bytes(&next, 92), "2021:03:04 05:06:08"),
    );
    // a copy that lost its EXIF: the hashes decide
    put(&fx.root, "shots/a_stripped.jpg", &jpeg_bytes(&beach, 80));

    let (code, out, _) = run(&fx, &["find", ".", "img"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("kin   shots/a_next.jpg") || out.contains("kin   shots\\a_next.jpg"),
        "{out}"
    );
    assert!(out.contains("another shot, 05:06:08"), "{out}");
    assert!(out.contains("+1 kin"), "{out}");
    assert!(out.contains("1 kin"), "{out}");
    assert!(
        out.contains("near  shots/a_export.jpg") || out.contains("near  shots\\a_export.jpg"),
        "same moment: a derivative\n{out}"
    );
    assert!(
        out.contains("near  shots/a_stripped.jpg") || out.contains("near  shots\\a_stripped.jpg"),
        "no moment on one side: the hashes decide\n{out}"
    );

    let (code, out, _) = run(&fx, &["purge", ".", "img", "-n"]);
    assert_eq!(code, 0, "{out}");
    assert!(!out.contains("would~    shots/a_next.jpg"), "{out}");
    let (code, out, _) = run(&fx, &["milk", ".", "img"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        fx.root.join("shots/a_next.jpg").exists(),
        "kin must not move\n{out}"
    );
    assert!(!fx.root.join("shots/a_export.jpg").exists(), "{out}");
}

// ---------------------------------------------------------------------------
// video fixture (needs ffmpeg; the tests skip without it)
// ---------------------------------------------------------------------------

fn ffmpeg_ok() -> bool {
    let (a, b) = dedupe::video::tools_present();
    a && b
}

fn ffm(args: &[&str]) {
    let st = dedupe::tools::ffmpeg()
        .args(["-hide_banner", "-loglevel", "error", "-y", "-nostdin"])
        .args(args)
        .status()
        .expect("spawn ffmpeg");
    assert!(st.success(), "ffmpeg {args:?}");
}

/// A 10-second fractal zoom and its derivatives, plus an unrelated clip.
fn add_videos(root: &Path) {
    let v = root.join("video");
    fs::create_dir_all(v.join("phone")).unwrap();
    let a = v.join("zoom.mp4");
    ffm(&[
        "-f",
        "lavfi",
        "-i",
        "mandelbrot=size=640x360:rate=24:start_scale=3:end_scale=0.3:maxiter=200",
        "-t",
        "10",
        "-c:v",
        "libx264",
        "-preset",
        "veryfast",
        "-crf",
        "20",
        "-pix_fmt",
        "yuv420p",
        a.to_str().unwrap(),
    ]);
    let a = a.to_str().unwrap();
    ffm(&[
        "-i",
        a,
        "-vf",
        "scale=320:180",
        "-an",
        "-c:v",
        "libx264",
        "-preset",
        "veryfast",
        "-crf",
        "30",
        v.join("phone/zoom.mp4").to_str().unwrap(),
    ]);
    ffm(&[
        "-i",
        a,
        "-vf",
        "hflip",
        "-an",
        "-c:v",
        "libx264",
        "-preset",
        "veryfast",
        "-crf",
        "22",
        v.join("zoom_mirror.mp4").to_str().unwrap(),
    ]);
    ffm(&[
        "-ss",
        "2",
        "-i",
        a,
        "-an",
        "-c:v",
        "libx264",
        "-preset",
        "veryfast",
        "-crf",
        "22",
        v.join("zoom_cut.mp4").to_str().unwrap(),
    ]);
    fs::copy(a, v.join("zoom (1).mp4")).unwrap(); // byte copy
                                                  // the same clip boxed in black pillars: 80 px each side of 800
    ffm(&[
        "-i",
        a,
        "-vf",
        "pad=800:360:80:0:black",
        "-an",
        "-c:v",
        "libx264",
        "-preset",
        "veryfast",
        "-crf",
        "22",
        v.join("zoom_pillars.mp4").to_str().unwrap(),
    ]);
    ffm(&[
        "-f",
        "lavfi",
        "-i",
        "testsrc2=size=640x360:rate=30",
        "-t",
        "10",
        "-c:v",
        "libx264",
        "-preset",
        "veryfast",
        "-crf",
        "20",
        "-pix_fmt",
        "yuv420p",
        v.join("pattern.mp4").to_str().unwrap(),
    ]);
}

fn fixture_with_videos() -> Fx {
    let fx = fixture();
    add_videos(&fx.root);
    fx
}

/// One column holds every keeper's date in the GROUP, NEAR and VIDEO
/// blocks of one listing (Phase 10), at a wide and a common width.
#[test]
fn keepers_dates_share_one_column_across_blocks() {
    let fx = fixture_with_photos();
    let videos = ffmpeg_ok();
    if videos {
        add_videos(&fx.root);
    }
    for cols in ["200", "100"] {
        let out = Command::new(bin())
            .args(["find", "."])
            .current_dir(&fx.root)
            .env("DEDUPE_JOURNAL_DIR", &fx.journal)
            .env("NO_COLOR", "1")
            .env("COLUMNS", cols)
            .output()
            .expect("spawn dedupe");
        let out = String::from_utf8_lossy(&out.stdout);
        let mut at: std::collections::BTreeMap<&str, std::collections::BTreeSet<usize>> =
            Default::default();
        let mut block = "";
        for line in out.lines() {
            for b in ["GROUP", "NEAR", "VIDEO"] {
                if line.starts_with(&format!(" {b} ")) {
                    block = b;
                }
            }
            if line.starts_with("   keep  ") {
                let t = line.trim_end();
                let date: String = t.chars().rev().take(10).collect();
                assert!(
                    date.chars().filter(|c| *c == '-').count() == 2,
                    "no date at the end of {t:?}"
                );
                at.entry(block).or_default().insert(t.chars().count() - 10);
            }
        }
        assert!(at.contains_key("GROUP") && at.contains_key("NEAR"), "{out}");
        assert!(!videos || at.contains_key("VIDEO"), "{out}");
        let all: std::collections::BTreeSet<usize> = at.values().flatten().copied().collect();
        assert_eq!(all.len(), 1, "dates at {at:?}, {cols} columns\n{out}");
    }
}

#[test]
fn video_pass_groups_derivatives_and_labels_them() {
    if !ffmpeg_ok() {
        eprintln!("ffmpeg missing; skipped");
        return;
    }
    let fx = fixture_with_videos();
    let (code, out, _) = run(&fx, &["find", ".", "vid"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("FRAMES") && out.contains("MATCH"), "{out}");
    // exact: zoom.mp4 + its byte copy; the earlier fixture's a.mp4 pair is also exact
    assert!(
        out.contains("GROUP 001") && out.contains("GROUP 002"),
        "{out}"
    );
    // video group: zoom (keeper, longest and most pixels), phone copy, mirror, cut, pillars
    assert!(out.contains("VIDEO 003   4 similar"), "{out}");
    assert!(
        out.contains("keep  video/zoom.mp4") || out.contains("keep  video\\zoom.mp4"),
        "{out}"
    );
    assert!(out.contains("mirrored"), "{out}");
    assert!(out.contains("trimmed 0:08 of 0:10"), "{out}");
    assert!(
        out.contains("scaled 640\u{2192}320"),
        "the phone copy\n{out}"
    );
    // the pillarboxed copy: 800×360 in the listing, frames compared inside
    // the bars, TMK at 1.00 against the keeper, `pillarboxed` on the line
    let pillars = out
        .lines()
        .find(|l| l.contains("zoom_pillars.mp4"))
        .unwrap_or_else(|| panic!("pillarboxed copy not listed\n{out}"));
    assert!(
        pillars.contains("near") && pillars.contains("800x360"),
        "{pillars}"
    );
    assert!(
        pillars.contains("tmk 1.00") || pillars.contains("tmk 0.9"),
        "{pillars}"
    );
    assert!(pillars.contains("pillarboxed"), "{pillars}");
    assert!(
        !out.contains("pattern.mp4"),
        "unrelated clip must stay out\n{out}"
    );
    assert!(out.contains("1 video group   4 similar"), "{out}");
    assert!(out.contains("keep: longest, then most pixels"), "{out}");
}

#[test]
fn video_pass_runs_for_plain_find_and_purge_alike() {
    if !ffmpeg_ok() {
        return;
    }
    let fx = fixture_with_videos();
    let (code, out, _) = run(&fx, &["find", "."]);
    assert_eq!(code, 0);
    assert!(
        out.contains("VIDEO"),
        "plain find runs the video pass\n{out}"
    );
    let (code, out, _) = run(&fx, &["find", ".", "-q"]);
    assert_eq!(code, 0);
    assert!(!out.contains("VIDEO"), "{out}");
    assert!(out.contains("full   dedupe find ."), "{out}");
    let (code, out, _) = run(&fx, &["purge", ".", "-n"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("VIDEO"), "purge runs every pass\n{out}");
    assert!(
        out.contains("would~    video/phone/zoom.mp4")
            || out.contains("would~    video\\phone\\zoom.mp4"),
        "{out}"
    );
    assert!(
        out.contains("would~    video/zoom_mirror.mp4")
            || out.contains("would~    video\\zoom_mirror.mp4"),
        "{out}"
    );
}

#[test]
fn milk_vid_moves_and_undo_restores() {
    if !ffmpeg_ok() {
        return;
    }
    let fx = fixture_with_videos();
    let before = files(&fx.root);
    let (code, out, _) = run(&fx, &["milk", ".", "vid"]);
    assert_eq!(code, 0, "{out}");
    // exact: zoom (1), clips/copy/a.mp4; near: phone, mirror, cut, pillars
    assert!(out.contains("6 moved"), "{out}");
    let after = files(&fx.root);
    assert!(
        after.contains(&"_milked/video/zoom_cut.mp4".to_string()),
        "{after:?}"
    );
    assert!(after.contains(&"video/zoom.mp4".to_string()));
    assert!(after.contains(&"video/pattern.mp4".to_string()));
    let (code, out, _) = run(&fx, &["--undo"]);
    assert_eq!(code, 0, "{out}");
    assert_eq!(files(&fx.root), before);
}

// ---------------------------------------------------------------------------
// deep tier (runs only with the model installed; see docs/DEEP.md)
// ---------------------------------------------------------------------------

#[test]
fn deep_tier_acts_only_with_the_fit() {
    if dedupe::deep::installed().is_none() {
        eprintln!("no model installed; skipped");
        return;
    }
    let fx = fixture_with_photos();
    // a letterboxed copy: the 64-bit pair misses it (p 20), SSCD does not
    let beach = image::open(fx.root.join("photos/beach.png"))
        .unwrap()
        .to_rgb8();
    let (w, h) = (beach.width(), beach.height());
    let mut lb = image::RgbImage::from_pixel(w, h * 5 / 4, image::Rgb([0, 0, 0]));
    image::imageops::overlay(&mut lb, &beach, 0, (h / 8) as i64);
    put(&fx.root, "photos/beach_letterbox.png", &png_bytes(&lb));

    let (code, out, _) = run(&fx, &["find", ".", "img"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("hashed   +deep"), "{out}");
    // the descriptor admits the letterbox (the hashes missed it at p 20);
    // the geometric fit confirms it, so it is an ordinary near duplicate
    assert!(
        out.contains("near  photos/beach_letterbox.png")
            || out.contains("near  photos\\beach_letterbox.png"),
        "{out}"
    );
    assert!(
        out.contains("deep 0.7") && out.contains("fit 1.00"),
        "{out}"
    );
    // the procedural forest (deep 0.76) fits nothing: listed, never acted on
    assert!(
        out.contains("deep  photos/forest.png") || out.contains("deep  photos\\forest.png"),
        "{out}"
    );
    assert!(out.contains("by deep only   listed for review"), "{out}");
    assert!(
        out.contains("keep  photos/beach.png") || out.contains("keep  photos\\beach.png"),
        "the original stays keeper\n{out}"
    );

    let before = files(&fx.root);
    let (code, out, _) = run(&fx, &["milk", ".", "img"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        !fx.root.join("photos/beach_letterbox.png").exists(),
        "descriptor + fit: two families agree, the letterbox moves\n{out}"
    );
    assert!(
        fx.root.join("photos/forest.png").exists(),
        "the descriptor alone never moves a file\n{out}"
    );
    let (code, _, _) = run(&fx, &["--undo"]);
    assert_eq!(code, 0);
    assert_eq!(files(&fx.root), before);
}

/// Touches the machine's Recycle Bin / trash. `cargo test -- --ignored`.
#[test]
#[ignore]
fn purge_then_undo_through_the_bin() {
    let fx = fixture();
    let before = files(&fx.root);
    let (code, out, _) = run(&fx, &["purge", "."]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("4 binned"), "{out}");
    assert!(!fx.root.join("backup/IMG_0041.jpg").exists());
    let (code, out, _) = run(&fx, &["--undo"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("4 restored"), "{out}");
    assert_eq!(files(&fx.root), before);
    let a = fs::read(fx.root.join("2019/trip/IMG_0041.jpg")).unwrap();
    let b = fs::read(fx.root.join("backup/IMG_0041.jpg")).unwrap();
    assert_eq!(a, b);
}

/// The result is the workspace: `find` on a terminal opens its groups in a
/// screen of their own; `n` previews on the main screen and enter comes
/// back; `m` + enter moves what is selected without a second scan; `u` on
/// the bar under the action reverses it; `q` leaves. Piped, the `next`
/// lines print instead.
#[cfg(any(unix, windows))]
#[test]
fn find_on_a_terminal_acts_from_the_result() {
    let fx = fixture_with_photos();
    let before = files(&fx.root);
    let Some(out) = run_tty(
        &fx,
        &["find", ".", "img"],
        &["n", "\r", "m", "\r", "u", "q"],
        160,
    ) else {
        eprintln!("no pseudo-terminal (script(1), or a Windows pseudo console); skipped");
        return;
    };
    // the groups screen: marks, the selection count, the keys
    assert!(out.contains("selected"), "the status line\n{out}");
    assert!(out.contains("m milk"), "the bar\n{out}");
    assert!(out.contains("p purge"), "{out}");
    assert!(out.contains("space select"), "{out}");
    assert!(out.contains("k keeper"), "{out}");
    // n: the preview, with the native dry-run command named
    assert!(out.contains("dedupe milk . img --dry-run"), "{out}");
    assert!(out.contains("would~"), "{out}");
    // m, enter: the move, on the groups already listed (one SCAN line
    // only; a Windows pseudo console repaints the main screen when the
    // workspace leaves its own, so there the line can come twice)
    if cfg!(unix) {
        assert_eq!(out.matches("SCAN ").count(), 1, "no second scan\n{out}");
    }
    assert!(out.contains("7 moved"), "{out}");
    assert!(out.contains("u undo it"), "{out}");
    // u: reversed on the spot
    assert!(out.contains("7 restored"), "{out}");
    assert_eq!(files(&fx.root), before, "tree back as it was");
    // piped, no workspace, the typed forms instead
    let (_, piped, _) = run(&fx, &["find", ".", "img"]);
    assert!(!piped.contains("m milk"), "{piped}");
    assert!(piped.contains("next   dedupe milk . img"), "{piped}");
    // shown by `--nocapture`: the Windows launcher's summary reads it
    eprintln!("the workspace driven through a pseudo-terminal: 7 moved, 7 restored");
}

/// Colour on a terminal (QOL-10): with `NO_COLOR` unset, the doctor finds
/// ANSI support and paints its lines. On Windows the terminal is a
/// pseudo console, the console host itself: "24-bit ANSI colour" means
/// virtual-terminal processing was asked for and granted, colour escapes
/// coming back mean the console drew colour, and no escape text in what
/// it shows means it read the escapes rather than printing them.
#[cfg(any(unix, windows))]
#[test]
fn a_terminal_gets_colour() {
    let fx = fixture_with_photos();
    let Some(raw) = run_tty_raw(&fx, &["doctor"], &[], 120, true) else {
        eprintln!("no pseudo-terminal (script(1), or a Windows pseudo console); skipped");
        return;
    };
    let shown = plain_text(&raw);
    assert!(shown.contains("24-bit ANSI colour"), "{shown}");
    // a colour escape of any form the terminal may send back: 24-bit,
    // 256-colour, or one of the sixteen
    let colour_sgr = raw.split('\u{1b}').skip(1).any(|seq| {
        let Some(params) = seq.strip_prefix('[') else {
            return false;
        };
        let end = params.find(|c: char| ('@'..='~').contains(&c));
        end.is_some_and(|e| {
            params.as_bytes()[e] == b'm'
                && params[..e].split(';').any(|p| {
                    p == "38"
                        || p.parse::<u32>()
                            .is_ok_and(|n| (30..=37).contains(&n) || (90..=97).contains(&n))
                })
        })
    });
    assert!(colour_sgr, "no colour escape came back\n{raw:?}");
    assert!(!shown.contains("[38;2;"), "escapes shown as text\n{shown}");
    // a note under a PASS (the deep row's model path, when no model is
    // installed) is as quiet as the detail: the last colour set before it
    // is grey (the Windows pseudo console moves the cursor in between)
    if let Some(at) = raw.find("expected at") {
        let sgr = raw[..at]
            .split('\u{1b}')
            .skip(1)
            .filter_map(|seq| {
                let params = seq.strip_prefix('[')?;
                let end = params.find(|c: char| ('@'..='~').contains(&c))?;
                (params.as_bytes()[end] == b'm').then(|| params[..end].to_string())
            })
            .last()
            .expect("a colour set before the note");
        assert!(
            sgr.contains("138;138;138"),
            "note under a PASS painted {sgr:?}"
        );
    }
    // shown by `--nocapture`: the Windows launcher's summary reads it
    eprintln!("colour on a terminal: 24-bit ANSI colour, colour escapes back");
}

// ---------------------------------------------------------------------------
// formats (Phase 10): a camera RAW and its JPEG; a tiled, rotated HEIC
// ---------------------------------------------------------------------------

/// A little-endian TIFF the way a camera writes a RAW: IFD0 with the
/// orientation, the embedded preview's offset and length, and an Exif IFD
/// with the capture moment; then bytes standing for sensor data; then the
/// preview JPEG.
fn raw_file(preview: &[u8], datetime: &str, orientation: u16) -> Vec<u8> {
    let le16 = |v: u16| v.to_le_bytes();
    let le32 = |v: u32| v.to_le_bytes();
    let entry = |t: &mut Vec<u8>, tag: u16, typ: u16, count: u32, value: [u8; 4]| {
        t.extend_from_slice(&le16(tag));
        t.extend_from_slice(&le16(typ));
        t.extend_from_slice(&le32(count));
        t.extend_from_slice(&value);
    };
    let ifd0 = 8u32;
    let ifd0_len = 2 + 4 * 12 + 4;
    let exif = ifd0 + ifd0_len;
    let exif_len = 2 + 12 + 4;
    let dt = format!("{datetime}\0");
    let dt_off = exif + exif_len;
    let sensor_off = dt_off + dt.len() as u32;
    let sensor = blob(77, 40_000);
    let preview_off = sensor_off + sensor.len() as u32;
    let mut t = b"II*\0".to_vec();
    t.extend_from_slice(&le32(ifd0));
    t.extend_from_slice(&le16(4));
    let mut short = [0u8; 4];
    short[..2].copy_from_slice(&le16(orientation));
    entry(&mut t, 0x0112, 3, 1, short);
    entry(&mut t, 0x0201, 4, 1, le32(preview_off));
    entry(&mut t, 0x0202, 4, 1, le32(preview.len() as u32));
    entry(&mut t, 0x8769, 4, 1, le32(exif));
    t.extend_from_slice(&le32(0));
    t.extend_from_slice(&le16(1));
    entry(&mut t, 0x9003, 2, dt.len() as u32, le32(dt_off));
    t.extend_from_slice(&le32(0));
    t.extend_from_slice(dt.as_bytes());
    t.extend_from_slice(&sensor);
    t.extend_from_slice(preview);
    t
}

/// A RAW keeps its group; the camera's JPEG beside it is a pair, left in
/// place unless chosen by hand; an export elsewhere is a near duplicate
/// of the RAW and moves; the RAW itself never moves.
#[test]
fn a_raw_keeps_its_group_and_its_camera_jpeg() {
    let fx = fixture();
    let shot = photo(41, 1200, 800);
    let preview = jpeg_bytes(&shot, 92);
    put_at(
        &fx.root,
        "dcim/IMG_0041.CR2",
        &raw_file(&preview, "2022:06:10 18:17:40", 1),
        1_600_000_000,
    );
    // the camera's own JPEG of the same exposure, beside it
    put(
        &fx.root,
        "dcim/IMG_0041.JPG",
        &with_capture_time(&jpeg_bytes(&shot, 95), "2022:06:10 18:17:40"),
    );
    // an export elsewhere, half size, the capture time kept
    let half = image::imageops::resize(&shot, 600, 400, image::imageops::FilterType::Triangle);
    put(
        &fx.root,
        "web/holiday.jpg",
        &with_capture_time(&jpeg_bytes(&half, 80), "2022:06:10 18:17:40"),
    );

    let (code, out, _) = run(&fx, &["find", ".", "img"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("1 RAW"),
        "the DECODE line counts the route\n{out}"
    );
    let line = |name: &str| {
        out.lines()
            .find(|l| l.contains(name))
            .unwrap_or_else(|| panic!("{name} not listed\n{out}"))
            .to_string()
    };
    let raw = line("IMG_0041.CR2");
    assert!(raw.contains("keep") && raw.contains("RAW"), "{raw}");
    let twin = line("IMG_0041.JPG");
    assert!(
        twin.contains("pair") && twin.contains("RAW + JPEG"),
        "{twin}"
    );
    let web = line("holiday.jpg");
    assert!(
        web.contains("near") && web.contains("from the RAW"),
        "{web}"
    );
    assert!(out.contains("+1 pair"), "{out}");

    let (code, out, _) = run(&fx, &["milk", ".", "img"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        fx.root.join("dcim/IMG_0041.CR2").exists(),
        "the RAW never moves\n{out}"
    );
    assert!(
        fx.root.join("dcim/IMG_0041.JPG").exists(),
        "the pair stays\n{out}"
    );
    assert!(
        !fx.root.join("web/holiday.jpg").exists(),
        "the export moves\n{out}"
    );
    assert!(
        out.contains("the camera's JPEG beside its RAW stays with it"),
        "{out}"
    );
}

/// The 23 bytes of an `hvcC` before its arrays, from the SPS's
/// profile_tier_level (12 bytes after the NAL header and one byte), for
/// 4:2:0 8-bit pictures with 4-byte NAL lengths.
fn hvcc(vps: &[u8], sps: &[u8], pps: &[u8]) -> Vec<u8> {
    // remove emulation prevention (00 00 03) from the SPS payload
    let mut rbsp = Vec::new();
    let mut zeros = 0;
    for &b in &sps[2..] {
        if zeros >= 2 && b == 3 {
            zeros = 0;
            continue;
        }
        zeros = if b == 0 { zeros + 1 } else { 0 };
        rbsp.push(b);
    }
    let ptl = &rbsp[1..13];
    let mut c = vec![1u8];
    c.extend_from_slice(ptl);
    c.extend_from_slice(&[0xF0, 0x00, 0xFC, 0xFD, 0xF8, 0xF8, 0x00, 0x00, 0x0F, 3]);
    for (typ, nal) in [(32u8, vps), (33, sps), (34, pps)] {
        c.push(0x80 | typ);
        c.extend_from_slice(&1u16.to_be_bytes());
        c.extend_from_slice(&(nal.len() as u16).to_be_bytes());
        c.extend_from_slice(nal);
    }
    c
}

/// NAL units of an Annex B stream.
fn nals(b: &[u8]) -> Vec<&[u8]> {
    let mut starts = Vec::new();
    let mut i = 0;
    while i + 3 <= b.len() {
        if b[i] == 0 && b[i + 1] == 0 && b[i + 2] == 1 {
            starts.push(i + 3);
            i += 3;
        } else {
            i += 1;
        }
    }
    let mut out = Vec::new();
    for (k, &s) in starts.iter().enumerate() {
        let mut e = starts.get(k + 1).map(|&n| n - 3).unwrap_or(b.len());
        while e > s && b[e - 1] == 0 {
            e -= 1; // the zero of a 4-byte start code
        }
        out.push(&b[s..e]);
    }
    out
}

fn has_encoder(name: &str) -> bool {
    dedupe::tools::ffmpeg()
        .args(["-hide_banner", "-encoders"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains(name))
        .unwrap_or(false)
}

/// A HEIC the way a phone writes one: the picture cut into a 2 × 2 grid
/// of HEVC tiles (x265, each tile its own IDR picture), a `grid` item as
/// the primary, `irot` turning it a quarter anticlockwise for display,
/// and an `Exif` item with the capture moment. `img` is the picture as
/// stored (before the turn).
fn heic_file(dir: &Path, img: &image::RgbImage, datetime: &str) -> Vec<u8> {
    let (tw, th) = (128u32, 128u32);
    let (w, h) = (img.width(), img.height());
    let tiles_dir = dir.join("tiles");
    fs::create_dir_all(&tiles_dir).unwrap();
    let mut n = 0;
    for r in 0..2 {
        for c in 0..2 {
            let mut t = image::RgbImage::new(tw, th);
            for y in 0..th {
                for x in 0..tw {
                    let sx = (c * tw + x).min(w - 1);
                    let sy = (r * th + y).min(h - 1);
                    t.put_pixel(x, y, *img.get_pixel(sx, sy));
                }
            }
            t.save(tiles_dir.join(format!("t{n}.png"))).unwrap();
            n += 1;
        }
    }
    let hevc = dir.join("tiles.hevc");
    ffm(&[
        "-framerate",
        "1",
        "-i",
        tiles_dir.join("t%d.png").to_str().unwrap(),
        "-c:v",
        "libx265",
        "-preset",
        "ultrafast",
        "-x265-params",
        "keyint=1:min-keyint=1:scenecut=0:log-level=none",
        "-pix_fmt",
        "yuv420p",
        "-f",
        "hevc",
        hevc.to_str().unwrap(),
    ]);
    let stream = fs::read(&hevc).unwrap();
    let units = nals(&stream);
    let typ = |n: &[u8]| (n[0] >> 1) & 0x3F;
    let vps = *units.iter().find(|n| typ(n) == 32).unwrap();
    let sps = *units.iter().find(|n| typ(n) == 33).unwrap();
    let pps = *units.iter().find(|n| typ(n) == 34).unwrap();
    // one picture per tile: the VCL units, length-prefixed
    let mut tiles: Vec<Vec<u8>> = Vec::new();
    for u in units.iter().filter(|n| typ(n) < 32) {
        let first_slice = u[2] & 0x80 != 0;
        if first_slice || tiles.is_empty() {
            tiles.push(Vec::new());
        }
        let t = tiles.last_mut().unwrap();
        t.extend_from_slice(&(u.len() as u32).to_be_bytes());
        t.extend_from_slice(u);
    }
    assert_eq!(tiles.len(), 4, "x265 wrote one picture per tile");

    // the Exif item: a 4-byte offset, then a TIFF with DateTimeOriginal
    let app1 = with_capture_time(&[0xFF, 0xD8], datetime);
    let tiff = &app1[4 + 2 + 6..app1.len()];
    let mut exif = 0u32.to_be_bytes().to_vec();
    exif.extend_from_slice(tiff);

    let bx = |t: &[u8; 4], body: &[u8]| {
        let mut v = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        v.extend_from_slice(t);
        v.extend_from_slice(body);
        v
    };
    let full = |t: &[u8; 4], ver: u8, body: &[u8]| {
        let mut b = vec![ver, 0, 0, 0];
        b.extend_from_slice(body);
        bx(t, &b)
    };
    // items 1–4 tiles, 5 grid, 6 Exif
    let grid = [0u8, 0, 1, 1]
        .iter()
        .copied()
        .chain((w as u16).to_be_bytes())
        .chain((h as u16).to_be_bytes())
        .collect::<Vec<u8>>();
    let datas: Vec<Vec<u8>> = tiles.iter().cloned().chain([grid, exif]).collect();
    let infe = |id: u16, t: &[u8; 4], hidden: bool| {
        let mut b = id.to_be_bytes().to_vec();
        b.extend_from_slice(&[0, 0]);
        b.extend_from_slice(t);
        b.push(0);
        let mut f = vec![2u8, 0, 0, hidden as u8];
        f.extend_from_slice(&b);
        bx(b"infe", &f)
    };
    let mut iinf = 6u16.to_be_bytes().to_vec();
    for id in 1..=4 {
        iinf.extend(infe(id, b"hvc1", true));
    }
    iinf.extend(infe(5, b"grid", false));
    iinf.extend(infe(6, b"Exif", false));
    let iinf = full(b"iinf", 0, &iinf);
    let mut dimg = 5u16.to_be_bytes().to_vec();
    dimg.extend_from_slice(&4u16.to_be_bytes());
    for id in 1..=4u16 {
        dimg.extend_from_slice(&id.to_be_bytes());
    }
    let mut cdsc = 6u16.to_be_bytes().to_vec();
    cdsc.extend_from_slice(&1u16.to_be_bytes());
    cdsc.extend_from_slice(&5u16.to_be_bytes());
    let mut iref = bx(b"dimg", &dimg);
    iref.extend(bx(b"cdsc", &cdsc));
    let iref = full(b"iref", 0, &iref);
    let ispe = |w: u32, h: u32| {
        let mut b = w.to_be_bytes().to_vec();
        b.extend_from_slice(&h.to_be_bytes());
        full(b"ispe", 0, &b)
    };
    // properties: 1 hvcC, 2 tile ispe, 3 grid ispe, 4 irot 1
    let mut ipco = bx(b"hvcC", &hvcc(vps, sps, pps));
    ipco.extend(ispe(tw, th));
    ipco.extend(ispe(w, h));
    ipco.extend(bx(b"irot", &[1]));
    let ipco = bx(b"ipco", &ipco);
    let mut ipma = 5u32.to_be_bytes().to_vec();
    for id in 1..=4u16 {
        ipma.extend_from_slice(&id.to_be_bytes());
        ipma.extend_from_slice(&[2, 0x81, 0x02]);
    }
    ipma.extend_from_slice(&5u16.to_be_bytes());
    ipma.extend_from_slice(&[2, 0x03, 0x84]);
    let ipma = full(b"ipma", 0, &ipma);
    let mut iprp = ipco;
    iprp.extend(ipma);
    let iprp = bx(b"iprp", &iprp);
    let hdlr = full(b"hdlr", 0, b"\0\0\0\0pict\0\0\0\0\0\0\0\0\0\0\0\0\0");
    let pitm = full(b"pitm", 0, &5u16.to_be_bytes());
    let ftyp = bx(b"ftyp", b"heic\0\0\0\0mif1heic");
    // iloc v0, 4-byte offsets and lengths; offsets known once the meta
    // box size is: its size does not depend on the offset values
    let iloc = |base: u32| {
        let mut b = vec![0x44, 0x00];
        b.extend_from_slice(&(datas.len() as u16).to_be_bytes());
        let mut off = base;
        for (i, d) in datas.iter().enumerate() {
            b.extend_from_slice(&((i + 1) as u16).to_be_bytes());
            b.extend_from_slice(&[0, 0]);
            b.extend_from_slice(&1u16.to_be_bytes());
            b.extend_from_slice(&off.to_be_bytes());
            b.extend_from_slice(&(d.len() as u32).to_be_bytes());
            off += d.len() as u32;
        }
        full(b"iloc", 0, &b)
    };
    let meta = |base: u32| {
        let mut m = hdlr.clone();
        m.extend(pitm.clone());
        m.extend(iloc(base));
        m.extend(iinf.clone());
        m.extend(iref.clone());
        m.extend(iprp.clone());
        full(b"meta", 0, &m)
    };
    let base = (ftyp.len() + meta(0).len() + 8) as u32;
    let mut out = ftyp;
    out.extend(meta(base));
    out.extend(bx(b"mdat", &datas.concat()));
    out
}

/// A phone's HEIC (tiled, turned by `irot`, its capture moment in an Exif
/// item) decodes upright and groups with its JPEG export; the export
/// moves, the HEIC stays.
#[test]
fn a_tiled_rotated_heic_decodes_and_groups_with_its_jpeg() {
    if !ffmpeg_ok() || !has_encoder("libx265") {
        eprintln!("ffmpeg with libx265 missing; skipped");
        return;
    }
    let fx = fixture();
    let work = fx.root.parent().unwrap().join("heic_work");
    // stored 240×200; displayed a quarter turn anticlockwise: 200×240
    let stored = photo(51, 240, 200);
    let heic = heic_file(&work, &stored, "2020:05:04 18:39:11");
    put_at(&fx.root, "phone/IMG_2927.HEIC", &heic, 1_588_000_000);
    let upright = image::imageops::rotate270(&stored);
    put(
        &fx.root,
        "phone/exports/IMG_2927.jpg",
        &with_capture_time(&jpeg_bytes(&upright, 90), "2020:05:04 18:39:11"),
    );
    // the same picture left unturned in a JPEG: only the dihedral
    // search finds it, and it says so
    put(
        &fx.root,
        "phone/exports/IMG_2927_sideways.jpg",
        &jpeg_bytes(&stored, 90),
    );

    let (code, out, _) = run(&fx, &["find", ".", "img"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("1 HEIC"),
        "the DECODE line counts the route\n{out}"
    );
    let line = |name: &str| {
        out.lines()
            .find(|l| l.contains(name))
            .unwrap_or_else(|| panic!("{name} not listed\n{out}"))
            .to_string()
    };
    let heic_line = line("IMG_2927.HEIC");
    assert!(heic_line.contains("keep"), "{heic_line}");
    assert!(
        heic_line.contains("200×240") || heic_line.contains("200x240"),
        "upright size\n{heic_line}"
    );
    let export = line("IMG_2927.jpg");
    assert!(export.contains("near"), "{export}");
    assert!(
        !export.contains("rotated"),
        "upright HEIC and upright JPEG: no turn between them\n{export}"
    );
    let sideways = line("IMG_2927_sideways.jpg");
    assert!(sideways.contains("rotated"), "{sideways}");

    let (code, out, _) = run(&fx, &["milk", ".", "img"]);
    assert_eq!(code, 0, "{out}");
    assert!(fx.root.join("phone/IMG_2927.HEIC").exists(), "{out}");
    assert!(
        !fx.root.join("phone/exports/IMG_2927.jpg").exists(),
        "{out}"
    );
}

/// `_milked` made a link to another file system (a person staging
/// duplicates on another drive): every move crosses volumes, so it is a
/// copy, read back and compared, then the original deleted; the journal
/// says so, and undo copies back the same way. Unix, where /dev/shm is a
/// tmpfs; skipped when it is not another device.
#[cfg(unix)]
#[test]
fn milk_across_volumes_copies_verifies_and_undoes() {
    use std::os::unix::fs::MetadataExt;
    let fx = fixture();
    let shm = std::path::Path::new("/dev/shm");
    let other = match tempfile::tempdir_in(shm) {
        Ok(d) => d,
        Err(_) => {
            eprintln!("no /dev/shm; skipped");
            return;
        }
    };
    let dev = |p: &Path| fs::metadata(p).map(|m| m.dev()).unwrap_or(0);
    if dev(other.path()) == dev(&fx.root) {
        eprintln!("/dev/shm is the same device; skipped");
        return;
    }
    std::os::unix::fs::symlink(other.path(), fx.root.join("_milked")).unwrap();
    let hash_tree = |root: &Path| -> Vec<(String, u64, Vec<u8>)> {
        files(root)
            .into_iter()
            .filter(|f| !f.starts_with("_milked"))
            .map(|f| {
                let p = root.join(&f);
                let md = fs::metadata(&p).unwrap();
                let secs = md
                    .modified()
                    .unwrap()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs();
                (f, secs, fs::read(&p).unwrap())
            })
            .collect()
    };
    let before = hash_tree(&fx.root);

    let (code, out, _) = run(&fx, &["milk", "."]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("4 moved"), "{out}");
    assert_eq!(
        out.matches("copied across volumes").count(),
        5,
        "4 lines and the footer\n{out}"
    );
    assert!(out.contains("each copy read back and compared"), "{out}");
    // the copies are on the other file system, the originals gone
    assert!(other.path().join("backup/IMG_0041.jpg").exists(), "{out}");
    assert!(!fx.root.join("backup/IMG_0041.jpg").exists());
    // the journal records the copies
    let j: Vec<String> = walkdir::WalkDir::new(&fx.journal)
        .into_iter()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json") && e.depth() >= 2)
        .map(|e| fs::read_to_string(e.path()).unwrap())
        .collect();
    assert_eq!(j.len(), 1);
    assert_eq!(j[0].matches("\"copied\": true").count(), 4, "{}", j[0]);

    let (code, out, _) = run(&fx, &["--undo"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("4 restored"), "{out}");
    assert!(out.contains("copied back across volumes"), "{out}");
    // bytes and modification times as they were
    assert_eq!(hash_tree(&fx.root), before);
}

/// `--skip` leaves a folder or a pattern out of the scan and remembers it
/// in the app folder; the header and the SCAN line say so; `--unskip`
/// forgets. `--min` leaves small files out, per run.
#[test]
fn skip_is_remembered_and_min_is_per_run() {
    let fx = fixture();
    let (code, out, _) = run(&fx, &["find", ".", "--skip", "backup"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("skip backup"), "the header names it\n{out}");
    assert!(out.contains("1 skipped"), "the SCAN line counts it\n{out}");
    assert!(!out.contains("backup/IMG_0041.jpg"), "{out}");
    assert!(
        fx.journal.join("skip.json").exists(),
        "remembered in the app folder"
    );
    assert!(!fx.root.join("skip.json").exists(), "nothing in the tree");
    // remembered: the next run skips it without being told
    let (_, out, _) = run(&fx, &["find", "."]);
    assert!(out.contains("skip backup"), "{out}");
    assert!(!out.contains("backup/IMG_0041.jpg"), "{out}");
    // a pattern, added to the list
    let (_, out, _) = run(&fx, &["find", ".", "--skip", "*.bin"]);
    assert!(out.contains("skip backup, *.bin"), "{out}");
    // forgotten
    let (_, out, _) = run(&fx, &["find", ".", "--unskip", "all"]);
    assert!(!out.contains("skip "), "{out}");
    assert!(
        out.contains("backup/IMG_0041.jpg") || out.contains("backup\\IMG_0041.jpg"),
        "{out}"
    );
    assert!(!fx.journal.join("skip.json").exists());

    // a floor: the 300 KB copies stay, the 560-byte notes go
    let (code, out, _) = run(&fx, &["find", ".", "--min", "100k"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("min 100 KB"), "{out}");
    assert!(out.contains("under 100 KB"), "{out}");
    assert!(!out.contains("notes"), "{out}");
    assert!(
        out.contains("next   dedupe milk . --min 100k"),
        "the typed forms keep the floor\n{out}"
    );
    // bad values and a missing one are usage errors
    let (code, out, err) = run(&fx, &["find", ".", "--min", "lots"]);
    assert_eq!(code, 1, "{out}{err}");
    assert!(format!("{out}{err}").contains("not a size"), "{out}{err}");
    let (code, out, err) = run(&fx, &["find", ".", "--skip"]);
    assert_eq!(code, 1, "{out}{err}");
    assert!(
        format!("{out}{err}").contains("--skip needs a value"),
        "{out}{err}"
    );
}

/// The operating system's trash and system folders are never entered
/// (Phase 13, CORE-09). IDENTIFIED M1: with a copy of a photo in
/// `$RECYCLE.BIN`, older than the live one, the bin's copy was elected
/// keeper and the live photo was the file `milk` would move. Now the bin
/// is not scanned, the live photo has no duplicate, and the SCAN line
/// says what was left out; the person's own freedesktop trash is found
/// by its path, here through `XDG_DATA_HOME`.
#[test]
fn the_bin_is_never_scanned() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("fx");
    let journal = tmp.path().join("journal");
    let photo = blob(41, 300 * 1024);
    put(&root, "photos/IMG_0041.jpg", &photo);
    put_at(
        &root,
        "$RECYCLE.BIN/S-1-5-21-1000/$RX4K2P1.jpg",
        &photo,
        1_577_836_800, // 2020-01-01: the oldest copy, the keeper it was
    );
    put(&root, ".Trash-1000/files/IMG_0041.jpg", &photo);
    put(&root, "System Volume Information/tracking.log", &photo);
    // the person's freedesktop trash: Linux and the BSDs only
    let home_trash = !cfg!(any(windows, target_os = "macos"));
    if home_trash {
        put(&root, "share/Trash/files/IMG_0041.jpg", &photo);
    }
    // a real duplicate beside them still groups
    let note = blob(42, 4096);
    put(&root, "notes/a.txt", &note);
    put(&root, "notes/a (copy).txt", &note);
    let fx = Fx {
        _tmp: tmp,
        root: root.canonicalize().unwrap(),
        journal,
    };
    let go = |args: &[&str]| {
        let out = Command::new(bin())
            .args(args)
            .current_dir(&fx.root)
            .env("DEDUPE_JOURNAL_DIR", &fx.journal)
            .env("XDG_DATA_HOME", fx.root.join("share"))
            .env("NO_COLOR", "1")
            .env("COLUMNS", "200")
            .output()
            .expect("spawn dedupe");
        (
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stdout).into_owned(),
        )
    };
    let (code, out) = go(&["find", "."]);
    assert_eq!(code, 0, "{out}");
    for gone in ["RECYCLE", "Trash", "System Volume"] {
        assert!(!out.contains(gone), "{gone} listed\n{out}");
    }
    assert!(out.contains("1 group"), "{out}");
    assert!(!out.contains("IMG_0041"), "{out}");
    let left_out = if home_trash { 4 } else { 3 };
    assert!(
        out.contains(&format!("{left_out} bin and system folders left out")),
        "{out}"
    );
    let (code, out) = go(&["milk", ".", "--dry-run"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        !out.contains("photos/IMG_0041.jpg"),
        "the live photo stays\n{out}"
    );
}

/// A file a sync client keeps online is not read (Phase 13, CORE-10).
/// Windows lets a program mark a file `OFFLINE` (the recall attributes a
/// sync client sets cannot be set from here; `walk::online_only` treats
/// the three alike, unit-tested): the marked copy is counted on the SCAN
/// line and in the doctor, its twin has no duplicate, and nothing reads
/// it.
#[cfg(windows)]
#[test]
fn a_file_kept_online_is_not_read() {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    extern "system" {
        fn SetFileAttributesW(name: *const u16, attributes: u32) -> i32;
    }
    let set = |p: &Path, a: u32| {
        let w: Vec<u16> = p.as_os_str().encode_wide().chain(Some(0)).collect();
        // SAFETY: a NUL-terminated wide path that outlives the call
        assert_ne!(
            unsafe { SetFileAttributesW(w.as_ptr(), a) },
            0,
            "{}",
            p.display()
        );
    };
    let fx = fixture();
    let cloud = fx.root.join("backup/IMG_0041.jpg");
    set(&cloud, 0x0000_1000 | 0x0000_0020); // OFFLINE | ARCHIVE
    let (code, out, _) = run(&fx, &["find", "."]);
    let (dcode, doc, _) = run(&fx, &["doctor"]);
    set(&cloud, 0x0000_0020); // local again, so the folder can go
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("1 online only, not read"), "{out}");
    assert!(
        !out.lines()
            .any(|l| l.trim_end().ends_with("backup\\IMG_0041.jpg")),
        "{out}"
    );
    assert!(
        out.contains("backup\\old\\IMG_0041 (1).jpg"),
        "its twins still group\n{out}"
    );
    assert_eq!(dcode, 0, "{doc}");
    assert!(doc.contains("1 file kept online only"), "{doc}");
}

/// A clock on every ffmpeg and ffprobe child (Phase 13, CORE-11): a
/// stand-in pair on PATH answers `-version` and then sends nothing. Each
/// file it is asked about is stopped after `DEDUPE_STALL` seconds, counted
/// `timed out` on its stage line and named under the totals; the run ends
/// and the byte copies still group. (Unix: the stand-ins are shell
/// scripts; the clock itself is unit-tested on Windows too, `tools`.)
#[cfg(unix)]
#[test]
fn a_stalled_ffmpeg_is_stopped_and_named() {
    use std::os::unix::fs::PermissionsExt;
    use std::time::{Duration, Instant};
    let fx = fixture();
    let bin_dir = fx.root.parent().unwrap().join("fake-bin");
    fs::create_dir_all(&bin_dir).unwrap();
    for name in ["ffmpeg", "ffprobe"] {
        let p = bin_dir.join(name);
        fs::write(
            &p,
            "#!/bin/sh\ncase \"$1\" in -version) echo \"$0 version stand-in\"; exit 0;; esac\nexec sleep 30\n",
        )
        .unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
    }
    put(&fx.root, "clips/b.mp4", &blob(7, 90_000));
    put(&fx.root, "clips/c.mp4", &blob(8, 91_000));
    put(&fx.root, "art/one.jxl", &blob(9, 5_000));
    put(&fx.root, "art/two.jxl", &blob(10, 5_100));
    let path = format!(
        "{}:{}",
        bin_dir.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let t = Instant::now();
    let out = Command::new(bin())
        .args(["find", "."])
        .current_dir(&fx.root)
        .env("DEDUPE_JOURNAL_DIR", &fx.journal)
        .env("DEDUPE_STALL", "1")
        .env("PATH", path)
        .env("NO_COLOR", "1")
        .env("COLUMNS", "200")
        .output()
        .expect("spawn dedupe");
    let took = t.elapsed();
    let text = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{text}");
    assert!(took < Duration::from_secs(25), "{took:?}\n{text}");
    // three videos stand for themselves (a.mp4 keeps its byte copy's place)
    let frames = text.lines().find(|l| l.contains("FRAMES")).unwrap_or("");
    assert!(frames.contains("3 timed out"), "{text}");
    let decode = text.lines().find(|l| l.contains("DECODE")).unwrap_or("");
    assert!(decode.contains("2 timed out"), "{text}");
    assert!(text.contains("timed out: "), "{text}");
    assert!(text.contains("ffmpeg sent nothing for 1 s"), "{text}");
    // the first three by path, then how many more
    let note = text
        .lines()
        .find(|l| l.contains("timed out: "))
        .unwrap_or("");
    for named in ["art/one.jxl", "art/two.jxl", "clips/a.mp4", "and 2 more"] {
        assert!(note.contains(named), "{named}\n{text}");
    }
    // the byte copies did not wait on any of it
    assert!(text.contains("IMG_0041 (1).jpg"), "{text}");
}

/// Visible top-level windows of process `pid`: (title, class).
#[cfg(windows)]
fn windows_of(pid: u32) -> Vec<(String, String)> {
    type Hwnd = *mut std::ffi::c_void;
    #[link(name = "user32")]
    extern "system" {
        fn EnumWindows(f: extern "system" fn(Hwnd, isize) -> i32, p: isize) -> i32;
        fn GetWindowThreadProcessId(h: Hwnd, pid: *mut u32) -> u32;
        fn IsWindowVisible(h: Hwnd) -> i32;
        fn GetWindowTextW(h: Hwnd, s: *mut u16, n: i32) -> i32;
        fn GetClassNameW(h: Hwnd, s: *mut u16, n: i32) -> i32;
    }
    struct Find {
        pid: u32,
        found: Vec<(String, String)>,
    }
    extern "system" fn each(h: Hwnd, p: isize) -> i32 {
        // SAFETY: `p` is the `Find` passed below, alive for the call
        let find = unsafe { &mut *(p as *mut Find) };
        let mut pid = 0u32;
        let mut title = [0u16; 256];
        let mut class = [0u16; 256];
        // SAFETY: a window handle from EnumWindows; buffers of the sizes given
        unsafe {
            GetWindowThreadProcessId(h, &mut pid);
            if pid == find.pid && IsWindowVisible(h) != 0 {
                let t = GetWindowTextW(h, title.as_mut_ptr(), 256).max(0) as usize;
                let c = GetClassNameW(h, class.as_mut_ptr(), 256).max(0) as usize;
                find.found.push((
                    String::from_utf16_lossy(&title[..t]),
                    String::from_utf16_lossy(&class[..c]),
                ));
            }
        }
        1
    }
    let mut find = Find {
        pid,
        found: Vec::new(),
    };
    // SAFETY: the callback reads `find` only during this call
    unsafe {
        EnumWindows(each, &mut find as *mut Find as isize);
    }
    find.found
}

/// A file the Recycle Bin is set not to hold (Phase 13, TECH-11).
/// `purge` promises never to delete outright. With the `trash` crate's
/// flags a 5 MB duplicate against a bin capped at 1 MB held `purge` on the
/// shell's "Delete File" dialog, which asks to delete permanently (CI run
/// 43226616276); `recycle` asks the shell without that warning and stops
/// any file the shell will not recycle. Two settings of the temp volume's
/// bin (`HKCU\…\BitBucket\Volume\{GUID}`): capped at 1 MB
/// (`MaxCapacity`), and set to delete at once (`NukeOnDelete`). Either way
/// the duplicate must end on disk, refused, or in the bin, restored
/// byte-identical by `--undo` — never gone, never a dialog. Each setting
/// is put back after. It changes the machine's settings, so it runs only
/// where the machine is disposable: CI sets `DEDUPE_TEST_BIN_CAP=1`.
#[cfg(windows)]
#[test]
#[ignore]
fn the_bin_refusing_a_file_leaves_it_on_disk() {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::time::{Duration, Instant};
    if std::env::var_os("DEDUPE_TEST_BIN_CAP").is_none() {
        eprintln!("DEDUPE_TEST_BIN_CAP not set: the bin's settings are left alone; skipped");
        return;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetVolumePathNameW(file: *const u16, out: *mut u16, len: u32) -> i32;
        fn GetVolumeNameForVolumeMountPointW(mount: *const u16, out: *mut u16, len: u32) -> i32;
    }
    let wide = |p: &Path| -> Vec<u16> { p.as_os_str().encode_wide().chain(Some(0)).collect() };
    let reg = |args: &[&str]| Command::new("reg").args(args).output().expect("reg.exe");
    for (value, setting) in [("MaxCapacity", "1"), ("NukeOnDelete", "1")] {
        let fx = fixture();
        // the fixture's volume and its GUID
        let mut mount = vec![0u16; 300];
        let mut name = vec![0u16; 100];
        // SAFETY: NUL-terminated input; output buffers of the lengths given
        unsafe {
            assert_ne!(
                GetVolumePathNameW(wide(&fx.root).as_ptr(), mount.as_mut_ptr(), 300),
                0
            );
            assert_ne!(
                GetVolumeNameForVolumeMountPointW(mount.as_ptr(), name.as_mut_ptr(), 100),
                0
            );
        }
        let name =
            std::ffi::OsString::from_wide(&name[..name.iter().position(|&c| c == 0).unwrap()])
                .to_string_lossy()
                .into_owned();
        let guid = &name[name.find('{').unwrap()..=name.find('}').unwrap()];
        let key = format!(
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\BitBucket\Volume\{guid}"
        );
        let old = String::from_utf8_lossy(&reg(&["query", &key, "/v", value]).stdout)
            .lines()
            .find(|l| l.contains(value))
            .and_then(|l| l.split_whitespace().last().map(str::to_string));
        assert!(reg(&[
            "add",
            &key,
            "/v",
            value,
            "/t",
            "REG_DWORD",
            "/d",
            setting,
            "/f"
        ])
        .status
        .success());
        let big = blob(77, 5 * 1024 * 1024);
        put(&fx.root, "big/keep.bin", &big);
        put(&fx.root, "big/copy/keep.bin", &big);
        // a dialog would hold the run: a clock
        let mut child = Command::new(bin())
            .args(["purge", "big", "."])
            .current_dir(&fx.root)
            .env("DEDUPE_JOURNAL_DIR", &fx.journal)
            .env("NO_COLOR", "1")
            .env("COLUMNS", "200")
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("spawn dedupe");
        let t0 = Instant::now();
        let status = loop {
            if let Some(s) = child.try_wait().unwrap() {
                break Some(s);
            }
            if t0.elapsed() > Duration::from_secs(120) {
                for (title, class) in windows_of(child.id()) {
                    eprintln!("window of the purge: {title:?}  class {class:?}");
                }
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            std::thread::sleep(Duration::from_millis(200));
        };
        let mut out = String::new();
        use std::io::Read as _;
        let _ = child.stdout.take().unwrap().read_to_string(&mut out);
        // the setting put back before anything is asserted
        match &old {
            Some(v) => {
                let v = u32::from_str_radix(v.trim_start_matches("0x"), 16).unwrap_or(0);
                reg(&[
                    "add",
                    &key,
                    "/v",
                    value,
                    "/t",
                    "REG_DWORD",
                    "/d",
                    &v.to_string(),
                    "/f",
                ]);
            }
            None => {
                reg(&["delete", &key, "/v", value, "/f"]);
            }
        }
        eprintln!("{value} = {setting}\n{out}");
        let copy = fx.root.join("big/copy/keep.bin");
        assert!(fx.root.join("big/keep.bin").exists(), "the keeper\n{out}");
        let status = status.expect("purge did not end in 120 s: a dialog may be waiting");
        if copy.exists() {
            // refused: left in place, and said
            assert_eq!(status.code(), Some(2), "{out}");
            assert!(out.contains("Recycle Bin refused it"), "{out}");
            eprintln!("{value} = {setting}: refused, left on disk");
        } else {
            // taken: in the bin, and back from it whole
            assert_eq!(status.code(), Some(0), "{out}");
            let (code, undo, _) = run(&fx, &["--undo"]);
            assert_eq!(code, 0, "{undo}");
            assert!(
                copy.exists(),
                "binned and not restorable: deleted outright\n{out}\n{undo}"
            );
            eprintln!("{value} = {setting}: in the Recycle Bin, restored by --undo");
        }
        assert_eq!(fs::read(&copy).unwrap(), big);
    }
}

/// The registry key of the Recycle Bin of the volume `path` is on.
#[cfg(windows)]
fn bin_key(path: &Path) -> String {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    #[link(name = "kernel32")]
    extern "system" {
        fn GetVolumePathNameW(file: *const u16, out: *mut u16, len: u32) -> i32;
        fn GetVolumeNameForVolumeMountPointW(mount: *const u16, out: *mut u16, len: u32) -> i32;
    }
    let w: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut mount = vec![0u16; 300];
    let mut name = vec![0u16; 100];
    // SAFETY: NUL-terminated input; output buffers of the lengths given
    unsafe {
        assert_ne!(GetVolumePathNameW(w.as_ptr(), mount.as_mut_ptr(), 300), 0);
        assert_ne!(
            GetVolumeNameForVolumeMountPointW(mount.as_ptr(), name.as_mut_ptr(), 100),
            0
        );
    }
    let name = std::ffi::OsString::from_wide(&name[..name.iter().position(|&c| c == 0).unwrap()])
        .to_string_lossy()
        .into_owned();
    let guid = &name[name.find('{').unwrap()..=name.find('}').unwrap()];
    format!(r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\BitBucket\Volume\{guid}")
}

/// What the shell does with a file its bin is capped below, by the flags
/// the operation is given (Phase 13, TECH-11): each callback, whether the
/// file is still on disk, and whether the bin lists it. A measurement for
/// the record, not a test of `purge`; on a disposable machine only
/// (`DEDUPE_TEST_BIN_CAP=1`). Run before the test that asserts.
#[cfg(windows)]
#[test]
#[ignore]
fn a_bin_too_small_measured_by_flags() {
    if std::env::var_os("DEDUPE_TEST_BIN_CAP").is_none() {
        eprintln!("DEDUPE_TEST_BIN_CAP not set; skipped");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let key = bin_key(tmp.path());
    let reg = |args: &[&str]| Command::new("reg").args(args).output().expect("reg.exe");
    reg(&[
        "add",
        &key,
        "/v",
        "MaxCapacity",
        "/t",
        "REG_DWORD",
        "/d",
        "1",
        "/f",
    ]);
    // FOF_NO_UI 0x614, FOF_ALLOWUNDO 0x40, FOFX_RECYCLEONDELETE 0x80000,
    // FOF_WANTNUKEWARNING 0x4000 (the last would ask in a dialog: left out)
    for (label, flags) in [
        (
            "NO_UI | ALLOWUNDO | RECYCLEONDELETE",
            0x614 | 0x40 | 0x8_0000u32,
        ),
        ("NO_UI | ALLOWUNDO", 0x614 | 0x40),
    ] {
        let p = tmp.path().join(format!("big-{flags:x}.bin"));
        fs::write(&p, blob(78, 5 * 1024 * 1024)).unwrap();
        let p = p.canonicalize().unwrap();
        let events = dedupe::recycle::trace(&p, flags);
        let shown = dedupe::util::strip_verbatim(&p);
        let in_bin = trash::os_limited::list()
            .map(|items| items.iter().any(|i| i.original_path() == shown))
            .unwrap_or(false);
        eprintln!(
            "MaxCapacity 1 MB, a 5 MB file, {label}: {events:?}; on disk {}, in the bin {in_bin}",
            p.exists()
        );
    }
    // the guard alone (no check before the shell): the file the bin lets
    // go is put back from its second name
    let fx = tmp.path().join("guarded");
    fs::create_dir_all(&fx).unwrap();
    fs::write(fx.join("big.bin"), blob(79, 5 * 1024 * 1024)).unwrap();
    let p = fx.join("big.bin").canonicalize().unwrap();
    let guard = fx.join("_milked").join(".purging");
    let left = dedupe::recycle::to_bin_guard_only(&[p.as_path()], &guard);
    eprintln!("the guard alone: {left:?}; on disk {}", p.exists());
    reg(&["delete", &key, "/v", "MaxCapacity", "/f"]);
    assert!(
        p.exists(),
        "the bin let it go and the guard did not put it back"
    );
    assert_eq!(fs::read(&p).unwrap(), blob(79, 5 * 1024 * 1024));
    assert!(
        left.iter()
            .any(|(q, e)| q == &p && e.words().contains("put back")),
        "{left:?}"
    );
    assert!(!guard.exists(), "the guard folder stays behind");
    assert!(
        !fx.join("_milked").exists(),
        "`_milked` made for the guard stays behind"
    );
}
