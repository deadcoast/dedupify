# SHELVED — actionable items, ordered by milestone

```
[IDENTIFIED + PLAN] -> [SHELVED] -> [TODO]
```

Pulled from `IDENTIFIED.md` (items by ID) and ordered under the `PLAN.md`
milestones. The first milestone not yet delivered is the one in
`TODO.md`. When TODO completes, the next section here becomes the new
TODO; when this file is empty, every PLAN milestone is delivered and a new
IDENTIFIED pass runs.

Status tags: `[todo]` in TODO now · `[shelved]` waiting for its
milestone · `[done]` delivered (kept one milestone for reference, then
removed).

Source review: IDENTIFIED 2026-11-09 (`780e3cc`).

---

## Phase 9 — the workspace, deep   `[todo]`

Order inside the milestone is build order: each item relies on the ones
above it.

1. **SC-01** one coherent doc set: decide RESEARCH/VERIFICATION file
   layout, delete the duplicate, fix the 47 stale references
2. **SC-04** stale menu and copy strings
3. **TECH-03** workspace state machine (Browse, Confirm, Acted, Keeper),
   unit-testable without a pseudo-terminal — the base for 4–8
4. **QOL-01** per-group / per-member selection, bar counts follow
5. **QOL-02** several changes per sitting, `u` reverses the last
6. **QOL-04** cursor, scroll and `g` jump over long results
7. **NF-04** `o` reveal the file in Explorer
8. **NF-03** in-terminal preview (sixel / kitty), OS-viewer fallback
9. **QOL-03** remembered last folder and kind on the bare menu
10. **QOL-05** undo history: pick which journal to reverse

## Phase 10 — formats and filesystems   `[shelved]`

1. **TECH-01** `decode.rs` owning every decoder
2. **NF-01** HEIC / HEIF / AVIF through ffmpeg, EXIF from the container
3. **NF-02** RAW stills through their embedded previews; RAW + JPEG pairs
4. **CORE-05** cross-volume `milk` (copy, verify, delete; journaled)
5. **SC-07** typed action errors (the workspace offers retry / fallback)
6. **QOL-06** `--skip` exclusions, remembered in the app dir
7. **QOL-08** `--min` size floor
8. **QOL-09** doctor: HEIC decode, `_milked` volume, model SHA-256
9. **QOL-10** Windows console colour verified (screenshot in VERIFICATION)
10. **TECH-07** Ctrl-C e2e on Windows
11. **SC-08** UTF-8 Windows logs, no log churn in commits

## Phase 11 — matching depth   `[shelved]`

1. **CORE-04** deep threshold on real labelled copies
2. **CORE-08** capture-moment rule on a real camera roll (needs NF-01)
3. **CORE-01** robust (trimmed / rank) NCC in the fit
4. **CORE-02** mirror search in the fit
5. **CORE-03** non-black video bars
6. **CORE-06** mid-file sample for files over 1 GB
7. **CORE-07** provenance-aware keeper policy
8. **NF-05** burst / series handling, keep the sharpest
9. **TECH-02** DCT-domain JPEG downscale, re-measured against conformance
10. **QOL-07** ETA on long stages

## Phase 12 — structure and CI   `[shelved]`

1. **SC-02** split `actions.rs` (scan / plan / act)
2. **SC-03** split `render.rs` by screen
3. **TECH-06** split `video.rs`
4. **SC-05** `GroupView` trait over the three group types
5. **SC-06** `unwrap()` audit
6. **TECH-05** in-tree benchmarks on fixed corpora
7. **TECH-04** ConPTY pseudo-terminal tests on Windows
8. **TECH-08** CI matrix (ubuntu, windows, ffmpeg)

## Not yet placed

Items IDENTIFIED raised that no planned milestone holds; they wait for a
PLAN milestone of their own (or the next IDENTIFIED pass):

- **NF-06** HTML report export
- **NF-07** audio duplicates
- **NF-08** folder-pair mode (`find A B`)
- **NF-09** model fetch with pinned SHA-256

---

## Next

When `TODO.md` is complete: mark its section here `[done]`, take the next
`[shelved]` section into a new `TODO.md`, and mark the PLAN milestone
delivered. When no `[shelved]` section remains: run IDENTIFIED.
