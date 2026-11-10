# TODO — Phase 12: structure and CI

```
[IDENTIFIED + PLAN] -> [SHELVED] -> [TODO]
```

Milestone: `PLAN.md` § Phase 12 · items: `SHELVED.md` § Phase 12 · source:
`IDENTIFIED.md` § 4.2, 4.3, 4.5 (IDs below). Build order top to bottom. Previous
milestone: Phase 11, delivered 2026-11-10 (`SHELVED.md` § Phase 11 `[done]`).

- [ ] **SC-02** split `actions.rs` (scan / plan / act)
- [ ] **SC-03** split `render.rs` by screen
- [ ] **TECH-06** split `video.rs`
- [ ] **SC-05** `GroupView` trait over the three group types
- [ ] **SC-06** `unwrap()` audit
- [ ] **TECH-05** in-tree benchmarks on fixed corpora
- [ ] **TECH-04** ConPTY pseudo-terminal tests on Windows
- [ ] **TECH-08** CI matrix (ubuntu, windows, ffmpeg)
- [ ] **TECH-07** Ctrl-C e2e on Windows — built in Phase 10; verified by a
  Windows pass with a console present
- [ ] **SC-08** UTF-8 Windows logs — built in Phase 10; read from the next
  `runs/win/RUN_VERIFY.cmd`
- [ ] **QOL-10** Windows console colour — built in Phase 10; the screenshot
  from the next `runs/win/RUN_VERIFY.cmd` goes into VERIFICATION

Done when: every box ticked, the suite green on Linux and in the CI
matrix on Windows, the three Phase 10 items verified on Windows with a
console present, `VERIFICATION` § Phase 12 written, and `PLAN.md` marks
Phase 12 delivered.

---

## Next

When this list is complete: mark `SHELVED.md` § Phase 12 `[done]`, then
rewrite this file from the next `[shelved]` section of `SHELVED.md`. If
SHELVED has no `[shelved]` section left (Phase 12 is the last planned
milestone), run the IDENTIFIED workflow instead.
