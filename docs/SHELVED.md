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

## Phase 10 — formats and filesystems   `[done]` 2026-11-10 — see PLAN.md § Phase 10, VERIFICATION.md § Phase 10

Items 9–11 are built; their Windows run was skipped and they wait in
Phase 12 (below) for their Windows verification.

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

## Phase 11 — matching depth   `[todo]`

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
9. **TECH-07** Ctrl-C e2e on Windows — built in Phase 10; verified by a
   Windows pass with a console present (the next `RUN_VERIFY.cmd`, or the
   TECH-08 job if its runner has a console: without one the test skips
   and proves nothing)
10. **SC-08** UTF-8 Windows logs — built in Phase 10; read from the next
    `runs/win/RUN_VERIFY.cmd`
11. **QOL-10** Windows console colour — built in Phase 10; the screenshot
    from the next `runs/win/RUN_VERIFY.cmd` goes into VERIFICATION

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
