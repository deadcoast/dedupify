# IDENTIFIED — review of `dedupe` against its design

```
[IDENTIFIED + PLAN] -> [SHELVED] -> [TODO]
```

This document is the first link of the chain. It is written by the
`identify` workflow (review → state vs design → strengths and weaknesses →
five improvement lists) and is read by `SHELVED.md`, which orders its
actionable items under the `PLAN.md` milestones. Every item carries an ID
(`NF-`, `SC-`, `QOL-`, `CORE-`, `TECH-`) so SHELVED and TODO can point at
it. A new IDENTIFIED pass runs only when every PLAN milestone is
delivered.

Review run: 2026-11-09, on `main` at `780e3cc` (Phase 8 delivered), 13,628
lines of Rust (src 10,339, tests 3,289), Linux suite and the Windows run
of Phase 7 as evidence.

---

## 1. Current state vs intended design

| Design requirement (brief, `docs/CLI.md` [SPEC]) | State | Evidence / gap |
|---|---|---|
| `find` — in-depth search ending with an actionable next step | **met (Phase 8)** | ends on the `m p n k u q` bar; one change per sitting only (QOL-01) |
| `milk` — move duplicates into `_milked` | met | `fs::rename` only — fails across volumes inside the root (CORE-05) |
| `purge` — largest, strongest, most accurate suite | met | byte verify, two-family rule, Recycle Bin, undo |
| `.` recursive, `img` / `vid` tokens | met | one depth rule since Phase 8 |
| `--help` proprietary designed page | met | fits 80 cells |
| `--doctor` troubleshooting suite | met | does not check HEIC decodability or volume layout (QOL-09) |
| `--dry-run`, `--quick-search`, `--undo` | met | undo reverses the newest journal only (QOL-05) |
| bare `dedupe` → opinionated workflow menus (FUOE) | partial | the menu builds an argv and hands off; its Find caption still says "prints groups, changes nothing" (`src/menu.rs:339`, SC-04) |
| no bloated menus; most use is quick sort / re-sort | partial | no remembered last folder or one-key repeat (QOL-03) |
| guardrails only for safety, no caches/locks in directories | met | journal lives in `%LOCALAPPDATA%`, nothing written in scanned folders except `_milked` |
| every decision backed by research and measurement | met | RESEARCH, VERIFICATION per phase, conformance with Meta's PDQ/TMK |
| designed CLI visuals, no emoji | met | words-first lines since Phase 8; colour on Windows unverified (QOL-10) |
| runs from the terminal on Windows | **met (Phase 7)** | build, suite, Recycle Bin round trip on the author's machine; Ctrl-C on Windows untested (TECH-07) |
| creative, novel enhancement over forking a proven tool | met in the engine | two-family acting rule, geometric fit, capture moment, bar cropping, result workspace (COMPARISON §5) |
| phone camera rolls | **gap** | `heic`/`heif` are in `IMAGE_EXT` but the `image` crate has no decoder for them: they are counted and silently undecodable; AVIF not listed (NF-01) |
| one coherent doc set | **gap** | the repo's `RESEARCH.md` became `RESEARCH-01.md` and `RESEARCH-02.md` (identical) and `VERIFICATION.md` became `-1`/`-2` (`-2` a stale subset); 47 references across docs and source still name the old files (SC-01) |

---

## 2. Strengths

- **The engine is measured, not asserted.** PDQ is bit-identical to Meta's
  reference given the same input; TMK reproduces Meta's `.tmk` files to
  float rounding; thresholds come from procedural and real corpora with
  the negatives' closest pairs recorded.
- **Two families before any action.** Hash pair + (PDQ or geometric fit),
  or descriptor + fit. On real photographs this is what refused the
  chessboard poses the hashes grouped.
- **Reversibility everywhere.** Journal written as it goes (atomic
  rename every 250 ms), `_milked` staging, Recycle Bin, `--undo`,
  interrupt-safe journals, `kept~` lines with reasons.
- **One pipeline, one decode.** An image is decoded once and feeds dHash,
  pHash, PDQ, the capture moment and the embedding; video is one ffmpeg
  pipe per file plus a keyframe-only bar pass.
- **Output is designed.** Fitted widths, words-first relations, evidence
  dim and dropped first, verbs in a fixed column.
- **Cross-platform discipline.** `\\?\` stripping, case-insensitive
  `_milked` on Windows, local dates, Windows type-checks every phase and a
  real Windows run.

## 3. Weaknesses

- **Two god modules.** `actions.rs` (1,242 lines) holds scanning, item
  building, verification, milk, purge, undo and journal plumbing;
  `render.rs` (1,289) holds every screen. Phase 8 had to split milk/purge
  by text surgery.
- **Three parallel group types with no shared interface.** `Group`,
  `NearGroup`, `VideoGroup` each re-implement members, keeper, paths;
  `Found` addresses them by index arithmetic (`set_keeper`,
  `group_paths`).
- **The workspace is shallow.** All-or-nothing actions, one change per
  sitting, no per-group skip, no way to look at a file before deciding.
- **Format coverage stops at the `image` crate.** HEIC/HEIF listed but not
  decodable; AVIF, RAW (CR2/NEF/ARW/DNG) absent.
- **Filesystem edge cases.** `rename` only (no copy+delete across
  volumes); symlinks not followed (correct) but not reported; no
  exclusion of folders the user never wants touched.
- **Doc drift.** Renamed doc files with stale links; duplicated research
  file; menu caption older than Phase 8.
- **Unverified paths.** Ctrl-C on Windows, colour on Windows, keeper
  re-election on video and exact groups through the bar, deep threshold on
  real labelled copies, capture-moment rule on a real camera roll.
- **`unwrap()` density** in `cli.rs` (18), `journal.rs` (18), `exact.rs`,
  `menu.rs` (12 each) — most are on invariants, not all are documented.

---

## 4. Improvement lists

### 4.1 NEW-FEATURES-AND-ADDITIONS

- **NF-01** HEIC/HEIF/AVIF decoding through ffmpeg (already a dependency
  for video): one `ffmpeg -i x.heic -f rawvideo -pix_fmt rgb24` per file,
  same `Decoded` path; EXIF read from the HEIF container. Phone camera
  rolls are the largest real duplicate source.
- **NF-02** RAW stills (CR2, NEF, ARW, DNG, RAF): hash the embedded JPEG
  preview (every RAW carries one), pair a RAW with its own JPEG export as
  `RAW + JPEG` kin-of-a-kind, never acting on the RAW.
- **NF-03** In-terminal preview in the workspace: sixel / kitty graphics
  when the terminal supports them (Windows Terminal 1.22+ does sixel),
  side-by-side keeper and member at thumbnail size; plain fallback opens
  the file with the OS viewer.
- **NF-04** `o` reveal in the workspace: open the selected file's folder
  with the file selected (`explorer /select,`), for a human check.
- **NF-05** Burst and series handling: consecutive `kin` shots grouped as a
  series with a "keep the sharpest" option (Laplacian variance on the
  288 grid already decoded).
- **NF-06** Report export: `find --report out.html` writing the result
  with thumbnails, for review away from the terminal.
- **NF-07** Audio duplicates (re-encoded MP3/FLAC/AAC) via Chromaprint-
  style fingerprints from ffmpeg-decoded PCM; listed in COMPARISON as
  missing against czkawka.
- **NF-08** Folder-pair mode: `dedupe find A B` — what in B already exists
  in A (the import/backup question), acting only on B.
- **NF-09** Optional model fetch: `dedupe --doctor --get-model` downloads
  the SSCD ONNX with a pinned SHA-256 instead of the manual DEEP.md steps.

### 4.2 SOURCE-CODE-CHANGES

- **SC-01** Doc integrity: one `RESEARCH.md`, one `VERIFICATION.md` (or
  keep the split and make it intentional: `-01` = phases 1–4, `-02` =
  5–8), delete the duplicate, fix the 47 stale references across README,
  docs and source comments.
- **SC-02** Split `actions.rs` into `scan.rs` (scan_and_group, Found),
  `plan.rs` (items, verify), `act.rs` (milk, purge, undo); commands become
  thin compositions — what the workspace already needs.
- **SC-03** Split `render.rs` by screen: `render/stages.rs`,
  `render/groups.rs`, `render/totals.rs`, `render/hints.rs`.
- **SC-04** Stale strings: the menu's Find step description
  (`menu.rs:339`), any remaining "change nothing" copy.
- **SC-05** A `GroupView` trait (members, keeper, paths, `set_keeper`,
  words for member *i*) implemented by the three group types; `Found`
  iterates groups instead of offset arithmetic.
- **SC-06** `unwrap()` audit: each one either an `expect("invariant: …")`
  or handled; start with `journal.rs` and `cli.rs`.
- **SC-07** Typed errors for the action layer (`ActError::{Collision,
  CrossDevice, Vanished, Permission, Bin}`) instead of strings, so the
  workspace can offer a retry or a fallback per kind.
- **SC-08** Remove the committed UTF-16 `runs/win/logs` churn: logs written
  UTF-8 by the script (`Out-File -Encoding utf8`), `runs/win/logs/` either
  ignored or committed only by the verification step.

### 4.3 QOL-ENHANCEMENTS

- **QOL-01** Per-group decisions in the workspace: `space` toggles a group
  (or a member) in or out, the bar counts follow, `m`/`p` act on the
  selection.
- **QOL-02** Several changes per sitting: after `m` or `p` the remaining
  groups stay on the bar; `u` undoes the last one.
- **QOL-03** Remember the last folder and kind: bare `dedupe` offers
  "again: C:\Photos  img" as the first menu line (stored beside the
  journals, never in the scanned folder).
- **QOL-04** Scroll and focus in long results: `j`/`k` or arrows move a
  cursor over groups when the list exceeds the screen; `g` jumps to a
  group number.
- **QOL-05** Undo history: `--undo` lists the last N journals for the
  folder with counts and dates and undoes the chosen one, not only the
  newest.
- **QOL-06** Exclusions without config files in the tree: `--skip
  <name>` repeated, and a remembered skip list in the app dir.
- **QOL-07** ETA on long stages (DECODE, FRAMES) from the running rate.
- **QOL-08** Size floor: `--min 100k` to ignore thumbnails and icons in
  app folders.
- **QOL-09** Doctor checks for HEIC decode (ffmpeg), the volume of `_milked`
  vs the files (cross-device warning), and the model's SHA-256.
- **QOL-10** Verify and, if needed, fix colour on the Windows console
  (`ansi_support` enablement path) with a screenshot in VERIFICATION.

### 4.4 CORE-FUNCTION-IMPROVEMENTS

- **CORE-01** Robust fit: trimmed / rank NCC so a local edit (the ELA
  pair, a caption bar) does not sink a true derivative; measured against
  the same negatives as FIT_MIN.
- **CORE-02** Mirror search in the geometric fit (mirrored crops are
  currently left to PDQ alone).
- **CORE-03** Non-black bars for video: detect uniform-colour and blurred
  "ambient" bars by row/column variance, not luma ≤ 24 only.
- **CORE-04** Deep threshold on real labelled copies (carried since
  Phase 4): measure DEEP_MIN on a real set, not procedural only.
- **CORE-05** Cross-volume `milk`: copy, fsync, verify, delete when
  `rename` returns EXDEV / ERROR_NOT_SAME_DEVICE; journal records it.
- **CORE-06** Exact-tier sampling on huge files: a third sample at the
  middle for files over 1 GB (head+tail misses mid-file edits in disk
  images and archives until the full hash).
- **CORE-07** Keeper policy by provenance: prefer files outside
  `exports`/`copy`/`(1)`-style names and outside download/temp folders
  before "oldest", measured on the sample tree.
- **CORE-08** Capture-moment rule on a real camera roll (carried): verify
  SubSecTime handling and bursts at the same second.

### 4.5 TECHNICAL-MODULE-ENHANCEMENTS

- **TECH-01** `decode.rs`: one module owning every decoder (image crate,
  ffmpeg stills, RAW previews), returning the shared `Decoded`.
- **TECH-02** Reduced decode in DECODE: each file is decoded at full
  resolution and box-prefiltered down to a 576–1151 px short side (pixels
  are dropped per file; only signatures are kept). JPEG's DCT-domain
  scaling (1/2, 1/4, 1/8 at decode time) would reach that size directly —
  less time and a lower peak per thread on 24-megapixel files; must be
  re-measured against the PDQ conformance and robustness tables.
- **TECH-03** `workspace.rs` state machine: explicit states (Browse,
  Confirm, Acted, Keeper) instead of nested matches, testable without a
  pseudo-terminal.
- **TECH-04** Pseudo-terminal tests on Windows: ConPTY-driven e2e so the
  bar's key loop is tested where it ships.
- **TECH-05** Benchmarks in-tree (`cargo bench` or an ignored test) for
  SCAN/HASH/DECODE/FRAMES on fixed corpora, recorded per phase.
- **TECH-06** `video.rs` (1,266 lines) split: `video/probe.rs`,
  `video/tmk.rs`, `video/bars.rs`, `video/group.rs`.
- **TECH-07** Ctrl-C on Windows: `GenerateConsoleCtrlEvent`-driven e2e
  mirroring the Unix SIGINT test.
- **TECH-08** CI: GitHub Actions matrix (ubuntu, windows) running the
  suite with ffmpeg installed, so Windows stops depending on a manual run.

---

## 5. Next

`SHELVED.md` takes the actionable items above and orders them under the
`PLAN.md` milestones (Phases 9–12).
