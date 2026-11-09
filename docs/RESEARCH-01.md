# RESEARCH — mathematics and algorithms behind `dedupe`

Every decision below is written in the form: what the source says, what the
mathematics gives us, what `dedupe` does with it. Items marked **[CLAUDE]** are
decisions the attached design does not specify and are mine; they are
reversible. Items marked **[SPEC]** come from the attached design.

Terms are defined on first use.

---

## 1. Exact duplicates — content hashing

### 1.1 The problem

Two files are exact duplicates when every byte is equal. Comparing every pair
byte-for-byte is O(n²) in file count and reads every file once per pair. A
content hash reduces each file to a short fingerprint so equality becomes a
hash-table lookup: O(n) reads, O(n) memory.

### 1.2 Which hash

**BLAKE3** (O'Connor, Aumasson, Neves, Wilcox-O'Hearn, 2020):

- 256-bit output; targets 128-bit security for collision resistance
  ([paper](https://cgi.cse.unsw.edu.au/~cs4601/refs/papers/blake3.pdf)).
- Input is split into 1 KiB chunks forming a binary Merkle tree, so chunks
  compress independently: SIMD (single instruction, multiple data — one CPU
  instruction processing several lanes at once) across chunks, and threads
  across subtrees. The paper reports single-threaded throughput of roughly
  4× BLAKE2b, 8× SHA-512 and 12× SHA-256 on Intel Cascade Lake, with
  near-linear scaling to 16 threads.
- Below ~128 KiB of input the paper notes threading overhead dominates; the
  Rust crate's `update_rayon` is therefore only used for large files
  **[CLAUDE]** (threshold 1 MiB, set in `exact.rs`).

Collision mathematics (why a 256-bit hash is enough to *act* on): the
birthday bound gives the probability that any two of n distinct files share a
hash as approximately n² / 2^(b+1) for a b-bit hash. For n = 10⁹ files and
b = 256 that is 10¹⁸ / 2^257 ≈ 4 × 10⁻⁶⁰. A non-cryptographic 64-bit hash
(xxHash64) at n = 10⁶ gives 10¹² / 2^65 ≈ 2.7 × 10⁻⁸ — still small, but an
adversarially crafted collision is cheap for xxHash and infeasible for BLAKE3.
Since `purge` deletes on the strength of a match, the cryptographic hash is
the correct tool; the I/O cost dominates either way.

### 1.3 Staged pipeline (minimise bytes read)

Reading every byte of every file is the cost floor for a full-hash approach.
The staged filter avoids most of it:

1. **Size.** Files are grouped by byte length (from directory metadata, zero
   reads). Any size held by one file is dropped. Files cannot be duplicates
   across sizes.
2. **Sample.** Within a size group, hash the first 64 KiB and the last
   64 KiB **[CLAUDE]** (one hash over both windows). Files whose sample differs
   are dropped. Media containers put headers at the front (JPEG SOI/APP1,
   PNG IHDR, MP4 `ftyp`/`moov` at front or back), so sampling both ends
   separates most non-duplicates of equal size without reading the body.
3. **Full hash.** Remaining candidates get a full BLAKE3. Equal hash ⇒
   duplicate group.
4. **Verify** (only for `purge`): byte-for-byte compare of each duplicate
   against its group's keeper before the Recycle Bin call **[CLAUDE]**. This
   removes hash collision from the failure surface entirely for the one
   command that removes data. It costs one more read of each duplicate.

Expected bytes read: let S = total bytes, D = bytes in files that share a
size, F = bytes in files that also share a sample. Stage 2 reads
≤ 128 KiB × |size-sharing files|; stage 3 reads F. For a media library where
most same-size pairs are genuine copies, F ≈ D ≈ reclaimable bytes × 2, which
is far below S.

`--quick-search` **[SPEC]** stops after stage 2 and reports sample matches as
*likely* duplicates **[CLAUDE: definition of "quick"]**. It is reported with
a distinct label and never used by `milk` or `purge` unless the user passes
the flag explicitly; the output then says plainly that the match is
sample-level.

### 1.4 Reading strategy

Buffered 1 MiB reads through `std::fs::File` feeding `blake3::Hasher`.
Memory mapping was considered **[CLAUDE]** and rejected for Phase 1: on
Windows, mapped files hold a share lock that blocks deletion/moves of the
file while mapped, and the gain over buffered reads for sequential hashing
is small. Hashing is parallel across files with rayon (work-stealing thread
pool), bounded by the number of logical CPUs.

### 1.5 Hard links and junctions

Two directory entries pointing at the same on-disk file (hard links) occupy
space once; "deduplicating" them reclaims nothing and `milk` would move the
same inode twice. The `same-file` crate identifies these by file index on
Windows and inode on Unix; such entries are collapsed to one candidate before
grouping **[CLAUDE]**. Symbolic links and junctions are never followed.

---

## 2. Near-duplicate images — perceptual hashing

A perceptual hash maps an image to a short bit string such that visually
similar images map to strings at small Hamming distance (number of differing
bit positions). Exact hashing cannot do this: a re-saved JPEG differs in
almost every byte.

### 2.1 Candidates and evidence

| Hash | Bits | Construction | Source |
|---|---|---|---|
| aHash (average) | 64 | 8×8 grayscale, bit = pixel > mean | Krawetz 2011 |
| dHash (difference) | 64 | 9×8 grayscale, bit = p[x] > p[x+1] (row gradient sign) | Krawetz 2013 |
| pHash (DCT) | 64 | 32×32 grayscale → 2-D DCT-II → 8×8 low-frequency block → bit = coeff > median | Zauner 2010 |
| PDQ | 256 | luminance → 64×64 via tent-filter downsample → 16×16 DCT → bit = coeff > median; plus a quality score | Meta 2019 |

DCT = discrete cosine transform, the same frequency decomposition JPEG uses;
low-frequency coefficients describe coarse structure and survive resizing
and recompression.

Benchmark evidence used for the decision:

- **Needle In A Haystack, Fast** (Amit, Rapaport et al., 2022,
  [arXiv 2206.00282](https://arxiv.org/pdf/2206.00282)): 100 images × 58
  perturbations, plus 43,660 Reddit meme images. dHash "performed
  surprisingly well across the board", degraded least as the database grew
  from 250 to 25,000 images, and was the fastest method with pHash a close
  second. At thresholds tuned to ~0.5 % false-positive rate on BSDS500, the
  normalised Hamming thresholds were: aHash 0.052, pHash 0.224, dHash 0.159,
  wHash 0.072. On the meme set those thresholds produced false-positive
  rates of 6.4 % (aHash), 0.90 % (pHash), **0.05 % (dHash)**, 9.3 % (wHash).
  All block hashes fail under strong rotation, shearing and cropping; only
  learned descriptors keep up there.
- **Zauner 2010** ([thesis](https://phash.org/docs/pubs/thesis_zauner.pdf))
  specifies the DCT hash exactly as implemented in pHash (grayscale → 7×7
  mean filter → 32×32 → DCT-II by matrix product → 8×8 low-frequency block
  with the DC coefficient omitted → median threshold) and reports that none
  of the tested hashes survive horizontal flipping.
- **PDQ** ([Meta test-drive paper, arXiv 1912.07745](https://ar5iv.labs.arxiv.org/html/1912.07745)):
  256-bit; random pairs average Hamming distance 128; a distance ≤ 30 is
  reported as confident matching; 99.96 % of format-changed images fell
  within 30; rotation beyond ~5° and heavy cropping break it. PDQ includes a
  **quality** score (proportional to feature count) that flags flat images
  whose hashes are unreliable.

### 2.2 Decision **[CLAUDE]** — implemented in Phase 2

`perceptual.rs` implements **dHash and pHash in-house** on one shared
luminance grid:

- Decode once (EXIF orientation applied), Rec. 601 luminance, integer box
  prefilter for large images, Lanczos3 to a **288×288** grid. 288 is
  9·32 = 8·36 = 32·9, so the 9×8 dHash grid and the 32×32 pHash grid are
  exact block means of the same array — no second resampling, and every
  dihedral variant (identity, three rotations, two flips, two transposes)
  is a free index permutation of that array. Eight variants are hashed per
  image; matching compares an image's identity hash against every variant
  of the other image in both directions, so mirrored and rotated copies
  are found with one decode.
- pHash follows the `imagehash` convention (8×8 low band including the DC
  term, median threshold with the even-count mean), because that is the
  convention the benchmark thresholds were measured with.
- A **quality** gate (share of 9×8 gradients above 2/255) drops flat
  images from matching, as PDQ's quality score does; `QUALITY_MIN = 10`.
- A pair matches when **both** dHash ≤ 10 and pHash ≤ 14 on the same
  variant. Conjunction: the two hashes fail on different perturbations.

**Measured on the generated corpus** (`tests/corpus.rs`: 150 procedural
photographs × 21 perturbations, 3,300 signatures, 480,525 negative pairs;
full table in docs/VERIFICATION.md):

- 0 false positives at d≤10 ∧ p≤14; the closest negative pair sat at
  d+p = 26 against a threshold sum of 24.
- 100 % recall for rescale (25 %–150 %), JPEG q30–q90, rotation by 90°,
  mirror, brightness ±20 %, gamma 1.5, blur, ±10 noise, 75 % aspect
  squeeze, 2° rotation.
- Partial: 5° rotation 77 %, 10 % border crop 70 %, watermark 77 % (pHash
  is what the bright patch breaks; dHash stays ≤ 2), letterbox 62 %.
- Fails, as the literature predicts for every block hash: 20 %–40 % crops.
- The sweep showed d≤12 ∧ p≤14 adds 1.1 points of recall at 0 false
  positives on this corpus; the shipped thresholds stay at the published
  values because the corpus negatives are procedural and photographs
  share more structure than procedural noise. Loosening is a measured,
  one-constant change if a real library shows misses.

### 2.2b PDQ — the confirmation tier for `purge` **[CLAUDE]** — Phase 2b

`pdq.rs` is a port of the reference algorithm
([pdqhashing.cpp, downscaling.cpp](https://github.com/facebook/ThreatExchange/tree/main/pdq/cpp)):
Rec. 601 luma → Jarosz filter (two passes of horizontal-then-vertical box
filters, window ⌈dim/128⌉ per axis, which composes to a tent) → decimate to
64×64 at cell centres → quality = Σ|⌊(u−v)·100/255⌋| / 90 over adjacent
pairs, capped at 100 → 16 rows of the 64-point DCT-II basis (rows 1–16, DC
skipped), B = D·A·Dᵀ → bit = value > median → 256 bits. The seven other
orientations come from the 16×16 band by sign flips and transposes
(reversing an axis negates the even-indexed basis rows), unit-tested
against pixel-domain transforms of the 64×64 buffer. The documentation's
recommended threshold is ≤ 31 and its quality cutoff ≤ 49
([README](https://github.com/facebook/ThreatExchange/blob/main/pdq/README.md)).

Two departures from the reference, both measured:

- The Jarosz filter runs on the integer-box-prefiltered luma that the grid
  path already produces (short side 576–1151 px), not the full image. A
  24-megapixel file would otherwise need two 96 MB float buffers per
  concurrent decode. Hashes are therefore not bit-identical to the
  reference tool's; `dedupe` only ever compares its own hashes. (The
  Python wrapper of the reference could not be installed in the build
  environment, so bit-level agreement was not measured either way.)
- Inputs with a short side under 384 px are Lanczos-upscaled by an integer
  factor first. Below 128 px the reference window is 1 — no filtering —
  and centre sampling lands up to half a cell off. On the corpus this took
  25 % thumbnails from a worst case of 32 bits (one miss at the threshold)
  to 14; 384 measured better than 256 on every row and no worse than 512
  on the rest.

**Measured on the same corpus** (full table in docs/VERIFICATION.md):

- 0 false positives at every threshold up to 64; the closest of 480,525
  negative pairs sat at 74 bits (mean 115.5, i.e. near the 128 expected
  for unrelated codes).
- 100 % recall for rescale 25–150 %, JPEG q30–q90, 90° rotation, mirror,
  brightness ±20 %, blur, ±10 noise, 75 % squeeze; 98.7 % for gamma 1.5.
- Where PDQ fails and the 64-bit pair does not: 10 % border crop 0 % (vs
  70 %), 2° rotation 22 % (vs 100 %), watermark 0.7 % (vs 76.7 %),
  letterbox 0 % (vs 62 %). The 16×16 band reaches four times the
  frequency of pHash's 8×8, and those edits shift mid-band phase.

**The rule.** `find` and `milk` group on dHash ≤ 10 ∧ pHash ≤ 14 (milk is
reversible by `--undo`). `purge` bins a near duplicate only when PDQ ≤ 31
also holds against the keeper; the others print as `kept~` with the
reason and the `milk` command that moves them instead. On this corpus both
rules have zero false positives, so the benefit of requiring PDQ before
deletion is not visible here; it rests on PDQ's published operation at
billion-image scale at this threshold, and the cost — crops, small
rotations, watermarks and letterboxes are not auto-deleted — is the
measured recall gap above, which is the set of edits where "same file" is
most arguable.

**Quality is shown, not enforced.** 93 of the 150 corpus sources fall
below PDQ's recommended quality 50 (the procedural textures are smoother
than photographs), yet their negative pairs never came within 74 bits and
their positives matched at the rates above. Gating `purge` on it would
have blocked 62 % of confirmations for no measured benefit. Flat images
are already excluded from the near pass by the gradient gate
(`QUALITY_MIN`).

### 2.6 The deep tier — Phase 4, optional **[CLAUDE]**

Learned descriptors are the frontier for crops, rotations and heavy edits.
**SSCD** (Pizzi et al., CVPR 2022,
[paper](https://openaccess.thecvf.com/content/CVPR2022/html/Pizzi_A_Self-Supervised_Descriptor_for_Image_Copy_Detection_CVPR_2022_paper.html)):
a ResNet-50 with GeM pooling trained with SimCLR-style contrastive
learning plus an entropy regulariser on DISC2021, producing a 512-d
L2-normalised embedding compared by dot product; reported to beat plain
SimCLR descriptors by 48 % absolute micro-AP on that benchmark.

`deep.rs` runs the `sscd_disc_mixup` checkpoint as ONNX through `tract`
(pure Rust, single binary, no runtime to install — the model file is the
only download; `docs/DEEP.md` has the install line and the provenance
chain back to Meta's checkpoint hash). Preprocessing is the official
`skew_320` transform. The tier is on when the model file exists and off
otherwise; no flag.

**Measured** (`measure_deep` in `tests/corpus.rs`: 40 procedural sources ×
21 perturbations, 880 embeddings, 33,540 negative pairs; table in
docs/VERIFICATION.md):

- Minimum / median cosine, with recall at the shipped threshold 0.70 where
  it is below 100 %: rescale and brightness ≥ 0.90; mirror 0.91; 10 % crop
  0.84; 20 % crop 0.71 / 0.79; 40 % crop 0.36 / 0.59 (2.5 %); 90° rotation
  0.62 / 0.71 (77.5 %); letterbox 0.66 / 0.77 (92.5 %); watermark 0.84;
  JPEG q30 0.65 / 0.75 (77.5 %); noise 0.67 / 0.81 (92.5 %).
- Negatives: median **0.425**, 99th percentile 0.582, maximum **0.669**.
  Procedural value-noise pictures are one texture family to this model;
  unrelated photographs will score lower, but that is a prediction, not a
  measurement here. 0.70 gives 0 false positives on this corpus with a
  0.03 margin; 0.65 lets 4 through, 0.60 lets 127 through.
- On the hand-made photo fixture a different procedural picture scored
  0.76 against the keeper — above the threshold. That is the measured
  reason the tier **lists and never acts**.

**How it enters the groups.** The deep tier adds no edges to the
union–find graph. After the 64-bit pair has built its components and
chosen keepers, a leftover image is attached to the keeper it scores
highest against when that cosine is ≥ 0.70, and leftover pairs form
star-shaped two-groups around the better keeper. One false deep score can
therefore pull one image into one group; it cannot weld two families
together through a chain, and the `deep 0.78` printed on a line is the
number that admitted it. Deep-only members print with the verb `deep`,
`milk` and `purge` leave them in place, and the totals say so.

**Keeper policy change.** A letterboxed derivative has more pixels than
its original and would have taken the keeper seat under "most pixels". The
near keeper is now the member with the most direct 64-bit matches inside
the group (the hub — derivatives resemble the original more than each
other), then most pixels, then oldest, then largest file.

Cost: ~0.3 s per image on two cores through tract; the binary grows from
2.9 MB to 25.6 MB with the tier compiled in (`--no-default-features`
builds without it). ONNX Runtime was not used because its prebuilt
binaries are not on crates.io and would have to be installed beside the
executable; whether it would be faster here was not measured.

### 2.3 Searching Hamming space — Multi-Index Hashing

With n hashes, checking all pairs costs n(n−1)/2 XOR+popcount operations.
At n = 10⁴ that is 5 × 10⁷ — sub-second. At n = 10⁵ it is 5 × 10⁹ — tens of
seconds. At n = 10⁶ it is 5 × 10¹¹ — not acceptable.

**Multi-Index Hashing** (Norouzi, Punjani, Fleet, CVPR 2012 / PAMI 2014,
[paper](https://ar5iv.arxiv.org/html/1307.2982)):

- Split each q-bit code into m disjoint substrings.
- **Proposition 1 (pigeonhole):** if two codes are within Hamming distance r,
  at least one substring pair is within ⌊r/m⌋. Proof: if every substring
  differed by more than ⌊r/m⌋ the total would exceed r.
- **Proposition 2 (reduced radii):** with r = m·r′ + a, search the first a+1
  substring tables at radius r′ and the rest at radius r′−1; all r-neighbours
  are still found.
- Query: look up each substring in its table, union the candidates, verify
  each candidate with a full Hamming distance.
- Reported "hundreds of times faster than linear scan" on up to 10⁹ codes.

Substring length: with n codes spread uniformly over an s-bit substring, the
expected bucket occupancy is n / 2^s; setting it to about 1 gives
s ≈ log₂ n, i.e. m ≈ q / log₂ n **[derivation, mine]**. For q = 64 and
n = 10⁵ (log₂ ≈ 16.6) that is m = 4 substrings of 16 bits. With r = 10 and
m = 4: r′ = 2, a = 2, so three tables are searched at radius 2 and one at
radius 1. Radius-2 lookups in a 16-bit table cost 1 + 16 + 120 = 137 bucket
probes; radius 1 costs 17. Per query ≈ 3 × 137 + 17 = 428 probes plus
verification of the candidates, versus 10⁵ full comparisons for linear scan.

Implemented in `hamming.rs` with m = 4 substrings of 16 bits over the
dHash; pHash is verified on the same variant for every candidate. Both
engines are tested to return identical pairs on random and on real
signatures. **Measured** (2 vCPUs, random codes with planted copies):
linear 0.15 s / MIH 0.12 s at n = 5,500; 2.4 s / 0.8 s at 22,000; 10.4 s /
2.3 s at 44,000. `LINEAR_MAX = 4,000` is the shipped crossover.

### 2.4 Grouping

Near-duplicate relations are not transitive (A≈B, B≈C does not force A≈C),
so groups are connected components of the match graph. Union–find with
path halving and union by rank (Tarjan 1975) gives near-constant amortised
time per union (`near.rs`). A member linked only transitively shows its
real distance to the keeper, in the warning colour when it exceeds a
threshold. Exact-duplicate groups are equivalence classes of one hash
value and need no graph; the near pass runs over exact-group keepers and
singletons only, so a byte copy is never reported twice.

Near keeper **[CLAUDE]**: most pixels (a downscaled copy is the
derivative), then oldest, then largest file (at equal age and size a
lossless encoding outweighs a lossy one), then path.

---

## 3. Near-duplicate video — Phase 3

### 3.1 Evidence

**TMK + PDQF** (Temporal Match Kernel on PDQ float features; Meta 2019; test
drive in [arXiv 1912.07745](https://ar5iv.labs.arxiv.org/html/1912.07745)):
videos are resampled to 15 fps; each frame yields PDQF (the 256 PDQ DCT
values before binarisation); features are pooled with trigonometric weights
over four periods into a fixed-size descriptor. Two-phase match: cosine of
the mean features (level 1) and a kernel maximised over time offsets (level
2), both suggested at 0.7 in the reference README. Robust to bitrate,
format and scale; trims perform well; cropping beyond ~20 % and letterboxing
are weak.

The kernel is from Poullot, Tsukatani, Nguyen, Jégou, Satoh, *Temporal
Matching Kernel with Explicit Feature Maps*, ACM Multimedia 2015
([dblp](https://dblp.org/rec/conf/mm/PoullotTNJS15)). The reference
constants ([tmkfv.cpp](https://github.com/facebook/ThreatExchange/blob/main/tmk/cpp/algo/tmkfv.cpp)):
periods {2731, 4391, 9767, 14653} frames; 32 Fourier coefficients
a_0 = (I_0(β) − e^−β)/(2 sinh β), a_j = I_j(β)/sinh β with β = 32, so that
Σ a_j cos(jδ) approximates a von Mises kernel in the offset δ; per frame the
L2-normalised feature is accumulated as cos(2πjt/T) and sin(2πjt/T)
projections; each projection vector is L2-normalised and scaled by √a_j;
level 2 = max over offsets of Σ_j a_j-weighted
[cos(jδ)(⟨c,c′⟩+⟨s,s′⟩) + sin(jδ)(⟨s,c′⟩−⟨c,s′⟩)], divided by
a_0 + 2Σa_j. The default hasher resamples to 15 fps with `pdqf` features
([tmk-hash-video.cpp](https://github.com/facebook/ThreatExchange/blob/main/tmk/cpp/bin/tmk-hash-video.cpp)).

### 3.2 Implementation (`video.rs`)

One `ffprobe` for metadata, then one `ffmpeg` process per file streaming
`fps=15,scale=288:288:flags=area,format=gray` as raw bytes through a pipe
— no temp files, frames hashed as they arrive, 83 KB per frame. The 288
grey frame is the same grid the image path uses, so dHash/pHash/PDQ run on
frames unchanged (PDQ's Jarosz window on 288 is ⌈288/128⌉ = 3). Every
frame's PDQ band feeds the TMK accumulator; the twelve frames nearest to
(i+0.5)/12 of the duration also get the full eight-orientation image
signature — the **positional tier**. ffmpeg is a subprocess, not a linked
library, because linking FFmpeg into a Rust build on Windows needs vcpkg
and a matching toolchain; `doctor` reports whether `ffmpeg`/`ffprobe` are
on PATH, and a run that wanted the pass without them says so and finishes
the exact tier. Concurrency: ⌈cores/2⌉ clamped to 1–4 ffmpeg processes with
`-threads 2` each **[CLAUDE]**.

Two tiers, because they fail differently:

- **Positional** — medians over the shared positions of (d, p, q) on the
  modal orientation. Invariant to re-encode, scale, mirror/rotation, speed
  change and frame-rate change (positions are fractions of duration); blind
  to trims.
- **TMK** — handles trims, head/tail cuts, title cards; blind to mirrors
  (PDQF sign flips → negative level 1) and letterboxes.

**Measured** (`tests/video_corpus.rs`: 16 synthetic sources × 12 edits,
208 clips, 60,304 frames, 3,000 negative pairs; table in
docs/VERIFICATION.md):

- The README's 0.7/0.7 gave **19 false groups**: two different zooms into
  the same fractal share a mean appearance (level 1 up to 0.817) and a
  temporal profile (level 2 up to 0.769). Level 1 is a weak discriminator;
  level 2 separated every negative (max 0.769) from every trim (min 0.785).
- Shipped **[CLAUDE]**: level 1 ≥ 0.5 as the gate before the offset search,
  **level 2 ≥ 0.8 to group** (0 false groups, 99.3 % of the TMK-carried
  edits), **level 2 ≥ 0.9 to bin** (0.13 margin over the closest negative;
  96.5 % recall — trims between 0.8 and 0.9 are grouped, printed `kept~`,
  left to `milk`). Positional confirms with the image rule plus q ≤ 31.
- Negatives on the positional tier came as close as d 1 with p 28 (two
  static bar patterns): the conjunction holds; q never below 118.
- The recovered time offset is approximate under the reference's
  per-coefficient normalisation (unit-tested on synthetic features) and is
  not displayed; trims are labelled from the ffprobe durations instead.

Cost: 2.2 ms per 15-fps frame end to end, of which ffmpeg's decode and
scale are ~88 %; the per-frame hashing is ~0.3 ms. A two-hour file is about
four minutes of one ffmpeg process.

Audio fingerprinting (Chromaprint) is out of scope.

## 4. Undo — the journal

`--undo` **[SPEC]** reverses the last change. A reversal needs a record of
what moved where; that record is the only state `dedupe` writes, and it is
written **only when a command changes the directory** (never on `find`,
never on `--dry-run`). It lives outside the scanned tree, in the OS's local
application data directory (`%LOCALAPPDATA%\dedupe\journal\` on Windows,
`~/.local/share/dedupe/journal/` on Linux) **[CLAUDE]**, so no file is ever
added to a user's directory except the `_milked` folder the spec defines.

- `milk` journals each move as (source, destination, size, hash).
  Undo = move back, in reverse order; a destination that is no longer there
  is reported, not fabricated.
- `purge` sends duplicates to the system Recycle Bin via the `trash` crate
  (5.3.0), whose `os_limited::list` / `restore_all` are supported on Windows
  and freedesktop Linux ([docs](https://docs.rs/trash)). Undo lists the bin,
  matches entries by original path and deletion time recorded in the
  journal, and restores them. A hard delete would make undo impossible;
  this is why `purge` does not hard-delete.
- One journal per action; undo consumes the latest unconsumed journal for
  that root. Journals are small JSON files and are never required for a
  scan.
- Written as the change runs, not after it (Phase 5): the first move
  creates the file, later moves rewrite it at most every 250 ms through a
  sibling `.tmp` and one rename, so a kill between writes leaves the last
  complete journal rather than a truncated one. Ctrl-C is handled: the
  loop stops after the file in hand and the final write happens; a second
  Ctrl-C ends the process. An interrupted undo rewrites the journal with
  the entries still to restore and leaves it pending.

---

## 5. Keeper selection

Within a group, one file stays and the rest are duplicates. Policy
**[CLAUDE]**: keep the file with the earliest modification time (the original
is older than its copies); tie → fewest path components (the copy in a
nested `backup/old/` folder is the duplicate); tie → shorter name; tie →
lexicographic path. The policy is printed in every result footer so it is
never hidden. The date on a `keep` line is the local calendar date of that
modification time (Phase 5), the same day Explorer shows; the comparison
itself uses the full timestamp.

---

## 6. Sources

- BLAKE3 paper — https://cgi.cse.unsw.edu.au/~cs4601/refs/papers/blake3.pdf
- Needle In A Haystack, Fast (perceptual hash benchmark) — https://arxiv.org/pdf/2206.00282
- Zauner, Implementation and Benchmarking of Perceptual Image Hash Functions — https://phash.org/docs/pubs/thesis_zauner.pdf
- PDQ & TMK+PDQF test drive — https://ar5iv.labs.arxiv.org/html/1912.07745
- Norouzi, Punjani, Fleet, Fast Exact Search in Hamming Space with Multi-Index Hashing — https://ar5iv.arxiv.org/html/1307.2982 (PAMI version: https://www.cs.toronto.edu/~fleet/research/Papers/MIH-pami2014.pdf)
- Pizzi et al., A Self-Supervised Descriptor for Image Copy Detection — https://openaccess.thecvf.com/content/CVPR2022/html/Pizzi_A_Self-Supervised_Descriptor_for_Image_Copy_Detection_CVPR_2022_paper.html
- TMK reference implementation — https://github.com/facebook/ThreatExchange/tree/main/tmk (cpp/algo/tmkfv.cpp, cpp/bin/tmk-hash-video.cpp, README.md); kernel paper — Poullot et al., ACM MM 2015, https://dblp.org/rec/conf/mm/PoullotTNJS15
- PDQ reference implementation — https://github.com/facebook/ThreatExchange/tree/main/pdq/cpp (hashing/pdqhashing.cpp, downscaling/downscaling.cpp) and README — https://github.com/facebook/ThreatExchange/blob/main/pdq/README.md
- SSCD ONNX export used — https://huggingface.co/nutszebra/sscd_onnx (sscd_disc_mixup.onnx, sha256 e65e2a5b…); Meta's checkpoint — https://dl.fbaipublicfiles.com/sscd-copy-detection/sscd_disc_mixup.torchscript.pt (sha256 9f26bd4c…); SSCD repository — https://github.com/facebookresearch/sscd-copy-detection
- `tract` — https://github.com/sonos/tract
- `trash` crate — https://docs.rs/trash
- `image_hasher` crate (evaluated, not adopted) — https://lib.rs/crates/image_hasher
