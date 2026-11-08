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
