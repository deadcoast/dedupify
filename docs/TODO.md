# TODO — Phase 13: the real disk

```
[IDENTIFIED + PLAN] -> [SHELVED] -> [TODO]
```

Milestone: `PLAN.md` § Phase 13 · items: `SHELVED.md` § Phase 13 · source:
`IDENTIFIED.md` § 4 (IDs below), review of 2026-11-12 at `f9227ee`. Build
order top to bottom. Previous milestone: Phase 12, delivered 2026-11-12
(`SHELVED.md` § Phase 12 `[done]`).

- [ ] **SC-11** doc drift: CLI §2 and RESEARCH §3 on where ffmpeg is
  looked for; RESEARCH §3's concurrency formula
- [ ] **CORE-09** never enter `$RECYCLE.BIN`, `RECYCLER`, `System Volume
  Information`, `.Trash`, `.Trashes`, `.Trash-<uid>` or the freedesktop
  trash; counted on the SCAN line
- [ ] **CORE-10** online-only files (Windows recall/offline attributes,
  macOS dataless) not read; counted; the doctor says how many
- [ ] **CORE-11** a clock on every ffmpeg and ffprobe child; a stalled
  file named `timed out`
- [ ] **TECH-09** the journal's `.tmp` flushed before its rename
- [ ] **TECH-11** the Recycle Bin refusing a file, driven on the Windows
  CI runner

Done when: every box ticked, the suite green on Linux and in the CI
matrix on Windows, `VERIFICATION` § Phase 13 written, and `PLAN.md` marks
Phase 13 delivered.

---

## Next

When this list is complete: mark `SHELVED.md` § Phase 13 `[done]`, then
rewrite this file from the next `[shelved]` section of `SHELVED.md`
(Phase 14). When SHELVED has no `[shelved]` section left, run the
IDENTIFIED workflow instead.
