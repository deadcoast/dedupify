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

### 2.2 Decision **[CLAUDE]**

Phase 2 implements **dHash and pHash in-house** (both are ~60 lines each; the
DCT is a 32×32 matrix product), with:

- dHash as the first-pass filter at normalised threshold 0.159 → **≤ 10 of 64
  bits** for "likely same image";
- pHash agreement at normalised threshold 0.224 → **≤ 14 of 64 bits**
  required to confirm a pair;
- a pair is a near-duplicate when **both** agree. The two hashes fail on
  different perturbations (dHash is a gradient sign, pHash is a frequency
  magnitude), so conjunction lowers the false-positive rate below either
  alone. The combined false-positive rate is not simply the product (the
  errors are correlated on the same image pair); Phase 2 verification
  measures it on a generated corpus before the thresholds ship.

PDQ (256-bit, with quality gating) is scheduled for Phase 2b as the
high-precision tier used by `purge`; the paper's threshold of 30/256 and its
quality gate carry over. The `image_hasher` crate (a fork of `img_hash`) was
evaluated and not adopted: no PDQ, uncertain maintenance
([lib.rs](https://lib.rs/crates/image_hasher)), and owning the DCT means the
thresholds above are tied to code we control.

Horizontal mirror: none of the block hashes tolerate it (Zauner). Phase 2
hashes each image **and its mirror** and stores both (2 × 64 bits); a match
against either counts. Cost: one extra 9×8 and 32×32 pass per image, which is
negligible against decode time.

Learned descriptors (SSCD — Pizzi et al., CVPR 2022,
[paper](https://openaccess.thecvf.com/content/CVPR2022/html/Pizzi_A_Self-Supervised_Descriptor_for_Image_Copy_Detection_CVPR_2022_paper.html),
trained for copy detection on DISC2021 and reported to beat SimCLR
descriptors by 48 % absolute) are the current frontier for crops, rotations
and heavy edits. They require a ~100 MB model and an inference runtime. They
are scheduled as an optional Phase 4 tier, off by default, because they
answer a different question ("same picture, edited") than the core tool
("copies of this file").

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

Decision **[CLAUDE]**: Phase 2 uses linear scan when n ≤ 20,000 (bounded at
2 × 10⁸ popcounts, well under a second) and MIH above that. The crossover is
measured in Phase 2 verification, not assumed.

### 2.4 Grouping

Near-duplicate relations are not transitive (A≈B, B≈C does not force A≈C),
so groups are connected components of the match graph. Union–find with path
compression and union by rank (Tarjan 1975) gives near-constant amortised
time per union. Exact-duplicate groups in Phase 1 are equivalence classes
of one hash value and need no graph.

---

## 3. Near-duplicate video

### 3.1 Evidence

**TMK + PDQF** (Temporal Match Kernel on PDQ float features; Meta 2019; test
drive in [arXiv 1912.07745](https://ar5iv.labs.arxiv.org/html/1912.07745)):
videos are resampled to 15 fps; each frame yields PDQF (the PDQ float
vector before binarisation); features are averaged over periods and combined
with trigonometric weights into a fixed-size descriptor (level 1 ≈ 1 KB,
level 2 ≈ 256 KB). Two-phase match: cosine similarity on level 1 and a
level-2 score, both at threshold 0.7. Robust to bitrate, format and
half-scale changes; trimming performs well; a 5-second title card
"dramatically" reduces matching; crops beyond 20 % are improbable to detect.

The underlying kernel is from Poullot, Tsukatani, Nguyen, Jégou, Satoh,
*Temporal Matching Kernel with Explicit Feature Maps*, ACM Multimedia 2015.

### 3.2 Decision **[CLAUDE]**

Phase 3 implements video matching in two tiers:

1. **Positional frame hashing.** Decode k = 12 frames at normalised
   positions t_i = (i + 0.5)/k × duration via `ffmpeg` as a subprocess
   (`-ss` seek, `-frames:v 1`, scaled to grayscale 32×32 raw output), hash
   each frame with dHash + pHash from Phase 2, and compare two videos by the
   median per-position Hamming distance. Normalised positions make the
   comparison invariant to re-encoding and resolution; they are not
   invariant to trims, which tier 2 covers. ffmpeg is used as a process, not
   linked, because linking FFmpeg into a Rust build on Windows requires
   vcpkg and a matching toolchain; `--doctor` checks that `ffmpeg` and
   `ffprobe` are on PATH.
2. **TMK-style descriptor** for trim/offset robustness, following the
   published pipeline (15 fps resample, per-frame float features,
   period-averaged trigonometric pooling, two-phase 0.7 thresholds).

Audio fingerprinting (Chromaprint) is out of scope.

---

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

---

## 5. Keeper selection

Within a group, one file stays and the rest are duplicates. Policy
**[CLAUDE]**: keep the file with the earliest modification time (the original
is older than its copies); tie → fewest path components (the copy in a
nested `backup/old/` folder is the duplicate); tie → shorter name; tie →
lexicographic path. The policy is printed in every result footer so it is
never hidden.

---

## 6. Sources

- BLAKE3 paper — https://cgi.cse.unsw.edu.au/~cs4601/refs/papers/blake3.pdf
- Needle In A Haystack, Fast (perceptual hash benchmark) — https://arxiv.org/pdf/2206.00282
- Zauner, Implementation and Benchmarking of Perceptual Image Hash Functions — https://phash.org/docs/pubs/thesis_zauner.pdf
- PDQ & TMK+PDQF test drive — https://ar5iv.labs.arxiv.org/html/1912.07745
- Norouzi, Punjani, Fleet, Fast Exact Search in Hamming Space with Multi-Index Hashing — https://ar5iv.arxiv.org/html/1307.2982 (PAMI version: https://www.cs.toronto.edu/~fleet/research/Papers/MIH-pami2014.pdf)
- Pizzi et al., A Self-Supervised Descriptor for Image Copy Detection — https://openaccess.thecvf.com/content/CVPR2022/html/Pizzi_A_Self-Supervised_Descriptor_for_Image_Copy_Detection_CVPR_2022_paper.html
- `trash` crate — https://docs.rs/trash
- `image_hasher` crate (evaluated, not adopted) — https://lib.rs/crates/image_hasher
