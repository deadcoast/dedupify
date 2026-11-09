# TODO — Phase 10: formats and filesystems

```
[IDENTIFIED + PLAN] -> [SHELVED] -> [TODO]
```

Milestone: `PLAN.md` § Phase 10 · items: `SHELVED.md` § Phase 10 · source:
`IDENTIFIED.md` (IDs below). Build order top to bottom. Previous
milestone: Phase 9, delivered 2026-11-09 (`SHELVED.md` § Phase 9 `[done]`).

- [ ] **TECH-01** `decode.rs` owning every decoder
- [ ] **NF-01** HEIC / HEIF / AVIF through ffmpeg, EXIF from the container
- [ ] **NF-02** RAW stills through their embedded previews; RAW + JPEG pairs
- [ ] **CORE-05** cross-volume `milk` (copy, verify, delete; journaled)
- [ ] **SC-07** typed action errors (the workspace offers retry / fallback)
- [ ] **QOL-06** `--skip` exclusions, remembered in the app dir
- [ ] **QOL-08** `--min` size floor
- [ ] **QOL-09** doctor: HEIC decode, `_milked` volume, model SHA-256
- [ ] **QOL-10** Windows console colour verified (screenshot in VERIFICATION)
- [ ] **TECH-07** Ctrl-C e2e on Windows
- [ ] **SC-08** UTF-8 Windows logs, no log churn in commits

Done when: every box ticked, the suite green on Linux and in
`runs/win/RUN_VERIFY.cmd`, `VERIFICATION` has the Phase 10 section, and
`PLAN.md` marks Phase 10 delivered.

---

## Next

When this list is complete: mark `SHELVED.md` § Phase 10 `[done]`, then
rewrite this file from the next `[shelved]` section of `SHELVED.md`
(Phase 11 — matching depth). If SHELVED has no `[shelved]` section left,
run the IDENTIFIED workflow instead.
