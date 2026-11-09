# VERIFICATION — Phase 1, run on 2026-11-05

Environment: Linux (kernel 6.18), 2 vCPUs, 7.8 GB RAM, Rust 1.97.0,
release profile. Windows was **type-checked** (`cargo check --target
x86_64-pc-windows-msvc`) so every `cfg(windows)` path compiles; it was not
executed on Windows. Numbers below are from this machine, with the files
already in the page cache; cold-disk timings depend on the drive.

## Automated

`cargo test` — 36 unit tests + 9 end-to-end tests, all passing.
`cargo test -- --ignored` — the purge → undo round trip through the OS
bin (freedesktop trash here; Recycle Bin on Windows), passing.
`cargo clippy --all-targets` — clean.

End-to-end coverage (tests/e2e.rs): `find` and `find .` counts per stage;
a same-size, same-sample, different-content decoy never reported; `-q`
reporting it as LIKELY; `milk -n` touching nothing and writing no journal;
`milk .` then `find .` reporting no duplicates; `--undo` restoring the
exact file list and pruning `_milked`; name collision inside `_milked`
getting ` (2)`; `purge -n` verifying bytes and changing nothing; `img`
filter excluding videos; every error path exiting 1 with the help hint;
help and doctor pages.

## Manual, on a generated tree

17 files, 6.4 MB, including a 4-way image group, a 2-way video group, a
pair differing only in the middle byte (sample-equal), a pair differing in
byte 5 (sample-different), two empty files, and a path 150 characters
deep.

| run | observed |
|---|---|
| `find .` | 15 candidates (2 empty skipped) → 13 share a size → 12 share a sample → 10 confirmed; 4 groups, 6 duplicates, 2.99 MB reclaimable; bytes hashed 5.76 MB (the four 300 KB copies, the two 2 MB videos, and the sample-equal pair) |
| `find` (no `.`) | 1 group: the two notes files in the root |
| `find . -q` | 5 LIKELY groups; the sample-equal decoy pair included and labelled |
| `milk . -n` | 6 `would` lines, no journal, tree untouched |
| `milk .` | 6 moved under `_milked/<same relative path>`, journal written |
| `find .` after milk | no duplicates (`_milked` is never scanned) |
| `--undo -n` | 6 `would restore`, newest first |
| `--undo` | 6 restored; `_milked` removed; second `--undo` reports nothing pending |
| `purge . img -n` | 4 verified byte for byte, 4 `would` lines |
| `purge . img` | 4 binned to the trash, journal written |
| `--undo` | 4 restored from the bin; `cmp` against the keeper: identical |
| `doctor` | 11 checks, all PASS, including bin reachable and ffmpeg found |
| `--help` | the designed page |
| errors | unknown flag, no command, missing folder, two paths → exit 1 |

Under a pseudo-terminal: 24-bit colour sequences present, Unicode glyphs
used, each stage line redrawn in place (`\r` + clear-line) and left in
the scrollback on completion. Piped: plain text, ASCII glyphs, no escapes.

Menu, driven through a pseudo-terminal with scripted keys
(enter, enter, down, enter, enter, enter): WORKFLOW → FOLDER → SCOPE →
KIND → CONFIRM frames rendered at exact frame width; CONFIRM showed
`dedupe find <folder> .`; the run produced the same output as the typed
command.

## Scale

2,300 files of 256 KB (575 MB): 1,500 originals, 500 copies, 300
same-size decoys differing in one middle byte.

| run | wall | result |
|---|---|---|
| `find .` | 0.38 s (first), 0.14 s (repeat) | 435 groups, 518 duplicates, 130 MB reclaimable; 353 MB hashed |
| `find . -q` | ~0.1 s | 613 groups, 800 "duplicates" — the 300 decoys counted, as the LIKELY label warns |

Peak resident memory 13 MB.

## Not verified here

- Execution on Windows (type-checked only): Recycle Bin send/restore via
  `trash::os_limited`, virtual-terminal enabling via crossterm, path
  separators in output.
- Behaviour on a tree with hundreds of thousands of files (walker is
  single-threaded `walkdir`; hashing is parallel).
- Terminals that are not UTF-8: the ASCII set is selected only by
  `DEDUPE_ASCII=1` or non-terminal output, not by code-page detection.

---

# VERIFICATION — Phase 2, run on 2026-11-05

Same environment (Linux, 2 vCPUs, Rust 1.97, release). Windows
type-checked again (`cargo check --target x86_64-pc-windows-msvc`), not
executed.

## Automated

`cargo test --release` — 48 unit tests + 12 end-to-end tests, all passing.
New unit coverage: DCT-II against the naive double sum; dihedral group
relations on the grid; mirrored and 90°-rotated files matching the
corresponding variant within 2 bits; 25 % rescale and q30 JPEG inside the
thresholds; flat image below the quality gate; EXIF orientation 6 applied
at decode; MIH query equals brute force at radii 0/3/7/10; MIH pairs equal
linear pairs; planted copies found and random pairs not; union–find.
New end-to-end coverage: `find . img` reports the exact groups first and
the NEAR group after, keeper by pixels, mirror labelled, `+GROUP` link,
flat image skipped, the different picture excluded; plain `find .` prints
the `near` hint and no NEAR; `purge -n` runs the near pass with `would~`
lines; `milk . img` moves 6 (3 exact, 3 near) and `--undo` restores the
exact file list.

## Threshold corpus — `cargo test --release --test corpus -- --ignored --nocapture`

150 procedural photographs (value noise, soft ellipses, hard rectangles,
tint; 640×480 to 900×1200) × 21 perturbations = 3,300 signatures. 22.5 ms
per perturbed image including the transform itself.

| perturbation | d max | d p95 | p max | p p95 | recall |
|---|---|---|---|---|---|
| scale 50 % | 1 | 0 | 2 | 0 | 100 % |
| scale 25 % | 2 | 1 | 2 | 0 | 100 % |
| upscale 150 % | 1 | 0 | 2 | 0 | 100 % |
| JPEG q90 | 1 | 0 | 2 | 0 | 100 % |
| JPEG q60 | 2 | 1 | 2 | 2 | 100 % |
| JPEG q30 | 3 | 1 | 2 | 2 | 100 % |
| crop 90 % | 16 | 13 | 20 | 18 | 70.0 % |
| crop 80 % | 25 | 22 | 32 | 26 | 2.7 % |
| crop 60 % | 34 | 29 | 36 | 32 | 0 % |
| rotate 2° | 10 | 6 | 12 | 8 | 100 % |
| rotate 5° | 21 | 13 | 16 | 14 | 77.3 % |
| rotate 90° | 0 | 0 | 0 | 0 | 100 % |
| mirror | 0 | 0 | 0 | 0 | 100 % |
| brightness +20 % | 2 | 1 | 6 | 4 | 100 % |
| brightness −20 % | 3 | 1 | 2 | 2 | 100 % |
| gamma 1.5 | 8 | 5 | 14 | 8 | 100 % |
| watermark | 2 | 2 | 24 | 20 | 76.7 % |
| blur | 1 | 0 | 2 | 0 | 100 % |
| noise ±10 | 3 | 1 | 2 | 2 | 100 % |
| letterbox | 14 | 11 | 26 | 20 | 62.0 % |
| squeeze 75 % | 1 | 1 | 2 | 0 | 100 % |

Negatives: 480,525 pairs (source vs source, every variant vs every other
source). **False positives at d≤10 ∧ p≤14: 0.** Closest negative: d 5,
p 10, d+p 26.

Threshold sweep — recall over the perturbations a block hash can be
expected to survive (all but rotate 2°/5° and crop 60 %) | false positives:

| d \ p | ≤10 | ≤12 | ≤14 | ≤16 | ≤18 | ≤20 |
|---|---|---|---|---|---|---|
| ≤6 | 81.7 % \| 0 | 83.4 % \| 0 | 85.7 % \| 0 | 87.2 % \| 0 | 87.7 % \| 0 | 88.1 % \| 0 |
| ≤8 | 83.2 % \| 0 | 85.5 % \| 0 | 88.3 % \| 0 | 90.0 % \| 0 | 90.8 % \| 0 | 91.3 % \| 0 |
| **≤10** | 83.7 % \| 0 | 86.5 % \| 0 | **89.5 % \| 0** | 91.4 % \| 1 | 92.4 % \| 1 | 93.0 % \| 1 |
| ≤12 | 84.3 % \| 0 | 87.4 % \| 0 | 90.6 % \| 0 | 92.8 % \| 1 | 93.9 % \| 2 | 94.7 % \| 3 |
| ≤14 | 84.4 % \| 0 | 87.7 % \| 0 | 91.0 % \| 2 | 93.4 % \| 19 | 94.5 % \| 38 | 95.6 % \| 65 |
| ≤16 | 84.5 % \| 0 | 87.8 % \| 0 | 91.2 % \| 3 | 93.7 % \| 27 | 95.1 % \| 90 | 96.4 % \| 298 |

Both engines on the 3,300 real signatures: linear 0.056 s, MIH 0.043 s,
identical 25,299 pairs.

## Engine crossover — `cargo test --release --lib -- --ignored crossover --nocapture`

Random codes with planted copies, 2 vCPUs:

| signatures | linear | MIH |
|---|---|---|
| 2,200 | 0.024 s | 0.032 s |
| 5,500 | 0.154 s | 0.120 s |
| 11,000 | 0.599 s | 0.364 s |
| 22,000 | 2.44 s | 0.80 s |
| 44,000 | 10.4 s | 2.29 s |

Shipped `LINEAR_MAX = 4,000`.

## Scale — 423 photographs, 33.8 MB

300 originals at 1600×1200 (JPEG q88), 60 downscaled 800×600 copies
(q70), 43 re-encodes (q60), 20 mirrors (q88), planted so that 94 groups
exist: 68 of two, 23 of three, 3 of four.

`find . img`: 3.8 s wall, of which DECODE 3.8 s (9 ms per image, two
cores), MATCH under 0.05 s. Result: **94 near groups, 68/23/3 by size —
exactly the planted structure**; 0 false merges; one member displayed over
threshold relative to its keeper (linked through the mirror), shown in the
warning colour as designed.

## Not verified here

- Execution on Windows (type-checked only).
- Real photographs: the corpus is procedural. Rates above are the
  measurement that fixed the thresholds; a photo library will show more
  structure per image and fewer accidental collisions than value noise.
- HEIC/AVIF/RAW are not decodable by this build's `image` features; they
  pass through the exact tier and count as `undecodable` in DECODE.
- Decode speed on 24-megapixel camera files (~10× the pixels timed here).

---

# VERIFICATION — Phase 2b (PDQ), run on 2026-11-07

Same environment. Windows type-checked, not executed. `cargo test
--release`: 56 unit + 12 end-to-end tests passing; `cargo clippy
--all-targets` clean.

## Unit coverage added

Window-size formula against the reference's values; the 1-D box pass
against a direct window average for w ∈ {2, 3, 7, 8} and the reference's
worked example (w = 8: output 0 averages 0..=4, output 4 averages 1..=8);
the all-columns pass against the per-column 1-D pass; a horizontal ramp
through filter + decimation stays a strictly increasing ramp with
identical rows; every DCT-domain dihedral variant within 2 bits of the
pixel-domain transform of the 64×64 buffer, and each non-identity variant
more than 40 bits from the identity; quality 0 on a flat buffer, 100 on a
0/255 checkerboard, ≥ 50 on textured synthetic content; exactly 128 set
bits; a 640×480 synthetic within 15 bits of its half-size and 1.5× Lanczos
resamples and bit-identical through u8 rounding; a 4×4 input yields
quality 0 and no hash.

## Corpus — `cargo test --release --test corpus -- --ignored --nocapture`

Same 150 × 21 corpus. `q` is the PDQ distance on the orientation the
64-bit pair chose; `q*` the best over all orientations.

| perturbation | q max | q p95 | q* max | recall q≤31 | d∧p∧q | d∧p only |
|---|---|---|---|---|---|---|
| scale 50 % | 8 | 8 | 8 | 100 % | 100 % | 100 % |
| scale 25 % | 14 | 10 | 14 | 100 % | 100 % | 100 % |
| upscale 150 % | 12 | 8 | 12 | 100 % | 100 % | 100 % |
| JPEG q90 | 8 | 6 | 8 | 100 % | 100 % | 100 % |
| JPEG q60 | 10 | 8 | 10 | 100 % | 100 % | 100 % |
| JPEG q30 | 16 | 10 | 16 | 100 % | 100 % | 100 % |
| crop 90 % | 110 | 102 | 110 | 0 % | 0 % | 70.0 % |
| crop 80 % | 148 | 138 | 124 | 0 % | 0 % | 2.7 % |
| crop 60 % | 140 | 136 | 128 | 0 % | 0 % | 0 % |
| rotate 2° | 62 | 50 | 62 | 22.0 % | 22.0 % | 100 % |
| rotate 5° | 116 | 98 | 110 | 0 % | 0 % | 77.3 % |
| rotate 90° | 18 | 16 | 18 | 100 % | 100 % | 100 % |
| mirror | 14 | 10 | 10 | 100 % | 100 % | 100 % |
| brightness +20 % | 16 | 12 | 16 | 100 % | 100 % | 100 % |
| brightness −20 % | 4 | 4 | 4 | 100 % | 100 % | 100 % |
| gamma 1.5 | 56 | 24 | 56 | 98.7 % | 98.7 % | 100 % |
| watermark | 100 | 90 | 100 | 0.7 % | 0.7 % | 76.7 % |
| blur | 4 | 2 | 4 | 100 % | 100 % | 100 % |
| noise ±10 | 12 | 6 | 12 | 100 % | 100 % | 100 % |
| letterbox | 142 | 122 | 124 | 0 % | 0 % | 62.0 % |
| squeeze 75 % | 10 | 4 | 10 | 100 % | 100 % | 100 % |

Negatives, 480,525 pairs: min q* **74**, 0.1st percentile 94, 1st
percentile 102, median 116, mean 115.5. False positives at q* ≤ T for
T ∈ {24, 28, 31, 36, 40, 48, 56, 64}: **0 at every T**. PDQ-alone recall
over the survivable perturbations: 77.6 % at T = 24, 77.7 % at 31, 80.4 %
at 64 — the ceiling is set by the crop/rotate/watermark/letterbox rows,
which no threshold under 64 reaches.

PDQ quality: 93 of 150 sources below 50 (procedural textures are smoother
than photographs). Their positives matched at the rates above and their
negatives never came within 74 bits.

### Small-input upscale floor (`MIN_SIDE`)

| MIN_SIDE | scale 25 % q max / p95 | scale 50 % q max | bright −20 % q max | gamma recall |
|---|---|---|---|---|
| none | 32 / 26 (99.3 %) | 12 | 4 | 98.7 % |
| 256 | 16 / 10 | 12 | 4 | 98.7 % |
| **384** | **14 / 10** | **8** | **4** | **98.7 %** |
| 512 | 10 / 8 | 8 | 8 | 98.0 % |

Negatives identical for all four. Shipped: 384.

## Manual, photo fixture (8 files)

| run | observed |
|---|---|
| `find . img` | GROUP 001 (export + byte copy); NEAR 002: keeper `beach.png`, mirror q 8, export q 2, 5 % crop d 10 p 6 **q 32** lit in warning colour, small q 4 |
| `purge . -n` | 1 `would` + 3 `would~` (q 8, 2, 4); `kept~ beach_crop.jpg  PDQ q 32 over 31`; footer names `dedupe milk .` |
| `purge .` | 4 binned through the OS trash; crop stays |
| `find . img` after | NEAR 001: keeper + crop only |
| `--undo` | 4 restored; file list identical to before |

## Scale — 423 photographs

`find . img`: 7.5 s wall before the column-pass rewrite, DECODE 16 ms per
image (two cores); after the rewrite the PDQ stage costs 3.1 ms per image
single-threaded on 800×600 prefiltered luma (Jarosz was 7.5 ms). 94 near
groups as before; **all 123 near members PDQ-confirmed, max q 6**; `purge
-n` would bin all 123.

Per-image profile (1600×1200 JPEG, single thread): decode 5.8 ms,
prefilter 3.9 ms, Lanczos grid 7.0 ms, PDQ 3.2 ms, dHash+pHash×8 1.2 ms.

## Not verified here

- Bit-identity with the reference PDQ tool (its Python wrapper could not
  be installed in this environment; the departures are documented in
  RESEARCH §2.2b).
- Windows execution.
- Real photographs: PDQ quality will sit higher than on this corpus; the
  recall figures for crop/rotate/letterbox are the mechanism's and will
  transfer.

---

# VERIFICATION — Phase 3 (video), run on 2026-11-07

Same environment; ffmpeg 6.1.1 with libx264. Windows type-checked, not
executed. `cargo test --release`: 63 unit + 15 end-to-end tests passing
(the three video end-to-end tests generate their clips with ffmpeg and
skip when it is absent); `cargo clippy --all-targets` clean.

## Unit coverage added

Position indices inside the frame range and increasing; frame-rate
fraction parsing; identical TMK descriptors score level 1 = level 2 = 1 at
offset 0; copies of a synthetic feature stream shifted by 45, 150 and 600
frames keep level 2 > 0.9 and level 1 > 0.9; independent streams score
below 0.3 on both; the normaliser c_0 + 2Σc_j equals 1.9292 from the
reference table; the grouping/binning rules on constructed relations
(TMK 0.85 groups and is not binned with the reason `TMK 0.85 under 0.9`;
frames d 2 p 3 q 40 group and are not binned with
`frames q 40 over 31; …`; frames d 20 p 30 do not group).

A note on what the TMK tests do not claim: with the reference's
per-coefficient normalisation the recovered time offset on a slowly varying
synthetic stream came out at 4 frames for a 45-frame shift. The score
stays high; the offset is approximate, is not asserted, and is not shown.

## Corpus — `cargo test --release --test video_corpus -- --ignored --nocapture`

16 sources (four Mandelbrot zooms, three Life runs, three cellular
automata, two test patterns, two gradient fields, SMPTE bars, RGB test),
20 s each at 640×360 libx264 crf 20; 12 edits each; 208 clips, 60,304
frames at 15 fps; generation 294 s, hashing 132 s (2.18 ms per frame
including ffmpeg).

| edit | d med | p med | q med | l1 min | l2 min | positional | TMK | grouped | binnable |
|---|---|---|---|---|---|---|---|---|---|
| re-encode crf30 | 2 | 2 | 6 | 0.957 | 0.948 | 94 % | 100 % | 100 % | 100 % |
| scale 50 % | 1 | 2 | 6 | 0.950 | 0.973 | 100 % | 100 % | 100 % | 100 % |
| crop 90 % | 9 | 14 | 88 | 0.501 | 0.493 | 50 % | 50 % | 56 % | 31 % |
| trim head 3 s | 16 | 20 | 104 | 0.762 | 0.890 | 19 % | 100 % | 100 % | 94 % |
| trim tail 3 s | 15 | 20 | 108 | 0.688 | 0.862 | 12 % | 100 % | 100 % | 88 % |
| trim both 2+2 s | 12 | 16 | 114 | 0.665 | 0.785 | 19 % | 94 % | 94 % | 88 % |
| title card 3 s | 15 | 20 | 116 | 0.994 | 0.989 | 12 % | 100 % | 100 % | 100 % |
| speed 1.1× | 1 | 2 | 16 | 0.975 | 0.982 | 94 % | 100 % | 100 % | 100 % |
| mirror | 0 | 0 | 4 | −1.000 | 0.010 | 100 % | 6 % | 100 % | 88 % |
| letterbox | 9 | 18 | 106 | −0.196 | 0.046 | 0 % | 25 % | 25 % | 0 % |
| fps 15 | 0 | 0 | 2 | 0.997 | 0.995 | 94 % | 100 % | 100 % | 100 % |
| brightness +0.1 | 1 | 2 | 8 | 0.990 | 0.986 | 100 % | 100 % | 100 % | 100 % |

Level 2 per edit (min / p10 / median): re-encode 0.948/0.981/1.000;
trim head 0.890/0.906/0.987; trim tail 0.862/0.876/0.971; trim both
0.785/0.834/0.982; title card 0.989/0.993/0.997.

Negatives, 3,000 pairs: max level 1 **0.817**, max level 2 **0.769**;
positional closest d 1 p 28 q 124. The six closest are all Mandelbrot
`mandel_a` against `mandel_d` and its edits.

TMK sweep — false groups | recall over the TMK-carried edits (all but
crop, mirror, letterbox):

| l1 \ l2 | ≥0.70 | ≥0.75 | ≥0.80 | ≥0.85 | ≥0.90 |
|---|---|---|---|---|---|
| ≥0.5 | 19 \| 100 % | 4 \| 100 % | **0 \| 99.3 %** | 0 \| 98.6 % | **0 \| 96.5 %** |
| ≥0.6 | 19 \| 100 % | 4 \| 100 % | 0 \| 99.3 % | 0 \| 98.6 % | 0 \| 96.5 % |
| ≥0.7 | 19 \| 98.6 % | 4 \| 98.6 % | 0 \| 97.9 % | 0 \| 97.9 % | 0 \| 96.5 % |
| ≥0.8 | 2 \| 97.9 % | 2 \| 97.9 % | 0 \| 97.2 % | 0 \| 97.2 % | 0 \| 96.5 % |

Shipped: gate l1 ≥ 0.5; group l2 ≥ 0.8; bin l2 ≥ 0.9. With these: **0
false groups, 0 false confirms** across the 3,000 negatives.

## Manual, six clips

`find . vid` on a fractal zoom, its byte copy, a 320×180 crf-32 re-encode,
a mirror, a 4-second head cut, and two unrelated clips: GROUP 001 (zoom +
byte copy); VIDEO 002 with the zoom as keeper (`keeper of GROUP 001`), the
mirror at d 0 p 0 q 2 `tmk -` `mirror`, the small copy at d 1 p 2 q 8
tmk 1.00, the cut at d 15 p 20 q 102 tmk 0.98 `trim`; the unrelated clips
out. `purge . vid`: 1 `binned`, 3 `binned~`, through the OS trash;
`--undo`: 4 restored, file list identical. Under a pseudo-terminal the
FRAMES line redrew in place and closed with `6 videos hashed   1,740
frames at 15 fps   4.5 s`.

## Scale — the 208-clip corpus as a folder

`find . vid` (821 MB): FRAMES 129 s (one ffmpeg process at a time on this
2-core box — `concurrency()` is ⌈cores/2⌉ clamped to 1–4), MATCH 3.0 s
for 21,528 pairs, **16 video groups = the 16 source families, none
mixed**, 11–13 members each (crop and letterbox edits are the ones
missing), 192 of 208 clips grouped.

## Not verified here

- More than one ffmpeg process at a time (this box runs one); the cap of
  four with `-threads 2` each is the shipped guess for a desktop.
- Real footage: the corpus is synthetic. The hard negatives (two zooms into
  the same fractal) model "same generator, different parameters" — burst
  clips, multiple takes — which is the class a real library will test.
- Windows execution: `ffmpeg.exe` through `std::process::Command` with a
  piped stdout is the same code path; not run.
- Files over ~2 hours (four minutes of ffmpeg each on this box; memory per
  descriptor is 256 KB regardless of length).

---

# VERIFICATION — Phase 4 (deep tier), run on 2026-11-07

Same environment. The model ran through `tract` 0.23 on two cores.

## Suite

`cargo test --release`, without the model: 65 unit + 16 end-to-end pass
(ignored: the two corpus harnesses, the video corpus, the bin round trip,
the two fixture writers). With `DEDUPE_MODEL` set: the same 65 + 16 pass,
the deep end-to-end test now running instead of skipping; one existing
near-pass test needed a model-aware assertion, since the procedural
`forest.png` is listed by the deep tier (as `deep`, never `near`) when the
model is present. `cargo clippy --all-targets --release` clean after one
redundant-closure fix; `cargo fmt` clean. With `--no-default-features`:
63 unit + 16 end-to-end pass (the two deep unit tests are compiled out).

Windows, type-checked only (`cargo check --target x86_64-pc-windows-msvc
--all-targets --features blake3/pure`): passes without the deep feature,
and with it when `clang` stands in as the cross C compiler for
`tract-linalg`'s assembly kernels (`CC_x86_64_pc_windows_msvc=clang
AR_x86_64_pc_windows_msvc=llvm-lib`). Not linked, not run.

## Provenance checks

- `sscd_disc_mixup.onnx` from Hugging Face `nutszebra/sscd_onnx`:
  98,156,202 bytes, SHA-256
  `e65e2a5b9a5e44a0b0f011bddaa6f28621021f6120ca7ce8b526c19f2f6156f8` —
  matches the repository's own `sscd.json`.
- Meta's `sscd_disc_mixup.torchscript.pt` downloaded from
  `dl.fbaipublicfiles.com`: SHA-256
  `9f26bd4c848cc19b73d2ae92eea6e04886f61a7b764ceb7a13aeee62e6a6db56` —
  matches the `sourceSha256` the export declares. Output agreement between
  the two was not checked (no PyTorch here).

## Unit coverage added

`skew_320` preprocessing shape and normalisation values; with the model
installed: self-cosine 1.0, a 4× downscale at 0.97, a 30 % crop at 0.79,
a structurally different picture at 0.51 — against `DEEP_MIN` 0.70.

## Corpus — `DEDUPE_MODEL=… cargo test --release --test corpus -- --ignored measure_deep --nocapture`

Run twice (the second time after the harness output of the first was lost;
the table below is the second run's output, copied by a parser, not
retyped). Every statistic recorded from the first run matches the second
to three decimals; the first took 326 s (370 ms per embedding), the second
343 s with an e2e test running beside it.

| perturbation | min | p5 | median | recall at 0.70 |
|---|---|---|---|---|
| scale 50% | 0.907 | 0.939 | 0.987 | 100.0 % |
| scale 25% | 0.764 | 0.886 | 0.955 | 100.0 % |
| upscale 150% | 0.967 | 0.993 | 0.998 | 100.0 % |
| jpeg q90 | 0.848 | 0.862 | 0.933 | 100.0 % |
| jpeg q60 | 0.656 | 0.739 | 0.841 | 95.0 % |
| jpeg q30 | 0.648 | 0.655 | 0.750 | 77.5 % |
| crop 90% | 0.838 | 0.852 | 0.889 | 100.0 % |
| crop 80% | 0.712 | 0.746 | 0.787 | 100.0 % |
| crop 60% | 0.362 | 0.430 | 0.592 | 2.5 % |
| rotate 2° | 0.748 | 0.763 | 0.836 | 100.0 % |
| rotate 5° | 0.666 | 0.691 | 0.751 | 90.0 % |
| rotate 90° | 0.619 | 0.638 | 0.712 | 77.5 % |
| mirror | 0.909 | 0.911 | 0.940 | 100.0 % |
| bright +20% | 0.938 | 0.944 | 0.977 | 100.0 % |
| bright -20% | 0.936 | 0.959 | 0.971 | 100.0 % |
| gamma 1.5 | 0.886 | 0.925 | 0.956 | 100.0 % |
| watermark | 0.844 | 0.872 | 0.926 | 100.0 % |
| blur | 0.831 | 0.905 | 0.983 | 100.0 % |
| noise ±10 | 0.666 | 0.673 | 0.806 | 92.5 % |
| letterbox | 0.662 | 0.694 | 0.768 | 92.5 % |
| squeeze 75% | 0.972 | 0.996 | 0.998 | 100.0 % |

Negatives, 33,540 pairs: **median 0.425, p99 0.582, p99.9 0.629, max 0.669.**
Sweep (false positives | recall over all perturbations | over crops and
rotations): 0.45 → 12,368 | 99.5 % | 98.0 %; 0.50 → 4,892 | 99.0 % | 96.0 %; 0.55 → 1,151 | 98.3 % | 93.0 %; 0.60 → 127 | 97.3 % | 88.5 %; 0.65 → 4 | 95.2 % | 82.5 %; **0.70 → 0 | 91.8 % | 78.5 %**; 0.75 → 0 | 82.9 % | 68.0 %; 0.80 → 0 | 70.7 % | 45.0 %.

Shipped `DEEP_MIN = 0.70`.

Where the hash pair stops and the model continues: dHash∧pHash caught
70.0 % of the 10 % crops, 2.7 % of the 20 % crops and 62.0 % of the
letterboxes (Phase 2 table); SSCD at 0.70 catches 100 %, 100 % and
92.5 %. Neither handles the 40 % crop (0 % and 2.5 %): at that crop less
than half of the picture's area remains, and the descriptor's median falls
to 0.59.

## Fixture — where the limit showed

On the photo fixture (a different procedural generator, smoother), the
unrelated `forest.png` scored **0.76** against `beach.png`: above the
threshold. The letterbox scored 0.78 (a true catch the hash pair missed),
the 95 % crop 0.82, the 50 % downscale 0.59 (a true copy below the
threshold; the hash pair has it). Both deep-only members printed with the
`deep` verb; `milk` left them in place; `purge -n` listed neither among
the `would` lines. That false positive is the measured basis for the
list-never-act rule.

Before the grouping change the same run had made the letterbox (more
pixels) the keeper and had pulled `forest.png` into the family through a
deep edge in the union–find. Degree-first keeper selection and star-shaped
deep attachment removed both; the no-model run is unchanged.

## Scale and size

Binary: 25.6 MB with the tier (default), 2.9 MB with
`--no-default-features`. One embedding 307 ms on two cores (tract,
single-threaded per image, parallel across images). DECODE for the
10-image fixture: 2.2 s with the model, 0.1 s without.

## Not verified here

- Output agreement of the ONNX export with Meta's TorchScript model.
- Behaviour on real photographs: the negative distribution above is for
  procedural textures and is expected to sit lower on photographs; the
  harness exists to measure that on a real library before the tier is
  ever allowed to act.
- Windows execution (type-checked only, see Suite); on a real Windows
  build `tract-linalg` assembles its kernels with the MSVC toolchain's
  `ml64`, a path this environment cannot exercise.

---

# VERIFICATION — Phase 5 (polish), run on 2026-11-08

Same environment (two cores, Linux, `ffmpeg` present, the model file
available through `DEDUPE_MODEL` for the deep cases). Windows: type-checked
as before, not executed.

## The audit

A fixture tree of 37 files assembled from the earlier fixtures: the exact
set (IMG_0041 ×4, two `a.mp4`, `notes.txt` + copy, the head/mid decoys,
two empty files, a PNG pair), the photo set (beach and its mirror, export,
copy, 5 % crop, small, rotated small, letterbox, forest), the video set
(`a.mp4` and its copy, mirror, small, 4-second trim, two unrelated clips),
an empty folder, a folder with one file, and a stale `_milked\backup\
IMG_0041.jpg` to force a rename. 51 cases — every command × token × flag
that the grammar admits, the error paths, the undo sequences, five menu
drives — each run under a pseudo-terminal at 80 and/or 120 columns (100
for the rest) and once piped; 56 terminal captures and 15 piped ones, all
kept and read.

Before the fixes, 36 of the 56 terminal captures held at least one line
wider than the terminal (measured in display cells after stripping colour
codes). After them, none at 80, 100 or 120 columns; the help page at 60
columns has seven lines over, and its claim was changed from 60 to 80.

## What the reading found, and what was done

| seen | fix |
|---|---|
| `next` lines and action footers carried the full absolute root again (`move duplicates into /tmp/…/audit_root/_milked`), 130+ cells | the typed form: `.\_milked` or `<PATH>\_milked` |
| DECODE / FRAMES stage lines wrapped at 80 columns; the in-place redraw then left debris | a stage line drops its rightmost segments to fit |
| NEAR member lines overflowed at 80 (tail widths were estimated); VIDEO lines overflowed below ~95 | path column sized from the widest real tail; VIDEO drops codec, then dimensions, before the path falls under 18 cells |
| NEAR / VIDEO headers said `6 similar` for keeper + 5 while the totals said 5 | headers count members beside the keeper |
| totals lines (policy + thresholds) wrapped at 80 | policy part moves to an indented second line |
| `near` / `video` hint verbs of different widths left the commands unaligned | fixed verb column |
| after `find -q`, the `next` lines suggested `milk . --quick-search` | follow-ups are the full form |
| `purge -q -n` listed `decoys/mid_differs.bin` as `would` — a size+sample match whose middle byte differs, with no byte check | quick purge verifies bytes; the line now reads `kept  bytes differ from keeper` and the total dropped from 9 to 8 |
| `purge . img` printed `FRAMES 0 videos hashed`, `purge . vid` printed `DECODE 0 images hashed` | each pass runs only when the kind can feed it |
| `find img` in a folder without images printed SIZE/SAMPLE/HASH/DECODE lines of zeros; one file printed `1 files` | empty scan: SCAN and totals only; nothing sharing a size: stage lines stop at SIZE; plurals |
| `no duplicates` printed above a near group | `no exact duplicates` when near or video groups exist |
| action-line details (`d 0 p 0 q 8 mirror`) ragged when right-aligned | same fixed-width layout as the NEAR/VIDEO lines |
| `img vid` together silently kept the last | error: `one kind at a time` |
| `q` did nothing in the menu; a menu-launched command carried the full cwd path, so its `next` lines were 200+ cells | `q` quits from the choice steps; the folder is omitted when it is the cwd |
| doctor: `1 files`; paths wrapped at 80; fix line for the model path wrapped | plural; details and fix lines shortened in the middle |
| the model was found through `DEDUPE_JOURNAL_DIR`'s sibling `models` folder (how the audit's own layout turned the deep tier on by accident) | the model path derives from the app data folder only; `DEDUPE_MODEL` overrides |
| `NO_COLOR=1` also switched to ASCII glyphs | it only removes colour |
| `tmk   -  ` (9 cells) beside `tmk 0.98` (8) | `tmk    -` |

Read from the code rather than the screens: Ctrl-C mid-change lost the
journal (written only at the end); Windows `canonicalize` would have shown
`\\?\C:\…` in the header, the `next` lines and the journal; `_milked`
matched case-sensitively on a case-insensitive file system; dates were
UTC. All four changed (CLI.md §1, §3, §5).

## Ctrl-C

3,000 duplicate pairs (`a/fNNNN.bin` = `b/fNNNN.bin`, 512–1024 bytes);
`dedupe milk .` started, SIGINT sent after the first `moved` line:

- exit 130; `2,826 moved … journal 20261108-063254`, then
  `interrupted after 2,826 moves; the journal covers them, dedupe --undo
  reverses them`
- the journal file holds 2,826 entries, `consumed: false`, no `.tmp` left
  behind; `_milked` holds 2,826 files
- `dedupe --undo`: `2,826 restored`; `a/` and `b/` back to 3,000 each,
  `_milked` gone

SIGINT during a `find` scan ends the process as before (no change had
started). The same sequence is an end-to-end test on Unix
(`interrupted_milk_leaves_a_journal_undo_can_use`).

## Dates

The build machine runs in PST. Fixture files stamped at UTC midnight
(`2019-06-12 00:00Z`) now print `2019-06-11`, the local calendar date, as
Explorer would show them on that machine. The journal id stays UTC.

## Suite

`cargo test --release`: 68 unit tests (3 new: ANSI-aware width, segment
fitting and counting, the verbatim-prefix strip; the journal round trip
now also covers the writer, `keep_remaining` and the unreadable count) and
22 end-to-end tests (6 new: quick purge verifies bytes, one kind at a
time, passes by kind, empty-scan stage lines, the undo-elsewhere hint,
the Unix Ctrl-C sequence). All pass without the model and with
`DEDUPE_MODEL` set; with `--no-default-features` 66 + 22 pass. `cargo
clippy --all-targets --release` clean, `cargo fmt` clean.

Windows, type-checked only (`cargo check --target x86_64-pc-windows-msvc
--all-targets --features blake3/pure`): passes with and without the deep
feature, so `ctrlc` (SetConsoleCtrlHandler) and `time`'s local offset
compile for that target. Not linked, not run.

Binary: 25,697,216 bytes with the deep tier (was 25,618,368; `ctrlc` and
`time` add 79 KB), 3,018,152 bytes without.

## Not verified here

- Windows execution: the `\\?\` strip, the case-insensitive `_milked`
  match, the console Ctrl-C handler and the local-date offset are all
  type-checked, none observed on a Windows console. An attempt to place
  the repository on the linked Windows machine for a real run was blocked
  by the session's folder-access policy.
- The deep tier's cost on Windows (tract's `ml64`-assembled kernels).

---

# VERIFICATION — Phase 6 (real photographs, the fit, the moment), run on 2026-11-08

Same environment. Real pictures: the 98 JPEG/PNG files and 4 AVI clips
in OpenCV's `samples/data` (sparse checkout of `opencv/opencv` at
master), 9.94 MB of images — photographs, renders, scans, diagrams and
line art, with known relations inside the set: two stereo pairs
(`aloeL/R`, `left/right`), a chessboard photographed from 26 positions
(`left01–14`, `right01–14`), consecutive frames (`basketball1/2`,
`rubberwhale1/2`), a viewpoint pair (`graf1/3`), an aerial pair
(`aero1/3`), an exposure pair (`leuvenA/B`), a mirrored render
(`Blender_Suzanne1/2`) and `lena.jpg` beside `lena_tmpl.jpg`. Real
videos: `vtest.avi` (80 s), `Megamind.avi` and its shorter `_bugy`
variant, `tree.avi`, plus three re-encodes made here with ffmpeg.

## 1. The shipped rule on the real pictures (before this phase's changes)

`DEDUPE_REAL_DIR=… cargo test --release --test real_corpus -- --ignored measure_real --nocapture`

All 4,753 pairs of distinct files, the closest by d+p:

| d | p | q | pair | shipped rule |
|---|---|---|---|---|
| 1 | 0 | 24 | Blender_Suzanne1 ~ Blender_Suzanne2 | group, PDQ confirms |
| 1 | 0 | 8 | rubberwhale1 ~ rubberwhale2 | group, PDQ confirms |
| 4 | 2 | 22 | basketball1 ~ basketball2 | group, PDQ confirms |
| 3 | 12 | 44 | lena ~ lena_tmpl | group, q over |
| 7 | 8 | 80 | opencv-logo-white ~ opencv-logo | group, q over |
| 13 | 6 | 64 | left11 ~ left14 | — |
| 10 | 12 | 76 | **right01 ~ right06** | **group, q over** |
| 10 | 12 | 102 | **right01 ~ right09** | **group, q over** |
| 14 | 8 | 110 | right05 ~ right12 | — |

7 pairs grouped; PDQ confirms 3 — the three that are the same picture.
The two bold rows are the same chessboard photographed from three
positions: distinct photographs the two hashes grouped at exactly their
thresholds. Before this phase `milk` would have moved `right06.jpg` and
`right09.jpg` into `_milked`. The d+p histogram shows where the real
world sits: 11 pairs at ≤ 24, 8 at 25–28, 23 at 29–32, 28 at 33–36 —
the calibration series fills the band just outside the rule.

Derivatives of the first 60 real sources (21 perturbations each) against
the shipped rule, and negatives (each variant against every other
source, 122,220 pairs):

| perturbation | d max | p max | q max | d∧p | d∧p∧q |
|---|---|---|---|---|---|
| scale 50 % | 2 | 18 | 92 | 98.3 % | 98.3 % |
| scale 25 % | 5 | 14 | 108 | 100 % | 98.3 % |
| upscale 150 % | 2 | 14 | 80 | 100 % | 98.3 % |
| JPEG q90 / q60 / q30 | 2 / 4 / 5 | 22 / 24 / 28 | 92 / 138 / 150 | 98.3 % | 98.3 % |
| crop 90 % | 34 | 22 | 138 | 35.0 % | 0 % |
| crop 80 % / 60 % | 32 / 35 | 29 / 34 | 150 / 140 | 0 % | 0 % |
| rotate 2° | 20 | 24 | 118 | 91.7 % | 30.0 % |
| rotate 5° | 22 | 24 | 120 | 43.3 % | 0 % |
| rotate 90°, mirror | 0 | 0 | 30 / 26 | 100 % | 100 % |
| bright ±20 %, gamma 1.5 | 5 / 1 / 5 | 20 / 10 / 14 | 118 / 50 / 64 | 98.3–100 % | 98.3 % |
| watermark | 2 | 32 | 128 | 95.0 % | 53.3 % |
| blur, noise ±10 | 2 / 20 | 14 / 24 | 54 / 128 | 100 / 96.7 % | 98.3 / 96.7 % |
| letterbox | 18 | 24 | 122 | 40.0 % | 0 % |
| squeeze 75 % | 1 | 10 | 64 | 100 % | 98.3 % |

The one source behind every 98.3 % is `gradient.png`, a synthetic ramp:
its hashes and PDQ change under any resampling. Real photographs behave
as the procedural corpus predicted for rescale, re-encode, mirror and
rotation, and worse for crops (35 % against 70 % at 10 %) and letterbox
(40 % against 62 %). Negatives: of 122,220 variant-vs-other-source
pairs the rule grouped 107, PDQ confirmed 58, and every one of them
belongs to five source pairs — three that are the same picture
(Suzanne pair, basketball frames, lena pair) and two chessboard poses
(`left11~left14`, `left12~left14`), which PDQ refused.

## 2. The geometric fit

`measure_geometry` (procedural, 60 × 21 positives, 5,588 negative source
pairs) and `measure_geometry_real` (60 × 21 positives from the real
sources, all 4,753 negative pairs). Fit = best band-passed NCC over the
search in RESEARCH §2.7.

Positives (fit minimum / 5th percentile / median, recall at 0.92):

| perturbation | procedural min / p5 / median | recall | real min / p5 / median | recall |
|---|---|---|---|---|
| rescale 50 %, 25 %, 150 % | 1.000 / 1.000 / 1.000 | 100 % | 0.575 / 0.999 / 1.000 | 98.3 % |
| JPEG q90 / q60 / q30 | 0.998 / 0.999 / 1.000 and below | 100 % | 0.445 / 1.000 / 1.000; 0.119 / 0.999 / 1.000; 0.170 / 0.999 / 1.000 | 98.3 % |
| crop 90 % | 0.956 / 0.986 / 0.993 | 100 % | 0.207 / 0.963 / 0.988 | 96.7 % |
| crop 80 % | 0.927 / 0.976 / 0.985 | 100 % | 0.158 / 0.929 / 0.979 | 95.0 % |
| crop 60 % | 0.935 / 0.975 / 0.991 | 100 % | 0.385 / 0.861 / 0.980 | 93.3 % |
| rotate 2° | 0.999 / 0.999 / 0.999 | 100 % | 0.672 / 0.993 / 0.999 | 98.3 % |
| rotate 5° | 0.945 / 0.959 / 0.975 | 100 % | 0.792 / 0.921 / 0.969 | 95.0 % |
| rotate 90°, mirror | 0.40 / 0.48 / 0.61; 0.31 / 0.33 / 0.48 | 0 % | 0.12 / 0.30 / 0.50; 0.08 / 0.27 / 0.40 | 0 / 1.7 % |
| brightness ±20 %, gamma 1.5 | ≥ 0.941 | 100 % | 0.220 / 0.986 / 0.998 and better | 98.3 % |
| watermark | 0.661 / 0.721 / 0.888 | 28.3 % | 0.506 / 0.927 / 0.984 | 95.0 % |
| blur, noise ±10 | ≥ 0.999 | 100 % | ≥ 0.820 / 0.997 / 1.000; 0.231 / 1.000 / 1.000 | 98.3 % |
| letterbox | 0.918 / 0.935 / 0.983 | 98.3 % | 0.218 / 0.950 / 0.979 | 95.0 % |
| squeeze 75 % | 0.999 / 1.000 / 1.000 | 100 % | 0.216 / 0.995 / 1.000 | 98.3 % |

Every real-set minimum under 0.6 and every "98.3 %" is `gradient.png`,
whose band-passed thumbnail is nothing (a box filter maps a ramp onto
itself, so the narrow-minus-wide difference is zero); the fit cannot
confirm it and the hashes miss it too. Mirror and 90° rotation are not
searched by the fit; the hashes own them and PDQ confirms them. The
procedural watermark (a bright 18 % × 8 % box with strokes) costs 0.3 of
correlation on value-noise textures, 0.07 on photographs.

Negatives over every pair of distinct files — procedural 5,588 pairs:
max 0.929, p99.9 0.881, p99 0.800, median 0.552; real 4,753 pairs: max
0.995, p99.9 0.913, p99 0.779, median 0.410. Sweep (false positives |
recall over all perturbations | over crops, rotations and letterbox):

| T | procedural | real |
|---|---|---|
| 0.85 | 18 \| 88.7 % \| 100 % | 14 \| 88.7 % \| 96.7 % |
| 0.88 | 6 \| 88.3 % \| 100 % | 9 \| 88.4 % \| 96.1 % |
| 0.90 | 3 \| 87.7 % \| 100 % | 7 \| 88.3 % \| 96.1 % |
| **0.92** | **2 \| 87.0 % \| 99.7 %** | **2 \| 88.1 % \| 95.6 %** |
| 0.95 | 0 \| 85.3 % \| 97.5 % | 1 \| 86.6 % \| 91.9 % |

The two real pairs at or above 0.92 are the same picture both times
(`rubberwhale1~2` 0.995, `basketball1~2` 0.931, consecutive frames); the
highest unrelated real pair is `digits.png~notes.png` at 0.918. The two
procedural pairs are value-noise textures the hashes never grouped. The
fit is consulted only for pairs a hash or the descriptor admitted, and
among those no non-derivative reached 0.62 in either corpus.

Shipped `FIT_MIN = 0.92`.

Named pairs in the real set (`geometry_real_named`): among the pairs
the hashes grouped, `rubberwhale1~2` fit 1.00, `basketball1~2` 0.93,
`lena~lena_tmpl` 0.80, logo pair 0.68, `right01~right06` 0.45,
`right01~right09` 0.31, the mirrored Suzanne pair 0.32 (the fit does not
search mirrors; PDQ confirms that pair). The best-fitting pairs of
distinct files that are not the same picture are text and line-art
images (`digits.png~notes.png` 0.918, `imageTextN.png~pic2.png` 0.914):
band-passed strokes correlate across unrelated documents. None of these
is a hash or deep candidate, so the fit is never asked about them.

## 3. The binary on the real set

`find img`, 98 files, DECODE 1.0 s, MATCH 0.1 s with 4 fits:

```
 --------------------------------------------------------------------------------------------------------------------------------
 SCAN   ==============================  98 files   9.94 MB   0.0 s
 SIZE   ==============================  0 share a size
 DECODE ==============================  98 images hashed   1.0 s
 MATCH  ==============================  7 similar pairs   13 images grouped   4 fits, 0 confirmed   0.1 s

 NEAR 001   1 similar   353 KB reclaimable
   keep  rubberwhale1.png                                                        584x388      352 KB   2026-11-07
   near  rubberwhale2.png                                                        584x388      353 KB   d  1  p  0  q   8
 NEAR 002   1 similar   130 KB reclaimable
   keep  basketball1.png                                                         640x480      130 KB   2026-11-07
   near  basketball2.png                                                         640x480      130 KB   d  4  p  2  q  22
 NEAR 003   1 similar   24.8 KB reclaimable
   keep  Blender_Suzanne1.jpg                                                    640x480      24.8 KB  2026-11-07
   near  Blender_Suzanne2.jpg                                                    640x480      24.8 KB  d  1  p  0  q  24  mirror
 NEAR 004   1 similar   0 B reclaimable
   keep  lena.jpg                                                                512x512      89.7 KB  2026-11-07
   near  lena_tmpl.jpg                                                           512x512      77.6 KB  d  3  p 12  q  44  fit 0.80
 NEAR 005   1 similar   0 B reclaimable
   keep  opencv-logo.png                                                         600x794      39.2 KB  2026-11-07
   near  opencv-logo-white.png                                                   180x238      9.53 KB  d  7  p  8  q  80  fit 0.68
 NEAR 006   2 similar   0 B reclaimable
   keep  right01.jpg                                                             640x480      26.4 KB  2026-11-07
   near  right06.jpg                                                             640x480      27.2 KB  d 10  p 12  q  76  fit 0.45
   near  right09.jpg                                                             640x480      25.3 KB  d 10  p 12  q 102  fit 0.31
 --------------------------------------------------------------------------------------------------------------------------------
 no exact duplicates   98 files   9.94 MB checked
 6 near groups   7 similar   507 KB reclaimable
   keep: shared moment, most matched, then most pixels   dHash <=10  pHash <=14  PDQ <=31 or fit >=0.92 to act
 next   dedupe milk img   move duplicates into ./_milked
        dedupe purge img  send duplicates to the Recycle Bin
```

`milk img -n`:

```
 would~    rubberwhale2.png                                                                                     d  1  p  0  q   8
 would~    basketball2.png                                                                                      d  4  p  2  q  22
 would~    Blender_Suzanne2.jpg                                                                         d  1  p  0  q  24  mirror
 kept~     lena_tmpl.jpg                                                                    PDQ q 44 over 31; fit 0.80 under 0.92
 kept~     opencv-logo-white.png                                                            PDQ q 80 over 31; fit 0.68 under 0.92
 kept~     right06.jpg                                                                      PDQ q 76 over 31; fit 0.45 under 0.92
 kept~     right09.jpg                                                                     PDQ q 102 over 31; fit 0.31 under 0.92
 3 would move   507 KB into ./_milked
 4 kept~: similar on two hashes, confirmed by nothing else; shown, left in place
```

With the model (`DEDUPE_MODEL`), the descriptor admitted 11 more pairs
on the real set, every one a sibling capture: the stereo pair
`aloeL~aloeR` (deep 0.91), chessboard poses at 0.72–0.88, and the ELA
forensic pair `ela_original~ela_modified` (0.78), which is a real edit
and crop of the same photograph. The fit refused all 15 pairs it was
asked about (stereo 0.74, ELA 0.77, poses 0.28–0.62), so nothing was
acted on. Two readings: the deep threshold of 0.70, set on procedural
pictures, admits same-scene photographs freely on real ones — the
list-never-act rule of Phase 4 is what keeps that harmless; and the ELA
pair is a true derivative that neither PDQ (q 122) nor the fit (0.77:
the edit, plus a crop — 902×770 against 897×708) confirms — a miss,
recorded.

Real video, `find vid` on the seven clips (FRAMES 4.7 s):

```
 VIDEO 001   2 similar   2.72 MB reclaimable
   keep  vtest.avi                                           768x576      1:20    msmpe  7.75 MB  2026-11-07
   near  vtest_small.mp4                                     384x288      1:20    h264   621 KB   d  0 p  2  q   2  tmk 1.00
   near  vtest_trim.mp4                                      768x576      0:40    h264   2.12 MB  d  6 p 12  q  58  tmk 0.84  trim
 VIDEO 002   2 similar   1.22 MB reclaimable
   keep  Megamind.avi                                        720x528      0:11    mpeg4  1.13 MB  2026-11-07
   near  megamind_x264.mp4                                   720x528      0:11    h264   408 KB   d  0 p  2  q   4  tmk 1.00
   near  Megamind_bugy.avi                                   720x528      0:09    mpeg4  841 KB   d  2 p  2  q  12  tmk 1.00  trim
 2 video groups   4 similar   3.94 MB reclaimable
   keep: longest, then most pixels   tmk >=0.8 groups, >=0.9 bins, or frames q <=31
```

## 4. The fixture

`find . img` without the model: the 5 % crop, PDQ 32 (one bit over the
confirmation threshold through Phase 5), now reads
`d 10  p  6  q  32  fit 1.00  crop 95%` and both commands act on it. With
the model: the letterboxed copy (`d 11  p 20  q  92  deep 0.78  fit 1.00`)
is `near` and acts; `forest.png` (`deep 0.76  fit 0.51`) stays `deep`,
listed. A copy with a caption bar over its lower 15 % (d 5, p 14, q 36,
fit 0.78 — chosen with `examples/fit_diag.rs`) is grouped by the hashes
and printed `kept~` by `milk`, which moves the confirmed members and
leaves it. Capture moments: three JPEGs of one exposure stamped
`2021:03:04 05:06:07` (original, re-export, one with EXIF stripped) and
the next frame stamped `05:06:08` — the frame prints as
`kin … taken 2021-03-04 05:06:08`, the keeper is the original, the
stripped copy is an ordinary `near`, `milk` moves the re-export and the
stripped copy and leaves the frame.

## Suite

`cargo test --release`: 78 unit tests (10 new: six for the fit —
identity, crop located at its scale, letterbox found the other way
round, small rotation recovered, different pictures do not fit,
brightness invariance — and four for capture moments, one of which
builds a JPEG with an EXIF segment and reads it back) and 24 end-to-end
tests (3 changed for the acting rule and the fit, 2 new: an edit the
hashes group but nothing confirms stays `kept~` under `milk`; four
JPEGs with synthesised capture moments give one `kin` that neither
command touches while the same-moment re-export and the EXIF-stripped
copy move). All pass without the model and with `DEDUPE_MODEL` set; with
`--no-default-features` 76 + 24 pass. `cargo clippy --all-targets
--release` clean, `cargo fmt` clean. Windows type-checks pass with and
without the deep feature (`kamadak-exif` compiles for the target). The
Phase 5 audit re-run on this build: 56 terminal captures, no line over
the width at 80, 100 or 120 columns (NEAR blocks drop their dimensions
column at 80 when a `fit` tail is present).

Binary: 25,903,792 bytes with the deep tier, 3,225,576 without
(`kamadak-exif` and the fit add about 200 KB).

## Not verified here

- The capture-moment rule on a camera roll: the real set carries no
  EXIF; verified on synthesised EXIF only.
- The fit on crops under half a side, rotations past 6°, and on real
  letterboxes with non-black bars (procedural letterbox measured only).
- Windows execution (type-checked, with and without the deep feature).
