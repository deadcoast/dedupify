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
        .output()
        .expect("spawn dedupe");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
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
    assert!(out.contains("NEAR 003   5 similar"), "{out}");
    assert!(out.contains("q "), "PDQ distance column\n{out}");
    assert!(
        out.contains("beach_crop"),
        "the 5 % crop groups on the two 64-bit hashes\n{out}"
    );
    assert!(
        out.contains("keep  photos/beach.png") || out.contains("keep  photos\\beach.png"),
        "{out}"
    );
    assert!(out.contains("mirror"), "{out}");
    assert!(out.contains("+GROUP 002"), "{out}");
    assert!(out.contains("1 near group   4 similar"), "{out}");
    assert!(out.contains("1 flat, skipped"), "{out}");
    assert!(
        !out.contains("forest"),
        "a different picture must not join the group\n{out}"
    );
    assert!(out.contains("DECODE") && out.contains("MATCH"), "{out}");
}

#[test]
fn near_pass_is_off_for_plain_find_and_on_for_purge() {
    let fx = fixture_with_photos();
    let (code, out, _) = run(&fx, &["find", "."]);
    assert_eq!(code, 0);
    assert!(!out.contains("NEAR"), "{out}");
    assert!(out.contains("near   dedupe find . img"), "hint line\n{out}");
    let (code, out, _) = run(&fx, &["purge", ".", "-n"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("NEAR"),
        "purge runs the comprehensive suite\n{out}"
    );
    assert!(out.contains("would~"), "{out}");
    assert!(out.contains("not a byte copy"), "{out}");
}

#[test]
fn milk_img_moves_near_duplicates_and_undo_restores() {
    let fx = fixture_with_photos();
    let before = files(&fx.root);
    let (code, out, _) = run(&fx, &["milk", ".", "img"]);
    assert_eq!(code, 0, "{out}");
    // 2 + 1 exact, 3 confirmed near, plus the crop: milk takes the two-hash grouping
    assert!(out.contains("7 moved"), "{out}");
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
    // video group: zoom (keeper, longest and most pixels), phone copy, mirror, cut
    assert!(out.contains("VIDEO 003   4 similar"), "{out}");
    assert!(
        out.contains("keep  video/zoom.mp4") || out.contains("keep  video\\zoom.mp4"),
        "{out}"
    );
    assert!(out.contains("mirror"), "{out}");
    assert!(out.contains("trim"), "{out}");
    assert!(
        !out.contains("pattern.mp4"),
        "unrelated clip must stay out\n{out}"
    );
    assert!(out.contains("1 video group   3 similar"), "{out}");
    assert!(out.contains("keep: longest, then most pixels"), "{out}");
}

#[test]
fn video_pass_off_for_plain_find_and_on_for_purge() {
    if !ffmpeg_ok() {
        return;
    }
    let fx = fixture_with_videos();
    let (code, out, _) = run(&fx, &["find", "."]);
    assert_eq!(code, 0);
    assert!(!out.contains("VIDEO"), "{out}");
    assert!(
        out.contains("video   dedupe find . vid"),
        "hint line\n{out}"
    );
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
    // exact: zoom (1), clips/copy/a.mp4; near: phone, mirror, cut
    assert!(out.contains("5 moved"), "{out}");
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
