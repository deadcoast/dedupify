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

## Standing rules

- No cache, index or lock file is ever written on a scan.
- A change writes exactly one journal; `--undo` consumes it.
- Thresholds ship only after they are measured on a corpus in the phase
  that introduces them.
- A tier acts (moves or bins) only when two independent families of
  evidence agree: the two 64-bit hashes with PDQ or the geometric fit, or
  the learned descriptor with the fit. A single family lists. Two files
  that state different capture moments are never acted on.
- New flags need a reason stronger than convenience; the menu stays at five
  choices per screen.
