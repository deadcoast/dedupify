# TODO — Phase 10: formats and filesystems

```
[IDENTIFIED + PLAN] -> [SHELVED] -> [TODO]
```

Milestone: `PLAN.md` § Phase 10 · items: `SHELVED.md` § Phase 10 · source:
`IDENTIFIED.md` (IDs below). Build order top to bottom. Previous
milestone: Phase 9, delivered 2026-11-09 (`SHELVED.md` § Phase 9 `[done]`).

- [x] **TECH-01** `decode.rs` owning every decoder
- [x] **NF-01** HEIC / HEIF / AVIF through ffmpeg, EXIF from the container
- [x] **NF-02** RAW stills through their embedded previews; RAW + JPEG pairs
- [x] **CORE-05** cross-volume `milk` (copy, verify, delete; journaled)
- [x] **SC-07** typed action errors (the workspace offers retry / fallback)
  — retry driven; the bin-refused fallback written, not driven (VERIFICATION § Phase 10)
- [x] **QOL-06** `--skip` exclusions, remembered in the app dir
- [x] **QOL-08** `--min` size floor
- [x] **QOL-09** doctor: HEIC decode, `_milked` volume, model SHA-256
- [ ] **QOL-10** Windows console colour verified (screenshot in VERIFICATION)
  — built: the launcher photographs its console after a live doctor; the first Windows
  run ran the launcher's previous copy (no picture); awaits the next run
- [ ] **TECH-07** Ctrl-C e2e on Windows
  — built: CTRL_BREAK through `GenerateConsoleCtrlEvent`; passed in the first Windows run,
  where a skip could not be told from a pass; with a console it now fails rather than
  skip; awaits the next run
- [ ] **SC-08** UTF-8 Windows logs, no log churn in commits
  — built: UTF-8 writer, `runs/win/logs/` removed, `log/` ignored; the first Windows run's
  logs are UTF-16 (the previous launcher wrote them); awaits the next run

Done when: every box ticked, the suite green on Linux and in
`runs/win/RUN_VERIFY.cmd`, `VERIFICATION` has the Phase 10 section, and
`PLAN.md` marks Phase 10 delivered.

---

## Next

When this list is complete: mark `SHELVED.md` § Phase 10 `[done]`, then
rewrite this file from the next `[shelved]` section of `SHELVED.md`
(Phase 11 — matching depth). If SHELVED has no `[shelved]` section left,
run the IDENTIFIED workflow instead.
