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

## Phase 15 — matching debts   `[done]` 2026-11-12 — see PLAN.md § Phase 15, VERIFICATION.md § Phase 15

1. **CORE-12** PDQ at a fixed working size
2. **CORE-13** the near keeper's degree against the copy marks
3. **CORE-14** dark blurred bars on video
4. **TECH-14** the ONNX export against Meta's TorchScript model, in CI
5. **CORE-17** a copy chained into another picture's group is measured
   against that keeper (found in Phase 14's 100,000-file run)

## Phase 16 — the workspace at size   `[todo]`

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
