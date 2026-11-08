# CLI — resolved grammar, output design, interactive workflows

**[SPEC]** = from the attached design. **[CLAUDE]** = my decision where the
design is silent or in conflict; each one is reversible.

---

## 1. Grammar

```
dedupe                                   interactive workflow menu
dedupe <command> [PATH] [.] [img|vid]    native command
dedupe --undo | -u  [PATH]               reverse the last change under PATH
dedupe doctor | --doctor | -dr [PATH]    troubleshooting suite
dedupe --help | -h                       help page
```

Commands **[SPEC]**: `find`, `milk`, `purge`.

Positional tokens are order-independent after the command **[CLAUDE]**:

| token | meaning |
|---|---|
| `PATH` | root directory; default is the current directory |
| `.` | **all** — recurse into subdirectories **[SPEC]**. Absent ⇒ the root's own files only |
| `img` / `image` | images only **[SPEC]** |
| `vid` / `video` | videos only **[SPEC]** |

Conflict resolved **[CLAUDE]**: `.` is both "the current directory" and the
spec's "all" token. Rule: a lone `.` is always the **all** switch; the root is
`PATH` when given, else the current directory. So `dedupe find .` and
`dedupe find D:\pics .` both recurse, and `dedupe find` scans the current
directory without recursing. There is no way to pass `.` as a path, and none
is needed — omitting PATH already means the current directory.

Flags:

| long | short | effect |
|---|---|---|
| `--help` | `-h` | **[SPEC]** proprietary help page |
| `--doctor` | `-dr` | **[SPEC]** troubleshooting suite. `-dr` is two characters; POSIX would read it as `-d -r`. `dedupe` has no `-d` or `-r`, and the token is rewritten to `doctor` before parsing, so the spec's alias works literally **[CLAUDE]** |
| `--dry-run` | `-n` | **[SPEC]** flag; short form **[CLAUDE]**, the convention of `make -n`, `rsync -n`, `git clean -n` |
| `--quick-search` | `-q` | **[SPEC]** flag; short form **[CLAUDE]**. `-q` is free because `dedupe` has no quiet mode: output is designed, not silenced; piping to a non-terminal switches to plain text automatically |
| `--undo` | `-u` | **[SPEC]** flag; the spec's `-` alias is unusable (a lone `-` is the stdin convention and cannot carry a flag), so `-u` **[CLAUDE]** |

`find --dry-run` is accepted and identical to `find`: `find` never changes
anything. `--undo` may follow any command (`dedupe milk --undo` **[SPEC]**)
and ignores the command: the journal, not the command, says what to reverse.

Exit codes **[CLAUDE]**: 0 success, 1 user error (bad token, unreadable
root), 2 operational failure (a move or trash call failed; the journal holds
what did complete).

---

## 2. Commands

### find **[SPEC]**
Runs the exact-duplicate pipeline (size → sample → full hash), prints every
group, then the **next** lines: the exact `milk` and `purge` invocations for
this root, so the follow-up is a copy-paste. With the `img` token the
near-duplicate image pass runs as well (DECODE → MATCH stages, NEAR
groups); with `vid`, the video pass (FRAMES → MATCH, VIDEO groups; needs
`ffmpeg` and `ffprobe` on PATH). Without them, an exact-only run that saw
images or videos ends with a `near` / `video` line giving the token form of
the same command **[CLAUDE]**: the default stays fast, the comprehensive
form is one line away.

### milk **[SPEC]**
`find`, then moves every duplicate into `<root>/_milked/`, created if absent.
The relative path under the root is preserved inside `_milked`
(`a/b/x.jpg` → `_milked/a/b/x.jpg`) so a milked tree can be inspected or
restored by hand **[CLAUDE]**. A name collision inside `_milked` appends
` (2)`, ` (3)`, … before the extension **[CLAUDE]**. `_milked` is never
scanned, so running `milk` twice is idempotent.

### purge **[SPEC]**
The full suite: exact pipeline with byte-for-byte verification of each
duplicate against its keeper, **and** the near-duplicate image and video
passes on every run, whatever the kind token. Near duplicates
cannot be byte-verified; `purge` bins one only when a third hash, PDQ
(256-bit), also agrees with the keeper (Phase 2b). Such lines carry `~` and
the three distances. A member the two 64-bit hashes grouped but PDQ did not
confirm prints as `kept~` with the reason, and the footer names the `milk`
command that moves it instead. `purge` reports the OS bin it used and how
to undo. `--quick-search` skips the near pass: quick is the obvious-
duplicates tier.

### doctor **[SPEC]**
Checks and prints PASS / WARN / FAIL with a one-line fix for each:
terminal colour and Unicode support, terminal width, journal directory
writable, root readable and writable, `_milked` present and its size,
Recycle Bin reachable, `ffmpeg`/`ffprobe` on PATH (the video pass), Windows
long-path support when any path exceeds 260 characters, and the state of
the last journal (consumed or pending).

---

## 3. Output design — theme "MILK" **[CLAUDE]**

Rules the renderer enforces:

- Width-aware: every line is measured in display cells (`unicode-width`) and
  fitted to the terminal; long paths are shortened in the middle with `…`,
  never wrapped.
- Colour: truecolour when the terminal is a TTY and `NO_COLOR` is unset;
  plain text otherwise. Windows virtual-terminal mode is enabled through
  crossterm at start.
- Glyphs: box-drawing and block characters only; an ASCII glyph set
  substitutes when the output encoding is not UTF-8. No emoji.
- Progress is redrawn in place on one line per stage; a finished stage
  leaves its final line behind, so the scrollback reads as a record.
- No background fills. Frames are single-weight `━`/`─` rules; a frame rule
  is the edge of its panel, nothing extends past it.

Palette:

| role | colour | used for |
|---|---|---|
| plate | `#E8E4D8` | primary text |
| keep | `#FFB000` | keeper lines, selected menu item |
| dupe | `#8A8A8A` | duplicate lines, secondary text |
| rule | `#4A4A4A` | frame rules |
| ok | `#7CD992` | PASS, completed stage |
| warn | `#FFB000` | WARN |
| danger | `#FF4D4D` | FAIL, `purge` |
| info | `#5FB3FF` | counts, byte totals |

Result layout (`find`):

```
 dedupe find  D:\pics  recursive  all kinds
 ───────────────────────────────────────────────────────────────
 SCAN   ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━  12,480 files   38.2 GB   0.9 s
 SIZE   ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━   3,112 share a size
 SAMPLE ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━   1,904 share a sample
 HASH   ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━   1,771 confirmed            4.1 s

 GROUP 001   3 copies   24.1 MB each   48.2 MB reclaimable
   keep  2019\trip\IMG_0041.jpg                        2019-06-12
   dupe  backup\IMG_0041.jpg
   dupe  backup\old\IMG_0041 (1).jpg
 ───────────────────────────────────────────────────────────────
 612 groups   1,159 duplicates   11.4 GB reclaimable   keep: oldest copy
 next   dedupe milk D:\pics .     move duplicates into D:\pics\_milked
        dedupe purge D:\pics .    send duplicates to the Recycle Bin
```

Groups are ordered by reclaimable bytes, largest first, and numbered. Paths
are shown relative to the root. The keeper line carries its modification
date; duplicate lines do not repeat the size (it is in the group header).

Near groups follow the exact groups and continue the numbering:

```
 NEAR 003   5 similar   542 KB reclaimable
   keep  photos\beach.png                        1000×750   436 KB   2019-06-12
   near  photos\export\beach_mirror.png          1000×750   444 KB   d  0  p  0  q   8  mirror
   near  photos\export\beach.jpg                 1000×750   45.9 KB  d  0  p  0  q   2  +GROUP 002
   near  photos\export\beach_crop.jpg            950×712    41.7 KB  d 10  p  6  q  32
   near  photos\beach_small.jpg                   500×375    10.6 KB  d  0  p  0  q   4
 ───────────────────────────────────────────────────────────────
 2 groups   3 duplicates   646 KB reclaimable   keep: oldest copy
 1 near group   4 similar   542 KB reclaimable   keep: most pixels   dHash ≤10  pHash ≤14  PDQ ≤31 to bin
```

`d` and `p` are the dHash and pHash distances to the keeper (of 64 bits),
`q` the PDQ distance (of 256); `mirror`, `rot 90`, `flip`, `transpose` name
the orientation that matched; `+GROUP 002` says the member is itself the
keeper of an exact group, whose copies are listed there. A distance over
its threshold is lit in the warning colour: `d`/`p` when the member is
linked only through another member, `q` when `purge` will keep it (`q 32`
on the crop above).

Video groups follow, numbered on:

```
 VIDEO 004   4 similar   6.14 MB reclaimable
   keep  a.mp4              640×360   0:20   h264  4.63 MB  2026-11-08
   near  a_mirror.mp4       640×360   0:20   h264  3.24 MB  d  0 p  0  q   2  tmk   -    mirror
   near  a_small.mp4        320×180   0:20   h264  95.3 KB  d  1 p  2  q   8  tmk 1.00
   near  a_trim4.mp4        640×360   0:16   h264  2.81 MB  d 15 p 20  q 102  tmk 0.98  trim
 ───────────────────────────────────────────────────────────────
 1 video group   3 similar   6.14 MB reclaimable   keep: longest, then most pixels   tmk ≥0.8 groups, ≥0.9 or frames q ≤31 to bin
```

`d p q` are the medians over twelve sampled frames at fixed fractions of
the duration; `tmk` is the TMK level-2 score, `-` when the mean features
disagree so strongly that the offset search is skipped (a mirror, or
different content — the frames carry those); `trim` marks a duration more
than 2 % off the keeper's. A `tmk` between 0.8 and 0.9 is lit in the
warning colour: grouped, not binned. The two DECODE and
MATCH stage lines report images hashed, undecodable, flat-and-skipped,
similar pairs and images grouped.

`milk` and `purge` print the same scan block, then one line per moved or
binned file in condensed form, then a footer with the journal id and the
undo command. A `moved` line carries only the source path: the destination
is always `_milked/<same relative path>`, so it is printed once in the
footer; a renamed collision is the one case that gets a word on the line
(`as IMG_0041 (2).jpg`). `--dry-run` prints the same lines prefixed
`would` and writes no journal. A `LIKELY` header (quick search) says
`files`, not `copies`, because the match is sample-level. A near duplicate
being moved or binned gets `moved~` / `binned~` / `would~` with its
distances; the footer states once what `~` means.

---

## 4. Interactive workflow (`dedupe` with no arguments)

A single keyed menu driven by crossterm raw mode. No React, no widget
library. Five workflows, four steps, one confirmation that shows the exact
native command being run, so every menu session teaches the native form.

```
 ┏━ dedupe ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
 ┃  WORKFLOW                                        ┃
 ┃  ▸ find    see duplicates, change nothing        ┃
 ┃    milk    move duplicates into _milked          ┃
 ┃    purge   send duplicates to the Recycle Bin    ┃
 ┃    undo    reverse the last change               ┃
 ┃    doctor  check this terminal and folder        ┃
 ┃                                                  ┃
 ┃  ↑↓ move   enter select   esc back / quit        ┃
 ┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛
```

Steps after a workflow is chosen (undo and doctor skip to confirm):

1. **FOLDER** — current directory preselected; typing replaces it.
2. **SCOPE** — `this folder only` / `include subfolders` (the `.` token).
3. **KIND** — `everything` / `images` / `videos`.
4. **CONFIRM** — shows `dedupe milk D:\pics . img` and runs it on enter.

`esc` steps back; at the first step it quits. The menu never offers more
than five choices on one screen **[SPEC: no bloat]**.

---

## 5. Safety model

- `find` and `--dry-run` never write.
- `milk` moves within the same volume (rename); nothing is copied, nothing
  is deleted. A file that cannot be renamed is reported as `failed` and
  left where it is; the exit code becomes 2.
- `purge` verifies bytes before the bin call and never hard-deletes. Near
  duplicates go to the bin only when confirmed against the keeper — images:
  dHash, pHash and PDQ all agree; videos: TMK level 2 ≥ 0.9, or the three
  image hashes agree on the sampled frames — marked `~` on their line; the
  rest print as `kept~`. `--undo` restores binned files like any other
  entry.
- Every change writes one journal; `--undo` reverses it. No cache, no index,
  no lock file, nothing persisted on a scan.
- `_milked` and the journal directory are the only things `dedupe` creates.
