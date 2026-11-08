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

## Standing rules

- No cache, index or lock file is ever written on a scan.
- A change writes exactly one journal; `--undo` consumes it.
- Thresholds ship only after they are measured on a corpus in the phase
  that introduces them.
- A tier acts (moves or bins) only on evidence from at least two
  independent hash families; a single learned score lists.
- New flags need a reason stronger than convenience; the menu stays at five
  choices per screen.
