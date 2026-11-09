# COMPARISON — what here is the standard pipeline, and what is not

The question this answers: why build this rather than fork a proven
duplicate finder. The honest answer has three parts, and the first is
that for exact duplicates there is nothing to improve on.

Facts about other tools below were read from their sources on
2026-11-08 (czkawka `qarmin/czkawka` at master: `instructions/`,
`czkawka_core/src/tools/`; the `image_hasher` and `similario_core`
crates it uses). Where this file says "as far as its documentation shows"
it means exactly that.

## 1. Exact duplicates — the same pipeline, on purpose

Size → sample → full hash → byte verification is how every serious tool
works, and `dedupe` does it the same way: czkawka's Duplicate Files uses
size, a prehash of the first and last 4 KB, then a full hash (BLAKE3 by
default, xxh3 or CRC32 selectable); `dedupe` uses size, the first and
last 64 KB, then full BLAKE3, and byte-compares each file against its
keeper before the bin call. Nothing in this stage is novel and nothing
should be. The two differences are policy, not algorithm:

- czkawka caches hashes to disk between runs (and recommends it);
  `dedupe` writes nothing on a scan — a design constraint of the
  specification, paid for in re-hashing time on the next run.
- `dedupe` byte-verifies before every bin, including under
  `--quick-search`. czkawka's documentation describes deleting or
  trashing on the hash match; a byte comparison step is not mentioned.

If exact duplicates were the whole job, forking fclones or czkawka
would have been the right call.

## 2. Near-duplicate images — the same hashes, a different rule

**What is the same.** The hashes are the published ones: dHash and pHash
as `imagehash` defines them, PDQ as Meta's reference C++ defines it. The
`image_hasher` crate czkawka uses offers the same family (its `Gradient`
is dHash, `DoubleGradient`, `Mean`, `Median`, `Blockhash`); czkawka's
geometric-invariance option hashes the mirrored and 90°-rotated variants
too, as `dedupe` does for all eight dihedral variants.

**What is different is how a decision is made.**

| | czkawka Similar Images (as its docs and source show) | `dedupe` |
|---|---|---|
| evidence to group | one perceptual hash at a chosen size, Hamming distance under a preset (`SIMILAR_VALUES`, e.g. ≤ 15 of 256 bits at 16×16 "Medium") | two 64-bit hashes of different construction (gradient and DCT) must both be within threshold on the same orientation |
| evidence to delete or move | the same single distance | a second family must agree: PDQ ≤ 31 (frequency domain, 256 bits) **or** the geometric fit ≥ 0.92 (spatial correlation); the learned descriptor admits but never acts alone |
| crops, letterboxes, small rotations | not matched by block hashes (nor by `dedupe`'s) | found by the fit when the hashes or the descriptor bring the pair in; the fit also tells which is the crop and by how much |
| burst frames, stereo pairs, the same scene from another position | grouped when the hash distance is small | the hashes may group them; PDQ and the fit refuse them (measured: chessboard poses fit 0.31–0.45); different EXIF capture moments make them `kin`, never acted on |
| thresholds | user-chosen presets | fixed, each measured on a corpus before shipping, with the measurement in `docs/VERIFICATION.md` |
| reversal | trash (optional) | one journal per change; `--undo` reverses moves and Recycle Bin entries; Ctrl-C mid-change still leaves a complete journal |

The novelty claimed here is exactly this much: **the acting rule** (two
independent families, with the geometric fit as the second family where
the frequency-domain hashes are blind) and **the capture-moment rule**
(a photograph is defined by its exposure, not its pixels). Neither is
an algorithm from a paper; both are design decisions with measurements
behind them (Phases 2, 2b, 6), and both would be easy to add to any
tool. They are not in the tools surveyed.

## 3. Near-duplicate video — a different descriptor

czkawka's Similar Videos (through `similario_core`) groups by duration
within a tolerance, hashes frames over temporal windows, and calls two
videos similar when enough windows match; it detects subclips and can
add an audio fingerprint, and can strip letterbox bars before hashing.

`dedupe` uses a port of TMK (Meta's temporal-matching kernel): the frame
stream is projected onto a small set of periodic kernels, so a trim or a
re-encode compares as a whole signal rather than window by window, with a
cheap level-1 gate and an alignment-searching level-2 score (a 1.1× speed
change measured at tmk 0.975 minimum on the synthetic corpus, Phase 3). Both approaches are published; TMK handles trims without
a duration prefilter and gives one score per pair. czkawka has audio
fingerprinting and letterbox cropping, which `dedupe` does not; `dedupe`
has the positional frame tier (three image hashes on twelve sampled
frames, medians on the modal orientation) as the second family for
mirrored or re-framed videos, which czkawka's docs do not describe.

Measured on real clips (Phase 6): the OpenCV `vtest.avi` and its
re-encode, downscale and 40-second trim grouped at tmk 1.00 / 1.00 /
0.84; `Megamind.avi`, its x264 re-encode and the shorter `_bugy` variant
grouped at 1.00; the unrelated `tree.avi` did not.

## 4. What the proven projects have that this does not

- Hash caches and incremental re-scans.
- A GUI, many more tools (broken files, empty folders, music tags, bad
  extensions), years of field use and bug reports.
- Audio fingerprints for videos; letterbox stripping before video
  hashing.
- Field-tested thresholds on real libraries. `dedupe`'s image thresholds
  were set on a procedural corpus and checked on 98 real photographs
  (Phase 6); that is a start, not field use.

## 5. What this has that the surveyed tools do not

- A decision that needs two families of evidence, with the second family
  chosen to cover what the first is blind to.
- The geometric fit as a confirmation step: crops, letterboxes and small
  rotations become actionable with their geometry stated on the line.
- The capture-moment rule.
- A reversible staging step (`milk` into `_milked`) separate from the
  bin, a journal for every change, undo across both, interrupt safety.
- The output as a designed record rather than a log: every line fitted,
  every number on a line the number that produced the verdict.

## 6. Where "it does nothing" would be a fair verdict today

- On a real camera roll the capture-moment rule is untested: the real
  set here carries no EXIF.
- The deep tier's threshold was set on procedural pictures; on real
  photographs it has been run (Phase 4 fixture, Phase 6 real set) but
  not measured against a labelled set of real copies.
- Windows execution is type-checked only.
- PDQ bit-identity with the reference tool and TMK agreement with the
  reference implementation were not checked.

Each of these is listed in `docs/VERIFICATION.md` under the phase that
left it open.
