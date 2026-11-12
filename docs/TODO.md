# TODO — Phase 14: cost at scale

```
[IDENTIFIED + PLAN] -> [SHELVED] -> [TODO]
```

Milestone: `PLAN.md` § Phase 14 · items: `SHELVED.md` § Phase 14 · source:
`IDENTIFIED.md` § 4 (IDs below), review of 2026-11-12 at `f9227ee`. Build
order top to bottom. Previous milestone: Phase 13, delivered 2026-11-12
(`SHELVED.md` § Phase 13 `[done]`).

- [ ] **TECH-10** the bench on real photographs and videos
  (`DEDUPE_PHOTO_DIRS`, `DEDUPE_VIDEO_DIRS`), with the model, peak memory
  per stage, a 100,000-file run; here and on the owner's machine
- [ ] **SC-09** split `near.rs` (relation / bars / group), every line
  accounted for
- [ ] **CORE-15** the fit's search coarse to fine, thumbnails reused;
  every fit score within 0.005, no decision changed
- [ ] **CORE-16** long video's decode cost measured on real 1–2 hour
  files; hardware decoding and skipped non-reference frames tried against
  the conformance and robustness tables
- [ ] **TECH-15** the deep tier's throughput, from TECH-10's profile;
  embeddings bit-identical

Done when: every box ticked, the suite green on Linux and in the CI
matrix on Windows, `VERIFICATION` § Phase 14 written, and `PLAN.md` marks
Phase 14 delivered.

---

## Next

When this list is complete: mark `SHELVED.md` § Phase 14 `[done]`, then
rewrite this file from the next `[shelved]` section of `SHELVED.md`
(Phase 15). When SHELVED has no `[shelved]` section left, run the
IDENTIFIED workflow instead.
