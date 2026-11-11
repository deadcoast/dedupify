# dedupe — the Dedupelifier

A duplicate finder for the terminal. One run finds exact duplicates,
near-duplicate images (rescaled, re-encoded, mirrored, rotated, cropped,
letterboxed) and near-duplicate videos (re-encoded, scaled, trimmed,
mirrored, boxed in bars; `ffmpeg` and `ffprobe` on PATH or where their
installer put them), says in
words what each one is — `resized 50%`, `cropped 80%`, `another shot,
05:06:08` — and then opens the result as a workspace: every file marked,
`space` to choose, `v` to see the keeper beside it in the terminal, `k`
to change a keeper, `m` to move the chosen into `_milked`, `p` to send
them to the Recycle Bin, `u` to reverse the last change. Nothing is moved
or binned on one family of evidence: the two 64-bit hashes must be
confirmed by PDQ or by the geometric fit (or, in the workspace, by your
own look, labelled as such), and two photographs with different capture
moments — or two frames of one burst — are shown and left alone unless
you cull them yourself (`c`).

Phone and camera files are read as they come: HEIC, HEIF and AVIF (the
tiled photos phones write, decoded through any ffmpeg with HEVC and AV1
decoders) and camera RAWs through the JPEG preview every RAW carries. A
RAW keeps its group and is never moved or binned by the near tier; the
camera's JPEG beside it is shown as `RAW + JPEG` and stays unless you
choose it. `milk` across volumes (a mount point in the tree, or
`_milked` linked to another drive) copies, reads the copy back, compares
it, then deletes the original.
`docs/COMPARISON.md` says what here is the standard pipeline and what is
not.

```
cargo install --path .
```

```
dedupe                      workflow menu
dedupe find [PATH] [.]      list every duplicate group, then decide in the workspace
dedupe milk [PATH] [.]      move duplicates into PATH\_milked
dedupe purge [PATH] [.]     verify bytes, then Recycle Bin
dedupe --undo [PATH]        reverse the last milk or purge
dedupe doctor [PATH]        checks, with a fix line for each
```

`.` includes subfolders; `img` / `vid` restrict the kind; `-n` dry run;
`-q` byte copies only (size and 64 KiB samples, no near pass); `--skip
NAME` leaves a folder or pattern out and remembers it (`--unskip` to
forget); `--min SIZE` leaves out small files for a run; `-h` the help
page.

- `docs/PLAN.md` — the phases, what this build contains, what is next
- `docs/RESEARCH.md` — the mathematics and the papers behind each choice
- `docs/CLI.md` — the resolved grammar, output design, menu, safety model
- `docs/VERIFICATION.md` — what was run in the build environment and what it showed
- `docs/DEEP.md` — the optional learned tier: install, provenance, what it may do
- `docs/COMPARISON.md` — against czkawka, fclones and the hash libraries: same, different, missing
- `docs/IDENTIFIED.md` → `docs/SHELVED.md` → `docs/TODO.md` — the latest review, its items by milestone, the milestone in progress

Nothing is cached or indexed on a scan, and nothing is written into the
scanned folder but `_milked`. A change writes one journal in the OS
local-data folder; `--undo` consumes it. Ctrl-C during a change stops
after the file in hand and the journal still covers every move. Set
`DEDUPE_JOURNAL_DIR` to move the journals, `NO_COLOR=1` for uncoloured
output, `DEDUPE_ASCII=1` for the ASCII glyph set.

`cargo test --release` runs the suite (the video and HEIC tests need
ffmpeg; without it they return early). `cargo test --release --test bench
-- --ignored --nocapture` times each stage on a fixed generated corpus.
`.github/workflows/ci.yml` runs the suite, clippy and rustfmt on Linux
and Windows on every push to `main`.
