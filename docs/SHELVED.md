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

Source review: IDENTIFIED 2026-11-12 (`f9227ee`).

---

## Phase 12 — structure and CI   `[done]` 2026-11-12 — see PLAN.md § Phase 12, VERIFICATION.md § Phase 12

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

## Phase 13 — the real disk   `[todo]`

1. **SC-11** doc drift: where ffmpeg is looked for; the concurrency formula
2. **CORE-09** never enter the OS trash or system folders
3. **CORE-10** online-only files are not read
4. **CORE-11** a clock on every child process
5. **TECH-09** the journal flushed before it is renamed into place
6. **TECH-11** the Recycle Bin refusing a file, driven on the Windows CI
   runner

## Phase 14 — cost at scale   `[shelved]`

1. **TECH-10** the bench on real files, the model, peak memory, 100,000
   files; here and on the owner's machine
2. **SC-09** split `near.rs` (relation / bars / group)
3. **CORE-15** the fit's search coarse to fine, thumbnails reused
4. **CORE-16** long video's decode cost
5. **TECH-15** the deep tier's throughput

## Phase 15 — matching debts   `[shelved]`

1. **CORE-12** PDQ at a fixed working size
2. **CORE-13** the near keeper's degree against the copy marks
3. **CORE-14** dark blurred bars on video
4. **TECH-14** the ONNX export against Meta's TorchScript model, in CI

## Phase 16 — the workspace at size   `[shelved]`

1. **SC-10** unit tests for the width logic, 40–200 columns
2. **QOL-11** narrow terminals: no line past the edge down to 40
3. **SC-12** split `workspace/state.rs` (state and keys / screen)
4. **QOL-12** sort and filter in the workspace
5. **NF-10** prefer a folder (`f`)
6. **TECH-12** a resize during a sitting, driven
7. **TECH-13** the launcher photographs the workspace and `v`
8. **QOL-13** `v` sized right in Windows Terminal (from TECH-13's picture)
9. **NF-09** the model fetched by the doctor

## Not yet placed

Items IDENTIFIED raised that no planned milestone holds; they wait for a
PLAN milestone of their own (or the next IDENTIFIED pass), for the
reasons IDENTIFIED gives with each:

- **NF-06** HTML report export
- **NF-07** audio duplicates
- **NF-08** folder-pair mode (`find A B`) — NF-10 first
- **NF-11** animated GIF, WebP and APNG in the video pass

---

## Next

When `TODO.md` is complete: mark its section here `[done]`, take the next
`[shelved]` section into a new `TODO.md`, and mark the PLAN milestone
delivered. When no `[shelved]` section remains: run IDENTIFIED.
