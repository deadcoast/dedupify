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
   A file over 1 GiB also hashes the 64 KiB at its middle **[CLAUDE,
   Phase 11, CORE-06]**: disk images, archives and databases of one
   size share their header and trailer, so two that differ only inside
   agreed on both ends and were each read in full — or reported *likely*
   by `--quick-search`. The third window costs one 64 KiB read against a
   full read of at least a gigabyte; an edit between the windows is still
   left to the full hash (measured: VERIFICATION § Phase 11, CORE-06).
3. **Full hash.** Remaining candidates get a full BLAKE3. Equal hash ⇒
   duplicate group.
4. **Verify** (only for `purge`): byte-for-byte compare of each duplicate
   against its group's keeper before the Recycle Bin call **[CLAUDE]**. This
   removes hash collision from the failure surface entirely for the one
   command that removes data. It costs one more read of each duplicate.

Expected bytes read: let S = total bytes, D = bytes in files that share a
size, F = bytes in files that also share a sample. Stage 2 reads
≤ 128 KiB × |size-sharing files| (192 KiB for each over 1 GiB); stage 3
reads F. For a media library where
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

**Conformance (Phase 7).** The port is bit-identical to the reference.
`examples/pdq_ref.rs` hashes the eight regression images Meta ships
(`pdq/data/reg-test-input/dih`) and compares with the hashes the
reference `pdq-photo-hasher` recorded in `cpp/reg_test/expected/out`.
Two things had to be matched first: the text layout — the reference
prints sixteen 16-bit words, last DCT row first, column j at bit j — and
the reference *tool's* input, which is not the full image: before luma
it squashes any picture with a side over 512 to 512×512 by nearest
neighbour (CImg `resize(512, 512, 1, 3, 1)`, source index ⌊x·W/512⌋).
Given that input, all eight hashes agree in all 256 bits, with both JPEG
decoders tried (`zune-jpeg` and libjpeg via PIL), and our seven DCT-domain
orientation variants land on the reference's eight dihedral hashes at
0 bits. (The reference C++ could not be built in the environment — its
`make` was refused — so the comparison is against the recorded outputs,
which is what the reference's own regression test compares against.)

Two departures from the reference *tool* in what the algorithm is fed,
both measured:

- The hash runs on the integer-box-prefiltered luma that the grid path
  already produces (short side 576–1151 px), resampled by a tent filter
  to one working size, 576 × 576, for every file (Phase 15) — not on the
  tool's 512 nearest-neighbour squash and not on the full image (a
  24-megapixel file would need two 96 MB float buffers per concurrent
  decode). Until Phase 15 the plane went in at its own size, so a photo
  and its half-size copy were filtered with different Jarosz windows and
  decimated on different grids: on 215 real JPEGs, copies at 70–25 % came
  up to 46–56 bits from their originals (95th percentile 10–14) and
  mirrors up to 62. At 576 they come within 18–36 (95th percentile 4–6),
  mirrors within 6, and different pictures no closer than 84
  (VERIFICATION § Phase 15). 576 and not 512: at 576 the Jarosz window
  (5) and the decimation cell (9) are odd, so the box is centred and each
  cell's middle pixel is sampled, and a mirrored picture is filtered as
  the mirror of the original; at 512 (window 4, sample at 8i + 4) mirrors
  moved 16 bits at the median. Nearest-
  neighbour decimation aliases, and it shows: on real photographs the
  tool's input keeps 91.7 % of half-size copies, 88.3 % of quarter-size,
  95 % of 90° rotations and 93.3 % of mirrors within 31 bits, where the
  prefiltered input keeps 98.3 %, 98.3 %, 100 % and 100 % (the one miss
  is a synthetic ramp); the procedural corpus agrees in direction
  (VERIFICATION.md, Phase 7). Ours land a median 10 bits from the tool's
  own hash of the same picture (the 215 JPEGs; up to 88 on a picture the
  tool's squash aliases) — `dedupe` compares its hashes with its own, and
  a hash the reference tool published is not expected to match. `perceptual::
  PDQ_PREP` selects the preparation; `PdqPrep::Tool` reproduces the tool
  exactly and is what the conformance check and the measurement harness
  use.
- Until Phase 15, inputs with a short side under 384 px were
  Lanczos-upscaled by an integer factor first (the 576 working size now
  brings every input up or down to one size). Below 128 px the reference window is 1 — no filtering —
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

**A reduced JPEG decode — Phase 11 (TECH-02), measured, not shipped.**
JPEG can decode at 1/2, 1/4 or 1/8 inside its inverse DCT, so a large
photo never has to exist at full size. Through `jpeg-decoder` (the
largest reduction that keeps the short side ≥ 576) it was no faster on
pictures of 9 megapixels and under, 1.3× faster on 12-megapixel phone
photos, and held 0.7 MP a file instead of 11.9; but a file's own PDQ
moved up to 22 bits from the full decode's, and against copies a
library holds it gained no margin, only moved it — a half-size copy came
closer, a full-size PNG export of the same JPEG went from 0 bits to 22
(VERIFICATION § Phase 11). The working size is what both routes must
agree on before the memory saving is worth taking. The same measurement
found the shipped route's own soft spot: a 12 MP photo and its 70 %,
half or quarter copies were up to 22–24 PDQ bits apart (95th percentile
10), because the box prefilter's integer factor gave copies of different
sizes different working sizes — closed in Phase 15 by the 576 working
size (§ 2.2b).

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
other), then most pixels, then oldest, then largest file (Phase 11 put
the copy marks of §5 before "oldest").

In a group of two the hub ties, and "most pixels" handed the seat to the
letterboxed copy after all: on the owner's Windows run (2026-11-11) the
original `baboon.jpg` stood to be milked under its export
`exports\baboon_letterbox.png`. **[CLAUDE]** Pixels are now the
picture's own, as the video tier counts them: `near::picture_bars` finds
bars in one grey copy 256 wide — rows at top and bottom, or columns at
left and right, 97 % of whose pixels sit within 8 grey levels of the
outermost line's colour, both ends of one colour and about one depth,
the picture keeping a quarter of the side — and a file with bars counts
the picture inside them, less a fiftieth, so it outranks the bare
picture only with clearly more of it (an upscaled export still does).
The member says `letterboxed`, `pillarboxed`, `with a border` or, the
other way round, `bars removed`. Exact uniformity is the test because an
export pads with one colour; the video rule (low detail along the band,
a picture that starts at once) missed half the letterboxed photographs
whose sky meets the bar. Measured (`tests/image_bars.rs`) on 523 real
pictures and three boxed copies of each: letterboxed 4:5 in black found
in 514, pillarboxed 16:9 in white in 514, a grey border in 521, with one,
seven and one placed more than 2 % off; in the pictures themselves bars
were found in 9 — graphics with real flat margins (OpenCV's `cards`,
`chessboard`, text images) and one hand-made Copydays copy printed with
a paper margin — and in none of the 157 holiday photographs or the 39
phone photographs.

Cost: ~0.3 s per image on two cores through tract; the binary grows from
2.9 MB to 25.6 MB with the tier compiled in (`--no-default-features`
builds without it). ONNX Runtime was not used because its prebuilt
binaries are not on crates.io and would have to be installed beside the
executable; whether it would be faster here was not measured.

**Real labelled copies — Phase 11 (CORE-04).** The procedural corpus
was the floor, as said; Phase 11 measured on real photographs
(`tests/real_copies.rs`, 3,310 embeddings): INRIA Copydays (Jégou,
Douze & Schmid, ECCV 2008 — 157 holiday photographs and 229 copies made
by hand: print and scan, paint, blur, perspective, collage), with its
JPEG series (quality 75 down to 3) and crop series (10 to 80 % of the
surface removed) regenerated from the originals at its own levels, and
the 98-picture real set of §2.7 as outside negatives.

- Unrelated photographs: 5,440,359 pairs, median cosine 0.007, 99th
  percentile 0.133, 99.9th 0.219, maximum **0.526** (two different
  photographs, both re-encoded at JPEG quality 5–15). None reaches 0.55.
- Copies at 0.70: JPEG at quality 10 and above 93–100 %; crops removing
  up to half the surface ≥ 92.4 %, 60 % removed 47.1 %, 70 % removed
  5.1 %; the hand-made copies 15.3 % (the hashes alone find 12 of the
  229, PDQ 2).
- Photographs of one subject from another position are the near miss:
  the real set's chessboard (13 poses from two cameras) reaches 0.88
  between poses, a stereo pair of one plant 0.91. Above 0.70 they were
  already listed and never acted on.

So `DEEP_MIN` stays 0.70 for listing; the measurement supports a band
under it instead, where the descriptor's word is not enough to list but
is enough to ask the fit. `DEEP_FIT_MIN = 0.60` **[CLAUDE, Phase 11]**: a
pair in [0.60, 0.70) enters a group only when the fit (§2.7, with the
Phase 11 search) confirms it at 0.92 — and then two families agree and it
acts; unconfirmed, it is dropped, not listed. On Copydays, the rule
"descriptor and fit" (plain fit, as measured) acts on 74.0 % of the
3,055 regenerated and hand-made copies at 0.70 and 82.1 % with the band:
40 %-of-area crops 46.5 → 91.7 %, 30 % 5.1 → 56.1 %, JPEG quality 3
43.3 → 65.0 %, hand-made 9.2 → 13.1 %. No unrelated pair is in the band;
on the real set, 81 pairs are, and the only two the fit confirms are true
derivatives (`lena_tmpl`, a local edit of `lena`, 1.000; the OpenCV logo
on another background, 0.946); the closest photograph of the same
subject, two chessboard poses, fits at 0.787. The band waits its turn:
a picture joins a keeper or a partner under 0.70 only after every pair
at 0.70 is made, best cosine first, so an attachment the fit may still
drop never takes a picture from a pair that lists (`tests/deep_band.rs`).

### 2.7 The geometric fit — Phase 6 **[CLAUDE]**

Where the hashes stop, correlation continues. Every hash above works in
the frequency domain on the whole frame, which is why crops, letterboxes
and small rotations defeat all three at once: the frame is a different
frame. The fit answers the question the hashes cannot — *does one picture
sit inside the other?* — directly, in the spatial domain, and that makes
it an independent family of evidence.

**Measure.** For a window `A_w` of picture A and a picture B resized to
the window's size, the normalised cross-correlation

```
ρ(A_w, B) = Σ (a − ā)(b − b̄) / √( Σ (a − ā)² · Σ (b − b̄)² )
```

over the window's pixels. ρ is invariant to any `b → αb + β` with α > 0,
so a crop that was also brightened, gamma-shifted or re-encoded still
scores as the same picture; ρ = 1 is an affine match, ρ ≈ 0 is unrelated.

**Band.** Both sides are band-passed first — a narrow box blur (radius 1
at 128 px) minus a wide one (radius 8). The wide term removes the mean and
every slow gradient (a box filter maps a linear ramp onto itself, so the
difference of two is zero on ramps), which stops a smooth sky from
correlating with a smooth ramp; the narrow term makes thin strokes
tolerant of the half-pixel shifts a rescale introduces. What is left to
correlate is edges and texture — the content. A border of the wide radius
is excluded from each window, because the crop's own edge and the
original's continuing picture blur differently there.

**Search.** Thumbnails keep their aspect, longest side 128. For B inside
A: window widths `s·W_A` for `s ∈ {1.00, 0.95, …, 0.50}` (a crop keeping
less than half a side — a quarter of the area — is not claimed), window
height from B's aspect, every integer offset. Coarse at 64 px (about five
million multiply-adds per direction), then the best placement refined at
128 px over ±1 scale step and ±2 px. Then A inside B (a letterbox is the
original inside a larger frame), B stretched onto A's whole frame and the
reverse (aspect changes), and rotations of ±2°, ±4°, ±6° at full frame
either way. The best ρ of all of these is the fit. Measured cost: 18 ms a
pair on the build machine with thumbnails in hand; about 40 ms including
the two decodes. It runs only where the cheaper evidence left a question.

**Where it is consulted.** Never over all pairs — as a confirmation:

1. a member the two 64-bit hashes grouped that PDQ did not confirm
   (crops, small rotations, letterboxes, watermarks — and, on real
   photographs, the same scene from another position);
2. a member the learned descriptor admitted (crops the hashes cannot
   reach).

**Measured** (`measure_geometry` in `tests/corpus.rs` and
`tests/real_corpus.rs`; full tables in VERIFICATION.md, Phase 6):

- Procedural positives at the shipped `FIT_MIN = 0.92`: crops 90/80/60 %,
  rotations 2°/5°, letterbox all ≥ 0.918 (minimum), 100 % recall; the
  real-photograph set at the 5th percentile: crop 90 % 0.963, crop 80 %
  0.929, crop 60 % 0.861, letterbox 0.950, rotate 5° 0.921.
- Negatives over every pair of distinct files: procedural max 0.929 (two
  value-noise textures, above the threshold); real photographs: the only
  pairs above 0.92 are two consecutive video frames (0.931) and two
  optical-flow frames (0.995) — the same picture both times; the highest
  unrelated pair is two text images at 0.918. Pairs the fit is actually
  asked about did better still: the chessboard photographed from other
  positions, which the hashes grouped and PDQ refused, fit at 0.31–0.45;
  the procedural `forest.png` the deep tier listed at 0.76 fit at 0.51.
- What it does not catch, by construction: mirror and 90° rotation
  (ρ ≈ 0.3–0.5; the hashes own those), rotations past 6°, crops under
  half a side, and text or line art, where band-passed strokes correlate
  across unrelated documents at 0.91 (never a candidate unless a hash or
  the descriptor admitted the pair first).

**What changes.** `milk` and `purge` act on a member only when two
families agree: the hash pair **and** (PDQ **or** the fit); or the deep
descriptor **and** the fit. A hash-pair member with neither prints as
`kept~` in both commands. On the real set this is the difference between
moving two distinct chessboard photographs and leaving them; on the
fixture it is the difference between leaving a 5 % crop (PDQ 32, one bit
over) and moving it (fit 1.00). The deep tier gains an acting path: a
letterboxed copy the hashes miss (p 20) is admitted by the descriptor
(0.78) and confirmed by the fit (1.00); `forest.png` (0.76, fit 0.51)
stays listed.

**Cells and the mirror — Phase 11 (CORE-01, CORE-02) [CLAUDE].** One
correlation over the whole window is sunk by a local change: a caption
bar over a fifth of the picture, a sticker, a retouched patch leave the
rest of a true copy identical and still pull ρ under 0.92. And a mirrored
copy correlates with its original at 0.3–0.5. Two additions to the
search, both at the placement the plain correlation chose (the search
itself is unchanged):

- **Cells** (`ncc_cells`): the window is cut into a 4 × 4 grid and each
  cell correlated on its own. A cell with almost no texture (variance
  under a twentieth of the mean cell's, in either picture) carries no
  evidence and is left out; of the rest the worst quarter is set aside
  and the others averaged — a trimmed mean, the robust estimator for a
  minority of outliers. Fewer than half the cells textured: no score.
- **Mirror**: the second picture is searched again left-to-right
  reversed, and the better of the two searches is kept (`mirror` on the
  line when it won).

The pair's score is the higher of the plain ρ and the cells' score, so
the new search never scores a pair lower than the old one did.

Measured (`tests/fit_measure.rs`; VERIFICATION § Phase 11): on the real
set's 60 pictures under 21 perturbations, recall at 0.92 goes from
88.1 % (plain) to 93.0 % (both); a caption bar over the lower 15 % from
20.0 % to 98.3 % (the cells); a mirrored crop to 80 % of the area from
3.3 % to 96.7 % (the mirror); the ELA pair from 0.766 to 0.885 (still
under, and listed); Copydays' hand-made copies from 20.5 % to 24.5 %.
What it costs in specificity: over every fourth pair of Copydays
originals (3,062 unrelated photographs) 13 reach 0.92 instead of 3
(maximum 0.969 against 0.947), and on the real set, where every pair of
distinct files was scored, the only unrelated pair added above 0.92 is
two text images (0.933, mirror). That is acceptable only because the fit
never decides alone: it is asked about a pair the hashes grouped or the
descriptor admitted, and among those the closest a photograph of one
subject from another position comes under the new search is 0.823 (two
poses of one chessboard, `right11` and `right14`). Cost: 33 ms a pair
against 20 ms (thumbnails in hand). `FIT_MIN` stays 0.92.

### 2.8 Capture moment — Phase 6 **[CLAUDE]**

Two photographs taken at different moments are two photographs, however
alike they look. A camera writes EXIF `DateTimeOriginal` at exposure
(with `SubSecTimeOriginal` where it keeps fractions); every editor and
exporter that preserves EXIF preserves it, so a copy, a re-encode, a
resize or a crop carries its source's moment. A burst, a stereo pair, the
same board from another angle each carry their own.

Rule: when both files state a moment and the two differ, the member is
`kin` — shown in its group with `taken <moment>`, never moved or binned,
whatever the hashes, PDQ or the fit say. When either file states none
(screenshots, renders, stripped exports) the rule says nothing and the
pictures decide. Fractions separate frames within one second only when
both sides recorded them.

The keeper of a near group is now the member whose moment the most
members share, before degree and pixels: a burst frame that strayed into
an exposure's family is the kin, not the family's keeper.

Known limits, all on the conservative side (a `kin` is never acted on):
two scans of one print carry two scan times and are shown as kin; a tool
that rewrites `DateTimeOriginal` on export turns derivatives into kin.
The real-photograph set used here carries no EXIF, so the rule is
verified on synthesised capture times (`capture.rs` builds a JPEG with an
EXIF segment in its tests; `tests/e2e.rs` stamps the fixture) and not yet
on a camera roll.

**On real cameras — Phase 11 (CORE-08) [CLAUDE].** Two sources: the
metadata dumps of `drewnoakes/metadata-extractor-images` (409 files from
real cameras and phones) and Google's HDR+ burst dataset (Hasinoff et
al., SIGGRAPH Asia 2016; every frame of a burst as the phone wrote it).
What they showed, and what changed:

- *Fractions are written at many precisions.* Of 362 files with a
  `DateTimeOriginal`, 75 (21 %) carry `SubSecTimeOriginal`, in 2, 3, 4
  or 6 digits; ten write `00`. An exporter may keep fewer digits than
  the camera: the phone's `117280` becomes `117` or `12`. The old rule
  (trailing zeros trimmed, then equal) called those another moment —
  a copy became `kin` and stayed. Now two fractions agree when the
  finer one, cut or rounded to the coarser one's digits, gives the
  coarser one (`fractions_agree`): `117280` agrees with `117`, `11728`,
  `12`, `11`, `1`; disagrees with `118`, `217`, `2`. A difference in
  precision is never evidence of another moment.
- *Bursts share their second, and often their fraction.* The four
  frames of an HDR+ burst all state `2016:07:27 18:59:22` and SubSec
  `117280`: the phone stamps the burst, not the frame. A camera that
  writes no fraction does the same at one-second resolution. By the
  capture rule alone, those frames were one moment, so the near tier
  read them as re-encodes of one picture, and `milk` would have moved
  three of the four frames (1.71 MB) as duplicates. A burst's frames differ in name instead: a camera
  numbers them (`IMG_0012`…`IMG_0015`, `payload_N000`…`N003`), while a
  copy changes the name's shape (`IMG_0012 (1)`, `IMG_0012 - Copy`) and
  a conversion keeps the number (`IMG_0012.png`). So two files of one
  moment whose names are equal once every run of digits is set aside,
  with a number different (`numbered_apart`), are another frame of that
  second: `kin`, worded `another shot, same second`, never acted on.
- *Dates are not always colon-separated.* One camera (a Samsung NV10)
  writes `2008.07.10  15:16:55`; the old parser required colons and lost
  it. Dots, dashes and slashes are read, and a `T` or doubled space
  before the time; an unset clock (zeros, blanks) is still no time.

The limits stay on the conservative side: a camera roll renumbered on
import (two copies of one frame under two numbers) shows the copy as a
frame — `kin`, left in place.

### 2.9 HEIF, HEIC and AVIF — Phase 10 **[CLAUDE]**

A phone photo today is a HEIC: HEVC-coded pictures in an ISO base media
file (ISO/IEC 23008-12, profiled by MIAF); AVIF is the same container
around AV1. The picture a viewer shows is the **primary item** (`pitm`),
and on a phone that item is not one coded picture but a `grid`: an
iPhone stores 4032×3024 as 48 HEVC tiles of 512×512 (6 × 8, cut to the
output size the grid states), a Sony A7M4 stores 7008×4672 as 6 tiles of
3520×1600 (the real files of VERIFICATION § Phase 10).

**Why the container is read here.** ffmpeg reads HEIF stills from 7.0
and its command line composes tiled ones from 8.1 (FFmpeg Changelog:
7.0 "Support for HEIF/AVIF still images and tiled still images", 8.1
"ffmpeg CLI tiled HEIF support"). The Windows machine this is verified
on has 9.0.2; the Linux build machine has 6.1.1, which reads no HEIF at
all. Every ffmpeg since 2013 decodes a raw HEVC stream, and with libdav1d
or libaom a raw AV1 one. So `heif.rs` reads the boxes (`ftyp`, `meta`:
`hdlr`, `pitm`, `iinf`/`infe`, `iloc`, `iref`, `iprp`/`ipco`/`ipma`,
`idat`), turns each tile into an elementary stream (HEVC: the `hvcC`
parameter sets and the length-prefixed NAL units rewritten as Annex B;
AV1: a temporal delimiter, then the item's OBUs), and `decode.rs` pipes
the stream to `ffmpeg -f hevc|obu`, reads one PPM per tile, composes the
tiles row by row and cuts to the grid's output size. One process per
file; the result is the same on every ffmpeg the person has.

**Transforms.** A HEIF's orientation is not EXIF's: the primary item's
transformative properties are applied in the order they are associated
with it — `clap` (clean aperture, ISO/IEC 14496-12 §12.1.4: centred at
`horizOff + (w − 1)/2`, `vertOff + (h − 1)/2`), `irot` (n × 90°
anticlockwise), `imir`. The `imir` field changed meaning between
editions of the standard (an "axis" in the first, a "mode" later), so
it was **measured**, not read: the AVIF test set's Link-U files state
what each must display as and ship the encoded picture as PNG
(AOMediaCodec/av1-avif `testFiles/Link-U`). `kimono.mirror-horizontal`
carries `imir 1` and needs a left-right flip to equal `kimono.png`
(mean difference 0.0; the top-bottom flip: 54.5); `kimono.mirror-vertical`
carries `imir 0` and needs top-bottom (0.0 against 54.5);
`kimono.rotate90` carries `irot 3` and needs 270° anticlockwise (0.0
against 54.5); `kimono.mirror-vertical.rotate270` carries `irot 1` then
`imir 0`, and rotating then flipping in that order gives 0.0 (the other
order: 54.5). The `clap` formula lands `kimono.crop` on offset (272, 39),
the best match of its nine neighbours against `kimono.crop.png`. On real
iPhone files `irot 3` sits with EXIF "Rotate 90 CW" (two files), and the
church interior of one of them decodes upright.

**Colour.** `colr nclx` in the container and the coded stream's own
signalling (HEVC VUI) can disagree. The Sony A7M4 HEIF says limited
range in `nclx`; its HEVC stream says full range. Its own JPEG thumbnail
item, decoded independently by the `image` crate, was compared with the
composed picture at the thumbnail's size: the stream's reading differs
by 4.3 levels per pixel on average (mean 187.4 against the thumbnail's
187.9), the container's by 9.2 (mean 195.8). So the stream decides;
`nclx` is not applied over it. Where the two agree (the AVIF set's
`kimono.avif`: limited, BT.2020 in both) the choice changes nothing, and
the decode is 2.8 levels from the original PNG. A stream that signals
nothing is decoded as ffmpeg's default (limited range) whatever `nclx`
says: a contrast difference the hashes tolerate.

**The capture moment** of a HEIF is the `Exif` item that describes the
primary (`cdsc`): a 4-byte offset, then a TIFF structure, read by
`kamadak-exif`.

### 2.10 Camera RAW — Phase 10 **[CLAUDE]**

Nothing here demosaics sensor data. Every RAW format carries the
camera's own JPEG rendering of the shot, and that preview is what is
hashed — but where it is differs per maker (TIFF IFDs, maker notes, a
CR3 `PRVW` box, a RAF header field), and on a real set of 18 cameras
(metadata-extractor-images: Canon CR2/CR3, Nikon NEF, Sony ARW, DNG
from DJI, Leica and an LG phone, Fuji RAF, Olympus/OM ORF, Panasonic RW2,
Pentax PEF) previews ranged from full size (Canon, Nikon, Pentax: up to
7360×4912) to 1616–3200 pixels (Sony, Fuji, Olympus, Panasonic) to none
(the phone DNG).

**Finding the preview.** One reader for all makers: scan for the JPEG
start marker and accept a candidate only when its whole marker chain
parses — well-formed segments inside the file up to a baseline or
progressive frame header (8-bit, 1 or 3 components, 16–16384 pixels a
side), quantisation tables before the scan, the entropy-coded data walked
to its EOI — and use it only if it decodes to the size its header
states. The naive scan of the same set finds byte runs in compressed
sensor data that begin like a JPEG and claim frames of 64328×14895 or
38581×40422 (one would be a 958-megapixel allocation); the chain check
rejects them. A RAW without a preview goes to ffmpeg's decoder (it reads
the LG phone's DNG), else it is reported as `RAW without a preview`.

**Which preview.** The hashes work at 288 and 512 pixels, so the
smallest preview with a short side of at least 1000 and the largest's
shape (within 1 %) is decoded — a 1620×1080 instead of a 7360×4912,
a twenty-first of the pixels — else the largest.

**Orientation and moment** come from the RAW's own metadata: the TIFF
structure most makers use (Olympus and Panasonic with their own magic
number, rewritten to 42 for the reader), CR3's `CMT1`/`CMT2` boxes, and
for the rest (Fuji) the preview's own EXIF. The Nikon D810 and Sony A7M3
files are portraits (`Rotate 270 CW`) and decode upright; the Nikon D70's
preview is stored already turned while its EXIF says normal, and decodes
upright too. The OM-1's `DateTimeOriginal` is `0000:00:00 00:00:00` (the
clock was never set) and reads as no moment.

**What a RAW is allowed.** A RAW is the original exposure, so:
- it is the keeper of its near group (after the shared-moment rule, so a
  RAW from another exposure cannot capture a family);
- as a member — another RAW of the same shot, or after a person made a
  JPEG the keeper with `k` — it is `raw`: listed, never moved or binned,
  not even by hand;
- the camera's own JPEG of it (same folder, same name: `IMG_0041.CR2`
  and `IMG_0041.JPG`, not at another moment) is its `pair`: `RAW + JPEG`
  on the line, left with it by `milk` and `purge`, choosable by hand in
  the workspace — people who shoot RAW + JPEG keep both on purpose;
- any other derivative (an export elsewhere) is a near duplicate of the
  RAW under the usual two-family rule, described `from the RAW`: its size
  is not compared with the RAW's, because the size of a RAW's picture is
  not read (cameras disagree on what their "image width" field means:
  the Fuji and Panasonic files state their preview's size), only its
  preview's;
- a byte-identical copy of a RAW is an exact duplicate like any other.

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
time per union (`near/group.rs`). A member linked only transitively shows
its real distance to the keeper, in the warning colour when it exceeds a
threshold. Members the keeper does not confirm are joined again over the
edges among themselves; a component that confirms a member against its
own keeper leaves as a group of its own, so a copy chained into a
stranger's group through one near-threshold edge is still acted on
(Phase 15, CORE-17) **[CLAUDE]**. Exact-duplicate groups are equivalence classes of one hash
value and need no graph; the near pass runs over exact-group keepers and
singletons only, so a byte copy is never reported twice.

Near keeper **[CLAUDE]**: the member sharing a known capture moment with
the most others (a stray burst frame is the kin, not the family), the
camera RAW, then the most matched — the members matching it directly on
the two 64-bit hashes with PDQ agreeing (Phase 15: a match on the 64-bit
hashes alone, near their thresholds, no longer counts) — then most pixels
of the picture inside any bars (a downscaled copy is the derivative), the
path not named or filed as a copy, oldest, largest file (at equal age and
size a lossless encoding outweighs a lossy one), path. Within one match
of the most matched, that degree does not decide: the rest of the order
does, so one near-threshold match more cannot seat a copy over the
original, and pixels still come before the copy marks (Phase 15,
CORE-13).

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
and a matching toolchain; `ffmpeg`/`ffprobe` are looked for on PATH, then
where WinGet, Scoop, Chocolatey or Homebrew put them, or beside `dedupe`
(`tools.rs`, Phase 11 on Windows); `doctor` says where they were found,
and a run that wanted the pass without them says so and finishes the
exact tier. Concurrency: ⌊cores/2⌋ clamped to 1–4 ffmpeg processes, each
with the machine's threads over that count, at least 2 (`-threads`;
Phase 14), the longest file first so a long one never runs alone at the
end; a file of ten minutes or more asks for the graphics hardware's
decoder (`-hwaccel auto`) and falls back to software when that run fails
(`DEDUPE_HWACCEL=0` turns it off) **[CLAUDE]**.

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

### 3.3 Conformance with the reference — Phase 7

`examples/tmk_ref.rs` parses the `.tmk` files Meta ships for its fifteen
sample videos (`tmk/sample-hashes`, produced by the reference
`tmk-hash-video`; layout from `io/tmkiotypes.h`: three 4-byte magics,
five `int`s, the periods, the coefficients, the pure average, then
cos[period][coefficient] and sin[period][coefficient], all little-endian
`float`) and scores them against our descriptor of the same files.

- Fed the reference tool's frames (ffmpeg `-s 64:64 -pix_fmt rgb24 -r 15`,
  Rec. 601 luma, the 64×64 straight to the DCT as `pdqFloat256FromFloatLuma`
  does when the input is already 64×64), our descriptor matches every
  reference file element for element to within 6.1·10⁻⁵ (the files store
  `float`), frame counts equal, level 1 and level 2 = 1.0000.
- Fed our shipped frames (288-px area-scaled grey → Jarosz → 64), the same
  video scores level 2 0.982–0.9999 against its reference file, and the
  level-2 pair scores between videos sit within 0.013 of the scores the
  reference files give each other across the fifteen related and
  unrelated pairs tried. The frame paths differ (ours filters before decimating; the
  tool's `-s 64:64` is bicubic), and the frame counts by one or two
  (`fps=15` against `-r 15`); the descriptor is the same.

### 3.4 Black bars — Phase 7 **[CLAUDE]**

Meta's sample set has the same clip exported with and without pillarbox
bars (`chair-orig-22-sd-bar`, 480×720 with 32 px black pillars, against
`chair-orig-22-hd-no-bar`, 720×1280). The reference's own hashes score
that pair at level 2 0.738 — over the README's 0.7 by a little, under
our 0.8 (set where the synthetic negatives stopped, §3.2) — and the
positional tier is worse off, since a bar shifts every frame's content. TMK does not see past bars (§3.1: letterboxing
is one of its weak cases — the frame feature is the DCT of the whole
frame, bars included); the field answer is to find the bars first
(czkawka's video matcher has a crop-detect option; ffmpeg ships
`cropdetect`).

Before the frame pass, one ffmpeg pass over the keyframes alone
(`-skip_frame nokey`, so no inter-frame decoding) runs
`cropdetect=limit=24:round=2:reset=0:skip=0` and the last `crop=W:H:X:Y`
it prints — with `reset=0` the box only grows, so it is the union over
every keyframe — becomes the picture. Two guards: bars thinner than 2 %
of a side are ignored (an encoder's edge, not a bar), and a box smaller
than a quarter of a side is a dark video, not a picture in bars, and
nothing is cropped. The frame pass then inserts `crop=W:H:X:Y` before the
scale, so the hashes see the picture at full resolution; the file's own
dimensions stay in the listing and a `bars` mark on the line says the
frames were compared inside them. Limit 24/255 is ffmpeg's default:
limited-range black is 16 and compression noise on a bar stays well
under 24. Non-black bars are left to a second pass (Phase 11, below);
the same file's dims are what the listing shows regardless.

The video keeper order counts the picture's pixels, not the file's —
inside the bars when there are any — and at equal pictures keeps the
file without bars: a padded export is a packaging of the picture it was
padded from, and under "most pixels" of the file it would have taken the
keeper seat (the fixture's 800×360 pillarboxed copy outranked its
640×360 source until this was changed). The footer still reads
`keep: longest, then most pixels`; the pixels are the picture's.

Measured on the sample set (VERIFICATION.md, Phase 7): the bar pair
scores level 2 1.00 and joins the group; the `pattern` family with bars
moves from 0.74–0.80 to 0.86–0.94 against its keeper; the two groups
that were one clip become one group. On the real AVI set, which has no
bars, nothing changes. The pass costs 15 % of the frame stage on h264
clips with keyframes every few seconds, and 32 % on all-intra AVI, where
every frame is a keyframe.

**Bars of any colour — Phase 11 (CORE-03) [CLAUDE].** Platforms post a
portrait clip in a landscape frame between white or coloured bars, or
over a blurred, enlarged copy of itself ("ambient"); `cropdetect` sees
none of these. When the black pass finds nothing, a second pass reads
up to 16 keyframes at 256 wide (`-skip_frame nokey`, each keyframe once)
and measures *detail*: for each column the mean step between vertically
adjacent pixels, for each row between horizontally adjacent ones — along
the band, so a bar's own edge never counts as the bar's detail. A frame
whose picture (the middle half of the columns or rows) has almost none
— a fade, a black frame — is set aside. Two edge bands are bars when, in
every remaining frame, each of their lines has under `FLAT_RATIO` (0.25)
of the picture's median detail; the two bands are within 2 % of the side
of each other, leave a quarter of the side, and the picture starts at
once — its first line, or the next when the edge falls inside a sampled
pixel, carries `FLAT_START` (0.4) of the picture's detail — so a sky or
a wall that fades into texture is not a bar.

Measured on the 25 real clips of Phases 7 and 10, each made a portrait
and framed four ways, and on the 79 clips, re-encodes and portraits that
have no bars (`tests/bars_measure.rs` and a ratio sweep; VERIFICATION §
Phase 11): white, grey and dark-blue pillarboxes are found in all 75
copies, and cropping to the picture takes the copy from TMK level 2
0.00 against its original to 0.98–1.00 in 70 of them (0.88–0.89, still
grouped, in the rest); a white letterbox where the clip leaves one, in 5
of 6; the blurred "ambient" bars in 7 of 25 — only where the picture
has much more detail than its blurred copy; in 5 others the black pass
takes a darkened blurred bar for black and crops wrongly, as it did
before. Of the 79 clips without bars, none shows any at the shipped
0.25 and 0.4; the first to show some does at a ratio of 0.33 (start 0.5)
or a start share of 0.3 (ratio 0.25) — a thermal camera's fixed overlay
strips, and a 190 × 240 sample clip. The
pass costs 16 % of the frame stage on those 29 clips (25 s → 29 s), whose
groups and words are unchanged.

Audio fingerprinting (Chromaprint) is out of scope.

## 4. Undo — the journal

`--undo` **[SPEC]** reverses the last change. A reversal needs a record of
what moved where; that record is written **only when a command changes
the directory** (never on `find`, never on `--dry-run`). The app folder
also holds two small conveniences, never in the scanned tree: the last
folder looked at (`recent.json`, Phase 9) and the skip list
(`skip.json`, Phase 10). It lives outside the scanned tree, in the OS's local
application data directory (`%LOCALAPPDATA%\dedupe\journal\` on Windows,
`~/.local/share/dedupe/journal/` on Linux) **[CLAUDE]**, so no file is ever
added to a user's directory except the `_milked` folder the spec defines.

- `milk` journals each move as (source, destination, size, hash).
  Undo = move back, in reverse order; a destination that is no longer there
  is reported, not fabricated.
- A move is a rename. Across volumes — a folder of the root that is a
  mount point, or `_milked` made a link to another drive — the system
  refuses the rename (EXDEV, ERROR_NOT_SAME_DEVICE) and the move becomes
  copy, `fsync`, read the copy back and compare its BLAKE3 with the
  original's, carry the times and the read-only bit over, delete the
  original; if the original will not go, the copy is removed again
  (Phase 10, `fsops.rs`). The journal marks such moves `copied`; undo
  crosses back the same way. **[CLAUDE]**
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
**[CLAUDE]**: keep the file whose name and folders do not mark it as a
copy (Phase 11, below); then the earliest modification time (the original
is older than its copies); tie → fewest path components (the copy in a
nested `backup/old/` folder is the duplicate); tie → shorter name; tie →
lexicographic path. The policy is printed in every result footer so it is
never hidden.

**Provenance — Phase 11 (CORE-07) [CLAUDE].** A modification time is when
a file was last written, and a download, a sync or a copy tool writes it
anew: a copy can be "older" than its original. On the owner's Windows
sample tree (`winsample`, VERIFICATION § Phase 11) the times put
`exports\graf1_mirror.png` in the keeper seat over the original, and with
the copies made older than their originals (as a restore from backup does)
"oldest" kept six copies. A copy's name and folder are written by the
tool that made it, and they say what it is. `keeper::derived` counts the
marks: one for a copy's name — `x (1)`, `x(2)`, `x - Copy`, `x copy`,
`x_copy`, `Copy of x`, `x-export`, `x-edited` (whole words, case ignored;
`copyright.jpg` is not one) — and one for each folder on the path named
`export(s)`, `exported`, `download(s)`, `temp`, `tmp`, `cache`,
`thumbnails`, `thumbs`, `backup(s)` or `copies`. Folders are counted, not
merely noticed: a scan inside `Downloads` or a temporary folder marks
every file of a group alike, and the `exports` below still tells the copy
from the original. Fewer marks keep; at equal marks, the times decide as
before. On the sample tree the keepers become the thirteen originals,
whatever the times say.

Near and video groups put the same count after their own criteria
(near: shared moment, the RAW, most matched, most pixels; video:
longest, most pixels inside the bars, no bars, bitrate), where the old
order went straight to the time: `IMG_0012 (1).jpg`, a copy of
`IMG_0012.jpg` with the same pixels and the same matches, no longer takes
the seat when its time is the earlier one (a folder copied in name order
writes it first). The date on a `keep` line is the local calendar date of that
modification time (Phase 5), the same day Explorer shows; the comparison
itself uses the full timestamp.

The policy is a default, not a verdict: on the result workspace (Phase 8,
CLI.md §2a) the `k` key re-elects a keeper by its number, and the group's
relations are recomputed against the new keeper — for a near group that
is `relation_of` plus the geometric fit where the cheaper evidence leaves
the question open, for a video group `relate`; an exact group only
reorders, every member being the same bytes.

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
- Normalised cross-correlation for template matching — J. P. Lewis, "Fast Normalized Cross-Correlation", Vision Interface 1995, http://scribblethink.org/Work/nvisionInterface/nip.html
- EXIF 2.32 (CIPA DC-008-2019), tags 0x9003 DateTimeOriginal and 0x9291 SubSecTimeOriginal — https://www.cipa.jp/std/documents/e/DC-X008-Translation-2019-E.pdf; reader: `kamadak-exif` — https://github.com/kamadak/exif-rs
- `trash` crate — https://docs.rs/trash
- HEIF: ISO/IEC 23008-12 (Image File Format), MIAF ISO/IEC 23000-22; FFmpeg Changelog (7.0 HEIF/AVIF stills and tiled stills; 8.1 CLI tiled HEIF) — https://github.com/FFmpeg/FFmpeg/blob/master/Changelog; AVIF specification — https://aomediacodec.github.io/av1-avif/; AVIF transform test files (Link-U) — https://github.com/AOMediaCodec/av1-avif/tree/master/testFiles/Link-U; HEIF conformance files — https://github.com/nokiatech/heif_conformance; libheif examples — https://github.com/strukturag/libheif/tree/master/examples
- Clean aperture: ISO/IEC 14496-12 §12.1.4 (`clap`)
- Real camera and phone files with independent metadata dumps — https://github.com/drewnoakes/metadata-extractor-images (heic, heif, avif, cr2, cr3, nef, arw, dng, raf, orf, rw2, pef)
- `image_hasher` crate (evaluated, not adopted) — https://lib.rs/crates/image_hasher
