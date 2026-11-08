# dedupe — the Dedupelifier

A duplicate finder for the terminal. Exact duplicates everywhere;
near-duplicate images (rescaled, re-encoded, mirrored, rotated) with the
`img` token, near-duplicate videos (re-encoded, scaled, trimmed, mirrored,
sped up) with the `vid` token — `ffmpeg` and `ffprobe` on PATH — and both
on every `purge`, where a stricter confirmation (PDQ for images, TMK for
videos) must hold before anything is binned.

```
cargo install --path .
```

```
dedupe                      workflow menu
dedupe find [PATH] [.]      see every duplicate group, change nothing
dedupe milk [PATH] [.]      move duplicates into PATH\_milked
dedupe purge [PATH] [.]     verify bytes, then Recycle Bin
dedupe --undo [PATH]        reverse the last milk or purge
dedupe doctor [PATH]        checks, with a fix line for each
```

`.` includes subfolders; `img` / `vid` restrict the kind and add that
kind's near-duplicate pass; `-n` dry run; `-q` size + sample only; `-h` the
help page.

- `docs/PLAN.md` — the phases, what this build contains, what is next
- `docs/RESEARCH.md` — the mathematics and the papers behind each choice
- `docs/CLI.md` — the resolved grammar, output design, menu, safety model
- `docs/VERIFICATION.md` — what was run in the build environment and what it showed

Nothing is cached or indexed on a scan. A change writes one journal in the
OS local-data folder; `--undo` consumes it. Set `DEDUPE_JOURNAL_DIR` to
move the journals, `NO_COLOR=1` for plain output, `DEDUPE_ASCII=1` for the
ASCII glyph set.
