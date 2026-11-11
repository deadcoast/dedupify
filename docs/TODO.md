# TODO — Phase 12: structure and CI

```
[IDENTIFIED + PLAN] -> [SHELVED] -> [TODO]
```

Milestone: `PLAN.md` § Phase 12 · items: `SHELVED.md` § Phase 12 · source:
`IDENTIFIED.md` § 4.2, 4.3, 4.5 (IDs below). Build order top to bottom. Previous
milestone: Phase 11, delivered 2026-11-10 (`SHELVED.md` § Phase 11 `[done]`).

- [x] **SC-02** split `actions.rs` — `actions/{scan,plan,act,undo,view}.rs`, the
  commands composed in `actions/mod.rs` (undo apart from act **[CLAUDE]**)
- [x] **SC-03** split `render.rs` by screen — `render/{stages,groups,totals,hints}.rs`
- [x] **TECH-06** split `video.rs` — `video/{probe,tmk,bars,group}.rs`
- [x] **SC-05** `GroupView` trait over the three group types — `Found::views`
- [x] **SC-06** `unwrap()` audit — 18 made invariants or handled; clippy
  `unwrap_used` denied outside tests
- [x] **TECH-05** in-tree benchmarks on fixed corpora — `tests/bench.rs`
- [x] **TECH-04** ConPTY pseudo-terminal tests on Windows — `run_tty` on
  Windows; green in CI run 43115899107 after the first run found the
  child drawing into the parent's redirected output
- [x] **TECH-08** CI matrix (ubuntu, windows, ffmpeg) —
  `.github/workflows/ci.yml`; green from run 43115899107
- [x] **TECH-07** Ctrl-C e2e on Windows — verified in CI run 43115899107
  by its own words (`CTRL_BREAK sent to the milk's process group …`)
- [x] **SC-08** UTF-8 Windows logs — verified from the 2026-11-11 run
  (VERIFICATION § Phase 11 on Windows)
- [ ] **QOL-10** Windows console colour — the enablement path runs in a
  test through the console host (`a_terminal_gets_colour`, in CI); the
  screenshot from the next `runs/win/RUN_VERIFY.cmd` goes into
  VERIFICATION

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
