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
