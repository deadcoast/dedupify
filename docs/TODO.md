# TODO — Phase 11: matching depth

```
[IDENTIFIED + PLAN] -> [SHELVED] -> [TODO]
```

Milestone: `PLAN.md` § Phase 11 · items: `SHELVED.md` § Phase 11 · source:
`IDENTIFIED.md` § 4.3–4.5 (IDs below). Build order top to bottom. Previous
milestone: Phase 10, delivered 2026-11-10 (`SHELVED.md` § Phase 10 `[done]`;
its three Windows-only verifications wait in § Phase 12).

- [ ] **CORE-04** deep threshold on real labelled copies
- [ ] **CORE-08** capture-moment rule on a real camera roll (needs NF-01)
- [ ] **CORE-01** robust (trimmed / rank) NCC in the fit
- [ ] **CORE-02** mirror search in the fit
- [ ] **CORE-03** non-black video bars
- [ ] **CORE-06** mid-file sample for files over 1 GB
- [ ] **CORE-07** provenance-aware keeper policy
- [ ] **NF-05** burst / series handling, keep the sharpest
- [ ] **TECH-02** DCT-domain JPEG downscale, re-measured against conformance
- [ ] **QOL-07** ETA on long stages

Done when: every box ticked, the suite green on Linux (with the Windows
type-check and clippy; a Windows run if the phase touches Windows-only
code), every new or moved threshold measured on a corpus named in
`VERIFICATION` § Phase 11, and `PLAN.md` marks Phase 11 delivered.

---

## Next

When this list is complete: mark `SHELVED.md` § Phase 11 `[done]`, then
rewrite this file from the next `[shelved]` section of `SHELVED.md`
(Phase 12 — structure and CI). If SHELVED has no `[shelved]` section left,
run the IDENTIFIED workflow instead.
