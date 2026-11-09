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

/// Run on a pseudo-terminal through `script(1)` with `keys` typed after
/// `delay`, so the workspace bar appears and answers; Unix only, and only
/// where `script` exists. Returns None when it cannot run.
#[cfg(unix)]
fn run_tty(fx: &Fx, args: &[&str], keys: &[&str], columns: u32) -> Option<String> {
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
    let mut child = Command::new("script")
        .args(["-qfec", &cmd, "/dev/null"])
        .current_dir(&fx.root)
        .env("DEDUPE_JOURNAL_DIR", &fx.journal)
        .env("NO_COLOR", "1")
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
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    // strip the terminal control sequences script passes through
    let mut plain = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                while let Some(&n) = chars.peek() {
                    chars.next();
                    if n.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        if c != '\r' {
            plain.push(c);
        }
    }
    Some(plain)
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
    // nothing written
    assert!(!fx.journal.exists());
    assert!(!fx.root.join("_milked").exists());
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
/// and undo restores every one of them. Unix only (SIGINT by `kill`).
#[cfg(unix)]
#[test]
fn interrupted_milk_leaves_a_journal_undo_can_use() {
    use std::io::Read as _;
    use std::process::{Command, Stdio};
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
    let mut child = Command::new(bin())
        .args(["milk", "."])
        .current_dir(&root)
        .env("DEDUPE_JOURNAL_DIR", &journal)
        .env("NO_COLOR", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
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
    // SAFETY: the pid is our own child's, still running (we hold its stdout)
    assert_eq!(unsafe { libc::kill(child.id() as i32, libc::SIGINT) }, 0);
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
    assert!(!fx.journal.exists(), "dry run must not journal");

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
    assert!(!fx.journal.exists());
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

/// A near duplicate the two hashes group but nothing confirms stays where
/// it is for `milk` as well as `purge`: a photograph of the same scene
/// from another position is the measured case (VERIFICATION.md, Phase 6).
#[test]
fn milk_leaves_unconfirmed_near_duplicates_in_place() {
    let fx = fixture_with_photos();
    // the beach with a caption bar over its lower 15 %: the two hashes
    // still land inside their thresholds (d 5, p 14), PDQ (q 36) and the
    // fit (0.78) do not — measured with examples/fit_diag.rs
    let beach = image::open(fx.root.join("photos/beach.png"))
        .unwrap()
        .to_rgb8();
    let mut edited = beach.clone();
    let (w, h) = (edited.width(), edited.height());
    for y in (h as f32 * 0.85) as u32..h {
        for x in 0..w {
            edited.put_pixel(x, y, image::Rgb([40, 40, 48]));
        }
    }
    put(&fx.root, "photos/beach_bar.png", &png_bytes(&edited));
    let (code, out, _) = run(&fx, &["find", ".", "img"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("near  photos/beach_bar.png") || out.contains("near  photos\\beach_bar.png"),
        "the hashes group it\n{out}"
    );
    assert!(
        out.contains("q  36") && out.contains("fit 0.7"),
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
    let st = Command::new("ffmpeg")
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

/// The result is the workspace: `find` on a terminal ends in the bar; `n`
/// previews, `m` + enter moves what is on screen without a second scan,
/// `u` reverses it, `q` leaves. Piped, the `next` lines print instead.
#[cfg(unix)]
#[test]
fn find_on_a_terminal_acts_from_the_result() {
    let fx = fixture_with_photos();
    let before = files(&fx.root);
    let Some(out) = run_tty(&fx, &["find", ".", "img"], &["n", "m", "\r", "u", "q"], 160) else {
        eprintln!("no script(1); skipped");
        return;
    };
    assert!(out.contains("m milk"), "the bar\n{out}");
    assert!(out.contains("p purge"), "{out}");
    assert!(out.contains("k keeper"), "{out}");
    // n: the preview, with the native dry-run command named
    assert!(out.contains("dedupe milk . img --dry-run"), "{out}");
    assert!(out.contains("would~"), "{out}");
    // m, enter: the move, on the groups already listed (one SCAN line only)
    assert_eq!(out.matches("SCAN ").count(), 1, "no second scan\n{out}");
    assert!(out.contains("7 moved"), "{out}");
    assert!(out.contains("u undo"), "{out}");
    // u: reversed on the spot
    assert!(out.contains("7 restored"), "{out}");
    assert_eq!(files(&fx.root), before, "tree back as it was");
    // piped, no bar, the typed forms instead
    let (_, piped, _) = run(&fx, &["find", ".", "img"]);
    assert!(!piped.contains("m milk"), "{piped}");
    assert!(piped.contains("next   dedupe milk . img"), "{piped}");
}
