# dedupify — docs

| file | what it holds |
|---|---|
| `PLAN.md` | the one plan: every phase, delivered and planned, and the standing rules |
| `CLI.md` | the resolved grammar, every screen, the workspace, the menu, the safety model |
| `RESEARCH.md` | the mathematics and the papers behind each choice, and what was measured for it |
| `VERIFICATION.md` | what each phase ran and what it showed, phase by phase, including the Windows run |
| `DEEP.md` | the optional learned tier: install, provenance, what it may do |
| `COMPARISON.md` | against czkawka, fclones and the hash libraries: same, different, missing |
| `IDENTIFIED.md` | the latest review of the code against the design, with ID'd improvement lists |
| `SHELVED.md` | the review's items ordered under the planned milestones |
| `TODO.md` | the milestone in progress |

The working chain:

```
[IDENTIFIED + PLAN] -> [SHELVED] -> [TODO]
```

A finished TODO pulls its next milestone from SHELVED; when no milestone
is left shelved, a new IDENTIFIED review runs.
