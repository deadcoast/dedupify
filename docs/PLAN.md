# PLAN — one thread, modulated phases

Language: **Rust**. Reasons, in the open: single static binary on Windows
with no runtime to install; BLAKE3 and rayon give multi-core hashing at I/O
speed; crossterm gives raw-mode menus and Windows virtual-terminal colour
without a widget framework. Python with `rich` was the alternative; it loses
on cold-start time for "fast and punchy" commands and on hashing throughput
for large trees. Effort was not a factor either way — both were built from
scratch here.

Every phase ends with the agent running the binary in its own runtime on a
generated fixture tree and recording what it observed. Nothing is marked
done on the strength of compiling.

## Phase 1 — exact duplicates, full command surface   ← delivered 2026-11-05

- Grammar and pre-parser (`-dr`, `.` as all, order-independent tokens)
- Walker with kind filters, hard-link collapse, `_milked` exclusion
- Size → sample → BLAKE3 pipeline, parallel hashing
- Keeper policy
- `find`, `milk`, `purge` (exact tier + byte verification + Recycle Bin),
  `--dry-run`, `--quick-search`, `--undo`, `doctor`, `--help`
- Journal (write on change only; undo for moves and bin restores)
- Renderer: theme, width fitting, in-place progress, result layouts
- Interactive menu: five workflows, four steps, confirm-with-command
- Tests: grammar, size/sample/hash staging, keeper ordering, journal
  round-trip, path shortening; end-to-end runs on a fixture tree

## Phase 2 — near-duplicate images   ← delivered 2026-11-05

- `perceptual.rs`: decode with EXIF orientation, 288×288 luminance grid,
  dHash and pHash from the mathematics in RESEARCH §2, eight dihedral
  variants per image, quality gate
- `hamming.rs`: parallel linear scan and Multi-Index Hashing, tested to
  agree; crossover measured (4,000)
- `near.rs`: union–find grouping over exact keepers and singletons;
  near keeper policy
- `img` token switches the pass on for `find`/`milk`; `purge` runs it
  always; exact-only runs print the `img` form as a `near` line
- `tests/corpus.rs`: 150 generated photographs × 21 perturbations; the
  shipped thresholds (dHash ≤10 ∧ pHash ≤14) measured at 0 false
  positives in 480,525 negative pairs — table in VERIFICATION.md
- Phase 2b — delivered 2026-11-07: `pdq.rs`, a port of the reference
  (Jarosz filter, 64×64, 16×16 DCT band, median, DCT-domain dihedral
  variants), measured on the same corpus; `purge` bins a near duplicate
  only when PDQ ≤ 31 also holds against the keeper, `kept~` otherwise;
  `q` column on every near line; quality shown, not enforced (reasoned in
  RESEARCH §2.2b)

## Phase 3 — near-duplicate video   ← delivered 2026-11-07

- `video.rs`: `ffprobe` metadata; one `ffmpeg` stream per file (15 fps,
  288 grey, raw pipe); positional tier (12 sampled frames, full image
  signatures, medians on the modal orientation); TMK descriptor ported
  from the reference constants; level-1 gate, level-2 group/bin rule;
  union–find grouping; keeper longest → most pixels → bitrate → oldest
- `vid` token switches it on for `find`/`milk`; `purge` runs it always;
  exact-only runs print the `vid` form as a `video` line; missing ffmpeg is
  reported once and the exact tier still runs
- `tests/video_corpus.rs`: 16 synthetic sources × 12 edits; the README's
  0.7/0.7 measured at 19 false groups, shipped 0.8 group / 0.9 bin at 0 —
  table in VERIFICATION.md

## Phase 4 — optional deep tier   ← delivered 2026-11-07

- `deep.rs`: SSCD `sscd_disc_mixup` through `tract` (pure Rust); on when
  the model file is installed (`docs/DEEP.md`), no flag; `doctor` reports
  it; cargo feature `deep` (default on; 25.6 MB vs 2.9 MB without)
- Lists, never acts: deep-only members print as `deep`, `milk`/`purge`
  leave them; threshold 0.70 measured at 0 false positives on the corpus
  with a 0.03 margin, and one procedural false positive on the fixture
- Grouping: star-shaped attachment to 64-bit keepers (no deep edges in the
  union–find); near keeper policy now degree-first
- `measure_deep` harness for promoting the tier on real photographs

## Phase 5 — polish   ← delivered 2026-11-08

No new tier. Every command, token and flag run on a combined fixture tree
(exact copies, near images, near videos, decoys, empty files, a stale
`_milked`, an empty folder, a folder with nothing duplicated) at 80 and 120
columns, as a terminal and piped, in the Unicode and ASCII glyph sets, with
and without the model — every screen kept and read line by line
(VERIFICATION.md, Phase 5). What that reading found is fixed here, plus the
items below that reading the code found.

- Interrupt safety: Ctrl-C during `milk`, `purge` or `--undo` stops after
  the file in hand and writes the journal for what was done; the journal is
  brought up to date on disk every 250 ms while a change runs (atomic
  replace), so a hard kill loses at most that window
- Windows paths: the `\\?\` prefix `canonicalize` adds is stripped from
  every displayed and journaled path; `_milked` is matched
  case-insensitively there
- Dates in `keep` lines are local calendar dates (what Explorer shows); the
  journal id stays UTC
- Grammar: `img` and `vid` together is an error, not last-one-wins
- Menu: `q` quits from any choice step; Esc still steps back
- `--undo` with nothing pending under PATH names other roots that have a
  pending journal
- `purge --quick-search` still verifies bytes before binning (it skipped
  that); `purge img` / `purge vid` run only the pass their scan can feed
- Output: NEAR/VIDEO headers count members beside the keeper, as the
  totals do; path columns sized from the real tails, compact VIDEO columns
  at narrow widths; stage lines drop segments rather than wrap; totals
  lines split rather than wrap; `next` lines in the typed form without
  `--quick-search`; action-line details in fixed-width columns; no empty
  stage lines after an empty or singleton scan; "no exact duplicates" when
  only near or video groups exist; plurals
- Help and doctor text cover the deep model; the colour fix line no longer
  names a shell; doctor shortens long paths instead of wrapping and reports
  unreadable journal files
- The model path no longer follows `DEDUPE_JOURNAL_DIR`; `NO_COLOR` keeps
  the Unicode glyphs
- Docs read against the binary line by line; VERIFICATION.md Phase 5 holds
  the audit record

## Phase 6 — real photographs, the fit, the moment   ← delivered 2026-11-08

The first measurements on real photographs (98 OpenCV sample pictures,
4,753 pairs; four real clips with re-encodes) and what they changed:

- `tests/real_corpus.rs`: the library as it is (which pairs the rule
  groups, which PDQ confirms) and derivatives of real sources; the
  chessboard photographed from several positions was grouped by the two
  hashes (d 10, p 12) and refused by PDQ — a false move `milk` would have
  made
- `geometry.rs`: the geometric fit (band-passed normalised
  cross-correlation searched over crops, offsets, small rotations,
  letterbox and stretch) as a second, spatial-domain family of evidence;
  measured on both corpora, shipped at 0.92
- Acting rule: `milk` and `purge` act only on two agreeing families — the
  hash pair with PDQ or the fit, or the deep descriptor with the fit; a
  hash-pair member confirmed by neither prints `kept~` in both commands;
  the deep tier gains an acting path (letterbox: deep 0.78 + fit 1.00)
- `capture.rs`: EXIF capture moment; two files with different moments are
  `kin` — shown, never acted on; the near keeper is the member whose
  moment most of the group shares
- `docs/COMPARISON.md`: what here is the standard pipeline and what is
  not, against the tools one would otherwise fork

## Phase 7 — conformance, and the second real set   ← delivered 2026-11-08

Checking the two ported algorithms against Meta's recorded outputs, and
what the second labelled real set (Meta's fifteen TMK sample videos)
changed:

- `examples/pdq_ref.rs`: PDQ against the reference regression hashes —
  bit-identical (0 of 256 on all eight images, both JPEG decoders) once
  the reference tool's 512-px nearest-neighbour input is reproduced
  (`pdq::compute_tool`); the eight dihedral hashes at 0 bits
- `perceptual::PDQ_PREP`: the tool's input measured against the shipped
  prefiltered input on both corpora; the prefiltered input kept (half-size
  copies 98.3 % against 91.7 % on real photographs, rotations and mirrors
  100 % against 93–95 %); the tool's input kept behind `PdqPrep::Tool`
  for the check
- `examples/tmk_ref.rs`: TMK against the reference `.tmk` files — equal
  element for element to 6.1·10⁻⁵ on the reference's frames; level 2
  0.982–0.9999 on ours; level-2 pair scores within 0.013 of the reference's own
- Black bars (`video::detect_bars`): one keyframe-only `cropdetect` pass
  before the frame pass; frames hashed inside the bars; `bars` on the
  line. The reference's own hashes score the sample set's pillarbox pair
  at 0.738; with the bars found it scores 1.00 and the clip's nine
  exports are one group. The video keeper counts the picture's pixels,
  not the file's, and at a tie keeps the file without bars
- Windows execution on the author's machine (`runs/win/`): build, 79 +
  23 tests, the suite with the model, the real Recycle Bin round trip,
  and every command on a sample tree with the same numbers as Linux

## Phase 8 — the result is the workspace   ← delivered 2026-11-09

What a person got before this phase: a listing whose primary content was
`d 0 p 0 q 4`, then two text lines telling them to retype a command with a
full path; `milk .` on a folder where `find . img` had just listed fifteen
near duplicates answered "no duplicates". The brief's rule is the
operational experience; this phase builds it.

- **One rule for depth** (`actions::near_enabled`/`video_enabled`):
  `find`, `milk`, `purge` run the full suite; `img`/`vid` scope the scan
  and nothing else; `--quick-search` is the byte-copy tier and names the
  full run in one `full` line. The `near`/`video` hint lines are gone.
- **Words first** (`near::Relation::words`, `video::VideoRelation::
  words`): each member line says what the member is — `same picture`,
  `re-encoded`, `near copy`, `resized 50%`, `cropped 80%`, `letterboxed`,
  `rotated 90°`, `mirrored`, `edited`, `looks alike`, `another shot,
  05:06:08`, `similar, unconfirmed`; videos `scaled 768→480`, `trimmed
  0:30 of 1:20`, `pillarboxed`, `bars removed`. The evidence follows,
  dim, and is the first column dropped when the width is short; the path
  keeps 32 cells before it goes. Action lines use the same words.
- **The workspace** (`workspace.rs`): after `find` on a terminal the bar
  `m milk N → _milked  p purge N → Recycle Bin  n preview  k keeper  u
  undo  q quit`; every action works on the groups already on screen
  (`actions::act_milk`, `act_purge`, `act_undo`, split out of the
  commands); `k` re-elects a keeper and recomputes the group's relations
  (`NearGroup::set_keeper`, `VideoGroup::set_keeper`); `q` before a change
  prints the typed forms. Piped output is unchanged.
- Tests: `words_for` unit tests; e2e `one_rule_for_depth_quick_is_the_
  exception`, `video_pass_runs_for_plain_find_and_purge_alike`, and the
  pseudo-terminal `find_on_a_terminal_acts_from_the_result` (Unix, through
  `script(1)`).
- Not changed: the matching engine, the thresholds, the journal.

## Phase 9 — the workspace, deep   ← delivered 2026-11-09 (from IDENTIFIED 2026-11-09)

The result workspace becomes where all decisions happen: per-group and
per-member selection, several changes per sitting, a look at the file
before deciding, long results navigable; the doc set made coherent.
Items: SHELVED.md § Phase 9. What it became:

- **SC-01, SC-04** one doc set (`RESEARCH.md`, `VERIFICATION.md` restored,
  the byte-identical copies dropped, `docs/README.md` the index); the
  menu's Find caption current.
- **TECH-03** `workspace/state.rs`: the cursor, scroll, selection, modes
  (browse, go-to, confirm) and the effects a key asks for, with the screen
  composed from captured listing lines — 8 unit tests, no terminal.
  `workspace/mod.rs` is the shell: alternate screen, raw keys, effects.
- **QOL-01** a mark on every line (`■ □ ◧ ·`); space per file or per
  group, `a` all/none; unconfirmed members choosable by hand, counted and
  labelled `by hand` through to the action's footer.
- **QOL-02** changes stack: each action keeps the groups from before it;
  `u` reverses the newest by its journal id and restores its groups; the
  bar under an action offers back, undo, quit.
- **QOL-04** arrows, PgUp/PgDn, Home/End, `g` + number; a header comes
  into view with its first member.
- **NF-04** `o` reveals in Explorer (`/select,`), Finder, or the folder.
- **NF-03** `v` draws keeper and member side by side — sixel (own encoder,
  216-colour cube, ordered dither), kitty, iTerm2 — chosen from the
  environment, `DEDUPE_GRAPHICS` overriding; the system viewer otherwise.
- **QOL-03** `recent.json` beside the journals; the menu's `again` line.
- **QOL-05** `--undo` with several pending changes on a terminal: a
  numbered list, newest first, local times.
- Act layer: `act_milk`/`act_purge` take an item list (`default_items`
  for the commands, the selection for the workspace), `act_undo_which`
  takes a journal id, `Found::prune_missing` renumbers `+GROUP` links.
- Found while verifying, fixed: a VIDEO header counted unconfirmed
  members as reclaimable (`10.2 MB` on the sample where `milk` moves
  8.62 MB; now confirmed members only, as NEAR groups always were); the
  `--undo` journal line ran past the width under a long root (its command
  now shortens in the middle).

## Phase 10 — formats and filesystems   ← delivered 2026-11-10 (Linux-verified; QOL-10, TECH-07 and SC-08 built, their Windows run skipped by the owner and carried to Phase 12)

Phone and camera files decode (HEIC/HEIF/AVIF via ffmpeg, RAW previews);
`milk` survives volume boundaries; Windows-only paths (Ctrl-C, colour)
verified; doctor covers the new failure modes. Items: SHELVED.md § Phase 10.
What it became:

- **TECH-01** `decode.rs` owns every still decoder and picks the route by
  the file's bytes: the `image` crate; HEIF/HEIC/AVIF; RAW previews;
  ffmpeg for the rest (JPEG XL, PSD, a DNG without a preview). Every
  image extension the scan accepts is decoded; the DECODE line names the
  routes and the reasons for what it could not decode.
- **NF-01** `heif.rs` reads the container (grid tiles, `iden`, `clap`,
  `irot`, `imir` in association order, the Exif item) and ffmpeg decodes
  the tiles as an elementary stream, so a phone's tiled photo decodes on
  any ffmpeg with HEVC (and libdav1d or libaom for AVIF), not only on
  8.1+. `imir`'s meaning and the colour authority were measured
  (RESEARCH §2.9): the stream's own colour signalling decides.
- **NF-02** `raw.rs`: the embedded preview by a marker-chain-validated
  scan, the mid-size preview when one has the full shape, orientation and
  moment from the RAW's metadata (TIFF, Olympus/Panasonic magic, CR3
  boxes, the preview's EXIF). A RAW keeps its group, is never acted on by
  the near tier (`raw`), and its camera JPEG twin is a `pair`, left with
  it unless chosen by hand (RESEARCH §2.10).
- **CORE-05** `fsops.rs`: a move across volumes is copy, fsync, read
  back, BLAKE3 compare, times kept, then delete (the copy removed if the
  original will not go); journaled as `copied`; undo crosses back the
  same way.
- **SC-07** typed failures (`ActError`: vanished, in use, permission,
  cross-device, bin, other) on the `failed` lines and to the workspace,
  whose bar offers `r retry` (in use, refused) and, after a purge,
  `m milk the N the bin refused`, each an undoable change of its own.
- **QOL-06** `--skip NAME` (`*`, `?`), remembered in the app folder,
  `--unskip NAME|all`; named on the header, counted on the SCAN line.
- **QOL-08** `--min SIZE`, per run, carried by the `next` lines and
  `again`.
- **QOL-09** doctor: `_milked` and the root's folders against the root's
  volume; HEIC and AVIF decoded from two built-in probe pictures; the
  model's SHA-256 against the published file; the skip list.
- **QOL-10, TECH-07, SC-08** the launcher writes UTF-8 logs (read in
  place, no longer committed), runs a Windows Ctrl-C test
  (`GenerateConsoleCtrlEvent`, CTRL_BREAK to a child in its own process
  group; with a console present it fails rather than skip) and the
  real-format tests against the machine's ffmpeg, and photographs its own
  console after a live doctor. When its own pull changes the script, the
  new copy runs the rest of the run (the first Windows run of this phase
  ran the old steps on the new code).
- **One date column per listing** **[CLAUDE]**, found in the first
  Windows run's workspace: the keeper's date and each member's words start
  at one cell in the GROUP, NEAR and VIDEO blocks alike; each block's
  path column takes what its own columns leave, and the listing gives up
  the least it can when the width is short (CLI §3).

## Phase 11 — matching depth   ← delivered 2026-11-10

The carried measurement debts and the engine's known misses, measured on
real data — INRIA Copydays, Google's HDR+ bursts, 409 real cameras'
metadata, the owner's sample tree (VERIFICATION § Phase 11):

- **CORE-04** the deep threshold on real labelled copies: unrelated
  photographs top out at cosine 0.526 in 5.44 million pairs; `DEEP_MIN`
  stays 0.70 (the descriptor alone lists); a band down to
  `DEEP_FIT_MIN = 0.60` groups a pair only when the fit confirms it, and
  then it acts (crops to 40 % of the area 46.5 → 91.7 %); the band waits
  until every pair at 0.70 is made.
- **CORE-01, CORE-02** the fit also scores its cells (4 × 4, the worst
  quarter set aside) and searches the mirror image: a caption bar 20 →
  98 %, a mirrored crop 3 → 97 %; `FIT_MIN` held at 0.92.
- **CORE-08** fractions of a second agree at any precision (`117280`
  is `117`); a burst's frames — one moment, names a camera numbered apart
  — are `another shot, same second` (`kin`); dates with dots, dashes or
  slashes are read.
- **CORE-03** bars of any flat colour: when the black pass finds none, a
  second reads the keyframes' detail along each edge band; white, grey
  and dark-blue pillarboxes found in 75 of 75 real copies, a blurred
  copy of the picture in 7 of 25, no bars in any of 79 clips without.
- **CORE-06** a third 64 KiB sample at the middle of a file over 1 GiB.
- **CORE-07** the keeper is the file not named or filed as a copy, then
  the oldest; near and video groups put the same marks after their own
  criteria (RESEARCH §5).
- **NF-05** `c` in the workspace chooses a group's other shots, by hand,
  to cull a burst; the "keep the sharpest" option was measured against
  HDR+'s own choice of frame (40 % agreement, the first frame 62 %) and
  not shipped.
- **TECH-02** a JPEG decoded reduced inside its DCT, measured on real
  phone and camera JPEGs and not shipped: 1.3× faster at 12 MP and no
  faster under 9, and a file's own PDQ moved up to 22 bits, with no
  margin gained against its copies; the route stays in its harness.
- **QOL-07** DECODE and FRAMES show the time left once they have run 5 s.
- Found while verifying: footer notes break at the width instead of
  running past it; the `~` legend no longer calls a by-hand row
  confirmed; the near policy label fits 80 columns.

## Phase 12 — structure and CI   ← in progress (TODO.md)

The code shaped for what it became: actions/render/video split, a shared
group interface, the `unwrap()` audit, in-tree benchmarks, pseudo-terminal
tests on Windows, CI on Linux and Windows, and the Windows verification
Phase 10 carried (Ctrl-C, UTF-8 logs, console colour). Items: SHELVED.md
§ Phase 12.

So far (VERIFICATION § Phase 12):

- **SC-02, SC-03, TECH-06** `actions`, `render` and `video` are folders of
  modules by concern — scan / plan / act / undo, the stage lines / the
  listing / the totals / the hints, probe / TMK / bars / grouping — with
  every line of the old files accounted for.
- **SC-05** a `GroupView` over the exact, near and video groups; the
  workspace no longer counts offsets across three lists.
- **SC-06** no `unwrap()` outside tests (18 before), and clippy denies a
  new one.
- **TECH-05** `tests/bench.rs` times each stage on a fixed generated
  corpus.
- **TECH-08** CI on Ubuntu and Windows, with ffmpeg: fmt, clippy with
  and without the deep tier, the suite both ways, the doctor.
- **TECH-04** the workspace driven by keys through a Windows pseudo
  console, green in CI (the first run found the child drawing into the
  test's own redirected output).
- **TECH-07** Ctrl-C on Windows verified in CI by its own words; **SC-08**
  by the 2026-11-11 Windows run.
- **QOL-10** colour's enablement path tested through the console host
  (`a_terminal_gets_colour`); its picture waits for the launcher's next
  run on the owner's machine — the last box of the phase.

## Docs chain

```
[IDENTIFIED + PLAN] -> [SHELVED] -> [TODO]
```

`IDENTIFIED.md` is the review; this plan holds the milestones;
`SHELVED.md` orders the review's items under the milestones; `TODO.md` is
the milestone in progress. When TODO is done, the next milestone comes
from SHELVED; when every milestone here is delivered, a new IDENTIFIED
pass runs.

## Standing rules

- No cache, index or lock file is ever written on a scan.
- A change writes exactly one journal; `--undo` consumes it.
- Thresholds ship only after they are measured on a corpus in the phase
  that introduces them.
- A tier acts (moves or bins) only when two independent families of
  evidence agree: the two 64-bit hashes with PDQ or the geometric fit, or
  the learned descriptor with the fit. A single family lists. Two files
  that state different capture moments are never acted on, and neither is
  a camera RAW by the near tier (Phase 10). In the workspace a person may
  choose a listed file by hand — their look is the second family for that
  file, and every line and footer says so. A RAW cannot be chosen;
  another shot (another capture moment, or another frame of its second)
  only all together through `c`, a person culling a burst (Phase 11).
- A move never leaves two files or none: across volumes it is a verified
  copy whose original is deleted last (Phase 10).
- New flags need a reason stronger than convenience; the menu stays at five
  workflows (plus `again`, the last folder) per screen.
