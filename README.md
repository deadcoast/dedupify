# dedupe — the Dedupelifier

A duplicate finder for the terminal. One run finds exact duplicates,
near-duplicate images (rescaled, re-encoded, mirrored, rotated, cropped,
letterboxed) and near-duplicate videos (re-encoded, scaled, trimmed,
mirrored, boxed in black bars; `ffmpeg` and `ffprobe` on PATH), says in
words what each one is — `resized 50%`, `cropped 80%`, `another shot,
05:06:08` — and then stays on the result: `m` moves them into `_milked`,
`p` sends them to the Recycle Bin, `k` changes a keeper, `u` reverses the
change. Nothing is moved or binned on one family of evidence: the two
64-bit hashes must be confirmed by PDQ or by the geometric fit, and two
photographs with different capture moments are shown and left alone.
`docs/COMPARISON.md` says what here is the standard pipeline and what is
not.

```
cargo install --path .
```

```
dedupe                      workflow menu
dedupe find [PATH] [.]      list every duplicate group, then decide on it
dedupe milk [PATH] [.]      move duplicates into PATH\_milked
dedupe purge [PATH] [.]     verify bytes, then Recycle Bin
dedupe --undo [PATH]        reverse the last milk or purge
dedupe doctor [PATH]        checks, with a fix line for each
```

`.` includes subfolders; `img` / `vid` restrict the kind; `-n` dry run;
`-q` byte copies only (size and 64 KiB samples, no near pass); `-h` the
help page.

- `docs/PLAN.md` — the phases, what this build contains, what is next
- `docs/RESEARCH.md` — the mathematics and the papers behind each choice
- `docs/CLI.md` — the resolved grammar, output design, menu, safety model
- `docs/VERIFICATION.md` — what was run in the build environment and what it showed
- `docs/DEEP.md` — the optional learned tier: install, provenance, what it may do
- `docs/COMPARISON.md` — against czkawka, fclones and the hash libraries: same, different, missing
- `docs/IDENTIFIED.md` → `docs/SHELVED.md` → `docs/TODO.md` — the latest review, its items by milestone, the milestone in progress

Nothing is cached or indexed on a scan. A change writes one journal in the
OS local-data folder; `--undo` consumes it. Ctrl-C during a change stops
after the file in hand and the journal still covers every move. Set
`DEDUPE_JOURNAL_DIR` to move the journals, `NO_COLOR=1` for uncoloured
output, `DEDUPE_ASCII=1` for the ASCII glyph set.
