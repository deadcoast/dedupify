# SHELVED — actionable items, ordered by milestone

```txt
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

## Phase 16 — the workspace at size   `[done]`

1. **SC-10** unit tests for the width logic, 40–200 columns
2. **QOL-11** narrow terminals: no line past the edge down to 40
3. **SC-12** split `workspace/state.rs` (state and keys / screen)
4. **QOL-12** sort and filter in the workspace
5. **NF-10** prefer a folder (`f`)
6. **TECH-12** a resize during a sitting, driven
7. **TECH-13** the launcher photographs the workspace and `v`
8. **QOL-13** `v` sized right in Windows Terminal (from TECH-13's picture)
9. **NF-09** the model fetched by the doctor

## Phase 17 — the fold: the output redesigned   `[done]`

The owner's direction of 2026-11-13, not an IDENTIFIED item; IDs here:

1. **OUT-03** a filter or a sort shows its own groups (the Windows
   run's `view.png`: `/castle` drew GROUP 001's lines over its rows)
2. **QOL-13** Windows Terminal known without `WT_SESSION` (its window
   class), and the sixel cell it lays pictures out on, 10 × 20
3. **OUT-01** the fold: the default output, animated, one line per
   pass and the yield; the full output behind `-v` / `--verbose`
4. **OUT-02** `RUN_VERIFY`: one live line per workflow, folded to its
   mark and result; each result checked, the pictures by what the
   console shows; the update step's words plain
5. **OUT-05** the workspace in the fold: sections by kind, a line per
   group with its bar, the cursor's group unfolded on a tree, the
   cursor's line lit, the gauge under the body; files fading in, the
   gauge rolling, the pointer breathing
6. **OUT-06** the workspace in two panels: the groups framed on the
   left; on the right the keeper and the file under the cursor as
   pictures (half blocks, 24-bit colour, decoded off the key loop, a
   sheen while coming, risen from the ground on arrival), their
   captions, the file's words, its evidence as meters against the limits

## Phase 18 — the batch pipeline   `[done]`

The owner's direction of 2026-11-14, not an IDENTIFIED item; IDs here:

1. **OUT-07** the workspace's keys where they act: one line under the
   gauge, keycaps; `s` and `/` on the groups' frame, the file's keys on
   the comparison's; every key on the card `?` opens
2. **BATCH-01** `dedupe batch PATH`: the whole tree, copies and
   confirmed lookalikes into `_milked` under one journal
3. **BATCH-02** keepers hold to one side of a pair of folders
4. **BATCH-03** the account by folder, `FROM`: what left each folder,
   how much of it, what it was, where the keepers are; mirrors raised
5. **BATCH-04** the end bar: open `_milked`, undo it all, done
6. **BATCH-05** the menu: two pipelines and a folder
7. **OUT-08** the yield's gauge: a group of one cell in its kind's hue,
   not a stripe

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
