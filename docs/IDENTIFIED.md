# IDENTIFIED — review of `dedupe` against its design

```
[IDENTIFIED + PLAN] -> [SHELVED] -> [TODO]
```

This document is the first link of the chain. It is written by the
`identify` workflow (review → state vs design → strengths and weaknesses →
five improvement lists) and is read by `SHELVED.md`, which orders its
actionable items under the `PLAN.md` milestones. Every item carries an ID
(`NF-`, `SC-`, `QOL-`, `CORE-`, `TECH-`) so SHELVED and TODO can point at
it. IDs continue from the previous pass (2026-11-09, `780e3cc`); an item
that pass left unplaced keeps its ID. A new IDENTIFIED pass runs only when
every PLAN milestone is delivered.

Review run: 2026-11-12, on `main` at `f9227ee` (Phases 1–12 delivered).
26,480 lines of Rust: `src` 19,010 (12,691 of them code, outside tests
and comments), `tests` and `examples` 7,470. Evidence: the Linux suite
(128 unit tests, the band test, 32 end-to-end), CI runs 43115899107 and
43120230190 (Ubuntu 26.04 and Windows Server 2025, both green), the
owner's two Windows runs of 2026-11-11 and 2026-11-12, and three
measurements made for this review, named in place (**M1**–**M3**).

Every item in § 4 is the agent's proposal **[CLAUDE]**; where an item
closes a debt a VERIFICATION section left open, that section is named.

---

## 1. Current state vs intended design

| Design requirement (brief, `docs/CLI.md` [SPEC]) | State | Evidence / gap |
|---|---|---|
| `find` — in-depth search ending in an actionable next step | met | the result opens as the workspace (CLI §2a); 32 end-to-end tests, one driving it by keys on Linux and Windows |
| `milk` — duplicates into `_milked` | met | rename within a volume, verified copy across volumes (`fsops.rs`); undo crosses back |
| `purge` — the largest, strongest, most accurate suite | partial | byte verify, two-family rule, Recycle Bin, undo; the bin **refusing** a file (`m milk the N the bin refused`) was never driven (VERIFICATION Phase 10, Not verified) — TECH-11 |
| `.` recursive, `img` / `vid` | met | one depth rule since Phase 8 |
| `--help` designed page | met | fits 80 cells |
| `--doctor` troubleshooting suite | met | 15 checks on the owner's machine, every one `PASS` (VERIFICATION Phase 12 § 10) |
| `--dry-run`, `--quick-search`, `--undo` | met | `--undo` chooses among pending changes |
| bare `dedupe` → opinionated workflow menus | met | five workflows plus `again` (`menu.rs`, `recent.rs`) |
| guardrails only for safety; nothing cached or written in the tree | **gap** | nothing is written in the tree, but the walker enters the operating system's own trash: **M1** — a byte copy in `$RECYCLE.BIN` (or `.Trash-1000`) is elected keeper and the live photo is the one `milk` would move (CORE-09). Files a sync client keeps online only are read, which downloads them (CORE-10) |
| every decision backed by research and measurement | partial | conformance with Meta's PDQ and TMK, thresholds measured per phase; still open: the ONNX export against Meta's TorchScript model (VERIFICATION Phase 4) — TECH-14; any library larger than the 1,492-file bench — TECH-10 |
| designed CLI visuals, no emoji | partial | words first, fitted columns, the theme measured exact on Windows Terminal; **M3** — the listing runs past the edge under 70 columns (8 lines at 50, 1 at 60) — QOL-11 |
| runs from the terminal on Windows | met | CI on Windows Server 2025; the owner's runs: 160 tests, Ctrl-C, the workspace by keys, colour; `v` (sixel) never photographed there — TECH-13 |
| creative, novel enhancement over forking a proven tool | met in the engine | two-family acting rule, geometric fit with cells and mirror, capture moment and burst frames, bars on stills and video, the workspace (COMPARISON §5) |
| phone camera rolls | partial | HEIC/HEIF/AVIF through ffmpeg, RAW previews, `RAW + JPEG`; a camera roll in OneDrive or iCloud is mostly online-only placeholders (CORE-10); a whole roll never run (VERIFICATION Phase 11) |
| one coherent doc set | partial | one set since Phase 9; drift since Phase 11: CLI §2 and RESEARCH §3 still say the video pass needs ffmpeg "on PATH", RESEARCH §3 gives the ffmpeg concurrency as ⌈cores/2⌉ where `video::concurrency` floors it (SC-11) |

---

## 2. Strengths

- **Measured, then shipped.** Every threshold has its corpus in
  VERIFICATION; PDQ bit-identical to Meta's reference given the same
  input, TMK to float rounding; the deep band (0.60–0.70) set against
  5.44 million unrelated pairs topping out at 0.526.
- **No `unwrap()` outside tests**, and clippy denies a new one
  (`lib.rs`); the 13 `expect`s left each state the invariant they rely on.
- **Two platforms under test on every push.** CI's first run found what
  no local run could (the pseudo-console child drawing into the parent's
  redirected output) and the fix was proven on the next.
- **Reversibility end to end.** One journal per change, rewritten
  atomically every 250 ms, interrupt-safe on Unix and Windows (Ctrl-C
  verified on both), verified copies across volumes, undo that crosses
  back.
- **The structure follows the work.** `actions`, `render`, `video` are
  folders by concern since Phase 12; a `GroupView` makes the workspace
  indifferent to the group's kind.
- **Formats as they arrive from devices**: tiled HEIC on any ffmpeg with
  HEVC (the container is read here), RAW previews from 18 cameras' files,
  AVIF, and the DECODE line names what could not decode and why.

## 3. Weaknesses

- **The scan trusts the tree.** Hidden and system folders are entered
  like any other: the OS trash (**M1**), `System Volume Information`
  (counted `not listed`), sync clients' online-only files (read, so
  downloaded). `walk.rs:85` skips only `_milked` and the skip list.
- **Child processes have no clock.** ffprobe, the bar passes and the
  frame pipe (`video/probe.rs:98`, `video/bars.rs:101,285`,
  `video/mod.rs:130`, `decode.rs:366`) wait for ffmpeg as long as it
  runs; one file that stalls it (a damaged stream, a share gone away)
  stalls the run, with no line saying which.
- **The journal is atomic against a kill, not against power loss.**
  `journal::rewrite` writes the `.tmp` and renames it without flushing
  it to disk first; after a power cut the renamed journal can be empty,
  and the change it described cannot be undone.
- **The fit is the expensive question.** **M2**: on the bench corpus 100
  of 200 similar pairs went to the geometric fit (PDQ rated those
  procedural pictures under its quality floor of 50); `near::group` took
  2.37 s of the 2.4 s image MATCH, the pair search under a millisecond of
  it; one fit timed alone, 60 ms (`geometry::fit_files`: two decodes of
  4 ms each, the rest the search). Libraries of screenshots, scans and
  skies send their pairs the same way.
- **The bench is not a library.** 1,492 generated files, procedural
  pictures (low PDQ quality, so fit-heavy), 10-second 640 × 360 clips, no
  deep model, 2 vCPUs. Nothing has been timed on 100,000 files, on a
  feature-length 4K video (every source frame is decoded at full size
  before `fps=15` drops it, `video/mod.rs:146`), or with the model on
  Windows (307 ms an image on 2 cores, Phase 4).
- **`near.rs` is the new largest module**: 1,496 lines, 942 of code —
  relations and their words, bars on stills, union-find, keeper order,
  the deep band, the fit dispatch. `workspace/state.rs` follows (737 code
  lines, cursor, selection, modes and screen composition together).
- **Width logic without unit tests.** `render/groups.rs` (491 code lines,
  the shared-date-column allocator), `render/totals.rs` (292),
  `doctor.rs` (431) and `workspace/mod.rs` (578) have no unit test; their
  output is checked only through end-to-end text, at 80 columns or more,
  which is why **M3**'s overflow under 70 has stood since Phase 10.
- **Open measurement debts carried forward**: the ONNX export against
  TorchScript (Phase 4); the Recycle Bin refusing a file (Phase 10); a
  resize during a sitting (Phase 9); PDQ between a large photo and its
  resized copies at 22–24 bits (Phase 11 § 8); the near keeper's "most
  matched" ahead of the copy marks (Phase 11, the fixture's mirrored
  export keeping by one match); dark blurred bars cropped as black in 5
  of 25 real clips (Phase 11 § 7).

---

## 4. Improvement lists

### 4.1 NEW-FEATURES-AND-ADDITIONS

- **NF-10** **Prefer a folder.** In the workspace, `f` on a file makes
  its folder the preferred home: every group with a member under that
  folder re-elects that member keeper (by `GroupView::set_keeper`, the
  group re-related), and the bar names the rule (`keeping in
  Photos\Library`). *Where:* `workspace/state.rs` (a key and an effect),
  `workspace/mod.rs`, `actions/view.rs`. *Why:* a real library's
  duplicates come from imports and backups, and the person knows which
  folder is the master; today that is one `k` per group, which on 600
  groups is 600 keys. *Verify:* state-machine tests (the effect, groups
  without a member there untouched, `f` on a file of the preferred
  folder clearing the rule); an end-to-end run choosing a folder and
  milking, every moved file outside it.
- **NF-06** (carried) report export: `find` writes the result as an HTML
  page with thumbnails. Still unplaced: the workspace's `v` and `o` answer
  the review need inside the terminal; no request for an away-from-
  terminal report has come.
- **NF-07** (carried) audio duplicates (re-encodes of one recording) by a
  Chromaprint-style fingerprint of ffmpeg-decoded PCM. Unplaced: a new
  family of evidence with its own corpus and thresholds, the size of a
  phase.
- **NF-08** (carried) folder-pair mode, `find A B`: what in B exists in
  A, acting only on B. Unplaced: NF-10 answers the same question inside
  one scan without new grammar; revisit if NF-10 falls short.
- **NF-09** (carried, reshaped) **the model fetched by the doctor.** When
  no model is installed and the doctor runs on a terminal, its deep row
  offers `enter: download the model (93.6 MB, SHA-256 checked)`; the file
  is fetched to a `.part`, its size and SHA-256 compared with the
  published file's (`deep::MODEL_SHA256`), then renamed into place. No
  new flag (PLAN's standing rule). *Why:* the deep tier is on in every
  build and used by almost no one: installing it is five manual steps in
  `DEEP.md`. *Verify:* a local HTTP server in a test serving the right
  file, a truncated one and a wrong one: installed, refused, refused, and
  nothing left behind but the right file.
- **NF-11** animated GIF, WebP and APNG in the video pass as well: a
  platform turns a GIF into an MP4 and a download folder holds both.
  Unplaced: no measurement shows how common the pair is in a real
  library.

### 4.2 SOURCE-CODE-CHANGES

- **SC-09** Split `near.rs` as `video` was: `near/relation.rs`
  (`Relation`, its predicates and words, `Boxed`), `near/bars.rs`
  (`picture_bars`, `uniform_band`, `picture_pixels`), `near/group.rs`
  (union-find, the deep band, the fit dispatch, keeper order), `near/mod.rs`
  (`Hashed`, `hash_all`, re-exports). *Why:* CORE-12, CORE-13 and CORE-15
  all change this file; the split comes first so their diffs are
  readable. *Verify:* the Phase 12 method — every line of the old file in
  the new ones (sorted-line comparison), suite and clippy unchanged.
- **SC-10** Unit tests for the width logic: the listing, the totals and
  the stage lines rendered at every width from 40 to 200 on fixed groups
  (exact, near with evidence and a RAW, video with bars, a long root),
  asserting no line exceeds the width in display cells and the date
  column is shared. *Where:* `render/groups.rs`, `render/totals.rs`,
  `render/stages.rs` (`Renderer::capture` already returns lines). *Why:*
  **M3** — the overflow under 70 columns stood for two phases because
  nothing measures it below 80. *Verify:* the test fails on today's code
  at the widths M3 found, passes after QOL-11.
- **SC-11** Doc drift: CLI §2 and RESEARCH §3 on where ffmpeg is looked
  for (PATH, then the installers' folders, `tools.rs`); RESEARCH §3's
  concurrency formula (`video::concurrency` is ⌊cores/2⌋ clamped to 1–4).
  *Verify:* `grep` for "on PATH" in the docs returns only statements that
  are true of `tools.rs`.
- **SC-12** Split `workspace/state.rs` into the state and its keys
  (`state.rs`) and the screen composed from it (`screen.rs`) before
  QOL-12 adds two modes to it. *Verify:* as SC-09; the 9 state tests
  unchanged.

### 4.3 QOL-ENHANCEMENTS

- **QOL-11** Narrow terminals: no line past the edge down to the
  workspace's own floor of 40 columns — the header's root shortened in
  the middle as paths are, the near policy label on lines of its own,
  stage lines dropping segments as they already do above 62. *Why:* a
  split pane in Windows Terminal is 50–60 columns; a line past the edge
  wraps and breaks the in-place redraw. *Verify:* SC-10's test at 40–69.
- **QOL-12** Sort and filter in the workspace: `s` cycles the order
  (reclaimable, folder, kind) and `/` filters the groups to those with a
  path containing the typed text, the bar saying `14 of 612 groups`;
  `a`, `m` and `p` act on what is shown. *Why:* the brief's quick sort /
  re-sort; a result of hundreds of groups is worked folder by folder, and
  scrolling is the only way through it today. *Verify:* state tests
  (order, filter, selection kept across both, actions limited to the
  shown groups); an end-to-end filter then milk.
- **QOL-13** `v` pictures sized right in Windows Terminal: the cell is
  taken as 9 × 18 pixels there (`preview.rs`; the terminal is not queried,
  by design, CLI §2a). If TECH-13's picture shows the pair sized wrong,
  take the cell size from what Windows Terminal is found to report
  instead of assuming it. *Verify:* TECH-13's picture before and after.

### 4.4 CORE-FUNCTION-IMPROVEMENTS

- **CORE-09** **Never enter the operating system's trash or its system
  folders**: `$RECYCLE.BIN`, `RECYCLER`, `System Volume Information`
  (any letter case, on Windows), `.Trash`, `.Trashes`, `.Trash-<uid>`,
  and the user's freedesktop trash (`~/.local/share/Trash`) — passed over
  as `_milked` is, and counted on the SCAN line (`2 system folders`).
  *Where:* `walk.rs` `filter_entry`. *Why:* **M1** — a copy in the trash
  is a file the person already threw away; elected keeper (it was the
  oldest), it makes the live photo the duplicate, and `purge` would bin
  the live photo into the same bin. *Verify:* an end-to-end test with
  copies under each name (M1's fixture): one group without them, no line
  naming them; on Windows, a drive-root scan in the launcher.
- **CORE-10** **Online-only files are not read.** A file a sync client
  keeps in the cloud (Windows: `FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS`,
  `RECALL_ON_OPEN` or `OFFLINE`; macOS: `SF_DATALESS`) is not a candidate:
  reading its bytes downloads it (OneDrive's Files On-Demand, on by
  default, keeps a synced Pictures folder that way). The SCAN line counts them (`3,204 online
  only`), the doctor says how many are under the folder and that making
  them local first lets them be compared. *Where:* `walk.rs` (the
  metadata the walk already reads), `doctor.rs`. *Verify:* unit tests on
  the attribute rule; on the Windows CI runner, a sync root registered
  through the Cloud Files API (`CfRegisterSyncRoot`,
  `CfCreatePlaceholders`) with a placeholder copy of a file: passed over,
  counted, and still a placeholder after the run.
- **CORE-11** **A clock on every child process.** ffprobe, the bar
  passes, the frame pipe and the still-image route stop a child that
  sends nothing for 60 s (the frame pipe) or runs past a ceiling (the
  others), kill it, and count the file as `timed out` on its stage line
  by name in the footer. *Where:* `tools.rs` (one runner the five call
  sites share), `video/*`, `decode.rs`. *Why:* one stalled
  ffmpeg holds the run forever, and its file is never named. *Verify:* an
  end-to-end test with a stand-in `ffmpeg` and `ffprobe` on PATH that
  sleep: the run ends, the files are named `timed out`, the other files
  are grouped as before.
- **CORE-12** PDQ at a fixed working size: the hash is computed on the
  shipped route's per-file reduced picture, whose size depends on the
  file, so a large photo and its resized copies differ by 22–24 bits
  (VERIFICATION Phase 11 § 8, left open for this pass). Hash every file at
  one working size and re-measure: copies' distances, the negatives'
  closest pairs, and conformance (bit-identity with the reference where
  the reference's own input size is used). *Verify:* the Phase 11 § 8
  tables re-run; ships only if the copies' distances fall and no negative
  comes under 31.
- **CORE-13** The near keeper's degree: "most matched" counts hash pairs,
  so a copy with one more near-threshold match outranks the original
  (Phase 11, left open). Count only confirmed relations (two families)
  in the degree, and put the copy marks ahead of degree when degrees are
  within one. *Verify:* the Phase 11 fixture with both caption-bar edits:
  `photos/` keeps; the keeper tables of Phases 6–11 unchanged otherwise.
- **CORE-14** Dark blurred bars: the black pass takes a dark "ambient"
  bar for black and crops wrong in 5 of 25 real clips (Phase 11 § 7).
  After the black pass, test the found band for blur (its detail against
  the picture's, the flat pass's own measure) and hand blurred bands to
  the flat pass. *Verify:* the 25 clips again (the 5 cropped right, the
  20 unchanged), the 79 clips without bars still without.
- **CORE-15** The fit's cost: **M2** — 60 ms a pair, 2.37 s of a 2.4 s
  MATCH. Search coarse to fine (the 7 angles × scales × positions at a
  64-pixel side first, the 128-pixel search only around the best few),
  and reuse each decoded thumbnail across the pairs of one keeper.
  *Verify:* every fit score on the `fit_measure` corpora and the Phase 11
  fixtures within 0.005 of today's and no decision changed; MATCH on the
  bench recorded before and after.
- **CORE-16** Long video's cost: every source frame is decoded at full
  size before `fps=15` drops it; a feature-length 4K HEVC file is tens of
  minutes. Measure FRAMES on real 1080p and 4K files of 1–2 hours, then
  try, measured against the TMK conformance and the video robustness
  tables: hardware decoding (`-hwaccel auto`, falling back to software),
  and skipping the frames no other frame refers to (`-skip_frame
  nonref`), the fps filter filling from their neighbours. *Verify:* the conformance (level 2 =
  1.0000 on Meta's samples) and the Phase 3/11 video tables unchanged;
  the time per hour of footage recorded on the owner's machine.

### 4.5 TECHNICAL-MODULE-ENHANCEMENTS

- **TECH-09** The journal flushed before it is renamed into place
  (`File::sync_all` on the `.tmp`, and on Unix the folder after the
  rename), at most once per 250 ms rewrite. *Where:* `journal::rewrite`.
  *Why:* atomic against a kill today, not against a power cut. *Verify:*
  `strace -f -e trace=fsync,rename` over a `milk` shows each rename
  preceded by the `.tmp`'s fsync; the change's time on the bench's 400
  files recorded before and after.
- **TECH-10** The bench made a measure of real use: `DEDUPE_PHOTO_DIRS`
  and `DEDUPE_VIDEO_DIRS` add real files to the corpus; the model is
  timed when installed; peak memory per stage is read
  (`getrusage` / `GetProcessMemoryInfo`); a scale run generates 100,000
  files. Run on the owner's machine through the launcher (16 threads) as
  well as here. *Why:* every number in the record is from a 2-core box
  and a generated corpus (§ 3). *Verify:* the tables in VERIFICATION,
  with the corpus each came from.
- **TECH-11** The Recycle Bin refusing a file, driven: on the Windows CI
  runner (disposable), the volume's bin capped at 1 MB through its
  registry value, a 5 MB duplicate purged — it must be refused, still on
  disk, offered to `milk`, and never deleted outright. *Why:* `purge`'s
  never-hard-delete promise rests on the `trash` crate's flags
  (`FOF_NO_UI | FOF_ALLOWUNDO | FOF_WANTNUKEWARNING`), never observed with
  a file too large for the bin (Phase 10, Not verified). *Verify:* the
  test itself, in CI.
- **TECH-12** A resize during a sitting, driven: `ResizePseudoConsole`
  in the Windows pseudo-console runner, and on Unix a pseudo-terminal the
  test owns (the `script` route cannot resize). The screen redrawn at the
  new width, the cursor and selection kept. *Verify:* the test, in CI on
  both systems (Phase 9, Not verified).
- **TECH-13** The launcher photographs the workspace and `v`: keys queued
  into its own console's input (`WriteConsoleInput`) before a live `find`
  on the sample — `v` on a near pair, a picture, `q` — so Windows
  Terminal's sixel rendering is seen without the person pressing a key.
  *Verify:* the pictures in VERIFICATION; QOL-13 decided from them.
- **TECH-14** The ONNX export against Meta's TorchScript model, in CI: a
  manually started job installs CPU PyTorch and runs both models (pinned
  SHA-256s) on the same 200 pictures, and the Rust tier on them too;
  cosine between the two exports' embeddings and between ours and the
  ONNX runtime's recorded. *Why:* open since Phase 4; the deep thresholds
  were measured on the export, and its agreement with the published
  model is assumed. *Verify:* the job's table in VERIFICATION; ships a
  note, not code, unless agreement is under 0.99.
- **TECH-15** The deep tier's throughput, after TECH-10's numbers: one
  picture per inference today (`[1, 3, SIZE, SIZE]`, `deep.rs:107`). Batch
  the pictures a worker holds, or give each inference the threads the
  decode does not use, whichever TECH-10's profile points at. *Verify:*
  embeddings bit-identical to single-picture inference; DECODE with the
  model recorded before and after on both machines.

---

## 5. Next

`SHELVED.md` takes the actionable items above and orders them under new
`PLAN.md` milestones (Phases 13–16); NF-06, NF-07, NF-08 and NF-11 stay
under `Not yet placed` for the reasons given with them.
