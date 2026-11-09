# CLI — resolved grammar, output design, interactive workflows

**[SPEC]** = from the attached design. **[CLAUDE]** = my decision where the
design is silent or in conflict; each one is reversible.

---

## 1. Grammar

```
dedupe                                   interactive workflow menu
dedupe <command> [PATH] [.] [img|vid]    native command
dedupe --undo | -u  [PATH]               reverse a change under PATH (several pending: choose)
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
| `vid` / `video` | videos only **[SPEC]** — one kind per run; both at once is an error, since leaving both out already means everything **[CLAUDE]** |

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

Ctrl-C **[CLAUDE]**: before a change starts it ends the run as in any
program (exit 130). Once `milk`, `purge` or `--undo` is moving files, the
first Ctrl-C lets the file in hand finish, stops, writes the journal for
what was done and prints the undo line; a second Ctrl-C ends the process
at once. While a change runs, the journal on disk is brought up to date
every 250 ms by an atomic replace, so a hard kill loses at most that
window. An interrupted `--undo` leaves the journal pending with the
entries still to restore, and the next `--undo` continues it.

Exit codes **[CLAUDE]**: 0 success, 1 user error (bad token, unreadable
root), 2 operational failure (a move or trash call failed; the journal holds
what did complete), 130 interrupted by Ctrl-C (the journal holds what did
complete).

---

## 2. Commands

### find **[SPEC]**
Runs the full suite — the exact pipeline (size → sample → full hash), the
near-duplicate image pass (DECODE → MATCH, NEAR groups) and the video
pass (FRAMES → MATCH, VIDEO groups; needs `ffmpeg` and `ffprobe` on
PATH) — prints every group, and then **stays on the result**: on a
terminal a one-line bar offers the decisions a person makes at that point
(§2a); piped, it prints the `next` lines instead, the exact `milk` and
`purge` invocations for this root. `img` / `vid` scope what is scanned
and nothing else; `--quick-search` is the fast byte-copy tier and skips
the near passes, ending with a `full` line that names the full run.
(Phase 8. Before it, `find` ran the near passes only with a kind token,
so `find . img` could list fifteen near duplicates and `milk .` answer
"no duplicates"; one rule for depth removed that.)

### 2a. The result is the workspace **[CLAUDE, under FUOE]**

On a terminal, after `find` prints its stages, groups and totals, the
same groups open in a screen of their own (the alternate screen; the
printed listing stays in the scrollback underneath). Phase 8 put a
one-line bar under the listing; Phase 9 made it this:

```
 dedupe find  C:\Photos  recursive  all kinds
 ──────────────────────────────────────────────────────────────────────
 ■ GROUP 001   2 copies   7.75 MB each   7.75 MB reclaimable
   keep  clips\vtest.avi                                     2026-11-09
▸■ dupe  clips\vtest - Copy.avi
 ◧ NEAR 005   3 similar  +1 kin   918 KB reclaimable
   keep  photos\shot_a.jpg                1282×1110   531 KB   2026-11-09
 ■ near  photos\exports\shot_a_export.jpg  961×832     223 KB   resized 75%
 □ near  photos\exports\shot_a_crop.jpg    1100×950    301 KB   similar, unconfirmed
 · kin   photos\shot_b.jpg                1282×1110   642 KB   another shot, 05:06:08
 ──────────────────────────────────────────────────────────────────────
 15 selected  19.2 MB                                         group 1/14
   ↑↓ move  space select  a all  k keeper  v view  o reveal  g go to
 ▸  m milk 15 → _milked  p purge 15 → Recycle Bin  n preview  q quit
```

Each line is the listing's own line (words first, evidence dim), two
cells narrower, with the cursor `▸` and a mark before it:

| mark | meaning |
|---|---|
| `■` | will be acted on |
| `□` | could be, is not: deselected, or grouped and not confirmed (off until chosen) |
| `◧` | a group with some of its members chosen |
| `·` | another capture moment: never acted on, not even by hand |
| (none) | the keeper |

By default the selection is exactly what `milk` and `purge` act on from
the command line. A person may add an unconfirmed member (`similar,
unconfirmed`, `looks alike`) by hand: the status line counts it (`2 by
hand`), its action line says `by hand`, and the action's footer says
those were confirmed by a look, not by a second family of evidence. The
standing rule — two families before the program acts — is unchanged; a
person's look at the two files (`v`) is the second family for the files
they choose. A file listed in two groups (the keeper of an exact group
that stands for it in a near group, `+GROUP` on its line) counts once.

| key | what happens |
|---|---|
| `↑` `↓`, PgUp PgDn, Home End | move the cursor; the window scrolls, and a group's header comes into view with its first member |
| `space` | on a file: choose it or not; on a header: anything chosen → nothing, nothing chosen → the group's confirmed members (or, when it has none, the rest, by hand) |
| `a` | anything chosen → nothing; nothing chosen → the program's own choice |
| `k` | on a file: it becomes the group's keeper; the group is re-related to it (the geometric fit included where the cheaper evidence leaves the question open) and reprinted; choices on members whose role did not change are kept |
| `v` | the keeper beside the file under the cursor, drawn in the terminal (sixel, kitty or iTerm2 graphics; a video shows the frame a third of the way in), with both paths and the file's words; any key back, `o` reveals the member. Where the terminal draws no pictures, both files open in the system viewer |
| `o` | reveal the file in Explorer, selected (Finder on macOS; the folder elsewhere) |
| `g` | type a group's number, enter: the group at the top of the window |
| `n` | the `would` lines of `milk --dry-run` for the selection, on the main screen, under the native command; enter comes back |
| `m` / `p` | a confirm bar, `milk 12 → _milked, 2.87 MB, 2 by hand   enter go   esc back`; enter acts on the selection — on the main screen, under the native command, `purge` with its VERIFY stage — then a bar: `enter back to the groups (3 left)   u undo it   q quit` |
| `u` | reverse the newest change of this sitting (its own journal, by id), restore its groups, and come back; changes stack, so `u` again reverses the one before |
| `q` / esc / Ctrl-C | leave; when nothing was changed, the `next` lines are printed so the typed forms sit in the scrollback |

Graphics: the protocol comes from the environment — kitty and Ghostty →
kitty graphics; iTerm2 and WezTerm → iTerm2 inline images; Windows
Terminal (`WT_SESSION`), foot and mlterm → sixel; otherwise none.
`DEDUPE_GRAPHICS=sixel|kitty|iterm|none` overrides it. The terminal is
not queried: a reply would arrive on the same input the keys come from,
and on Windows crossterm reads console input records, not the reply.
Sixel pictures use the 216-colour cube with 4 × 4 ordered dithering; the
cell is taken as the terminal reports it (Unix) or 9 × 18 pixels.

Piped, or with stdin not a terminal, none of this runs: `find` prints
the `next` lines, as it always has.

### milk **[SPEC]**
`find`, then moves every confirmed duplicate into `<root>/_milked/`, created
if absent — the exact duplicates and the near duplicates `find` just
listed, by the same full suite. Confirmed means what it means for
`purge` (two families of evidence agree, RESEARCH §2.7); a near duplicate
the two hashes grouped and nothing confirmed prints as `kept~` with the
reason and stays — on real photographs that was two pictures of one
chessboard from different positions, which `milk` would otherwise have
moved. A `kin` member (another capture moment) is never moved.
The relative path under the root is preserved inside `_milked`
(`a/b/x.jpg` → `_milked/a/b/x.jpg`) so a milked tree can be inspected or
restored by hand **[CLAUDE]**. A name collision inside `_milked` appends
` (2)`, ` (3)`, … before the extension **[CLAUDE]**. `_milked` is never
scanned, so running `milk` twice is idempotent.

### purge **[SPEC]**
The full suite with byte-for-byte verification of each exact duplicate
against its keeper before the bin call, and the near-duplicate image and
video passes as `find` and `milk` run them. Near duplicates
cannot be byte-verified; `purge` bins one only when a second family of
evidence agrees with the two 64-bit hashes — PDQ (256-bit, Phase 2b) or
the geometric fit (Phase 6) — or, for a member the deep descriptor
admitted, when the fit confirms it. Such lines carry `~` and the
distances, plus `fit 1.00  crop 95%` when the fit was the deciding
evidence. A member nothing confirmed prints as `kept~` with the reason
and stays. `purge` reports the OS bin it used and how
to undo. `--quick-search` skips the full hash and the near passes, never
the byte verification: a sample-level match whose bytes differ prints as
`kept   bytes differ from keeper` and stays **[CLAUDE]**. With a kind
token, only that kind is scanned (`img` → images and their pass, `vid` →
videos and theirs; no token → both).

### doctor **[SPEC]**
Checks and prints PASS / WARN / FAIL with a one-line fix for each:
terminal colour and Unicode support, terminal width, journal directory
writable, root readable and writable, `_milked` present and its size,
Recycle Bin reachable, `ffmpeg`/`ffprobe` on PATH (the video pass), the
optional model (`docs/DEEP.md`), Windows long-path support when any path
exceeds 260 characters, and the state of the journals (pending, and any
file that could not be read). The write check creates and removes one
probe file in the root.

---

## 3. Output design — theme "MILK" **[CLAUDE]**

Rules the renderer enforces:

- Width-aware: every line is measured in display cells (`unicode-width`) and
  fitted to the terminal; long paths are shortened in the middle with `…`,
  never wrapped. The path column of a NEAR or VIDEO block is sized from
  the widest tail in that block; when the path would fall under 14 cells
  a NEAR block drops its dimensions column, and a VIDEO block drops its
  codec column, then its dimensions, before the path falls under 18.
  A stage line drops its rightmost segments rather than wrap (the in-place
  redraw cannot clear a wrapped line). A totals line whose policy part does
  not fit moves that part to an indented second line.
- Colour: truecolour when the terminal is a TTY and `NO_COLOR` is unset;
  plain text otherwise. Windows virtual-terminal mode is enabled through
  crossterm at start.
- Glyphs: box-drawing and block characters only; the ASCII glyph set
  substitutes when stdout is not a terminal, when `TERM=dumb`, or when
  `DEDUPE_ASCII=1`. `NO_COLOR` alone keeps the glyphs. No emoji.
- Dates on `keep` lines are local calendar dates (what Explorer shows); the
  journal id is UTC.
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
On a terminal the `next` lines are replaced by the workspace bar (§2a);
piped, they carry the root as the user typed it (`.\_milked` when no PATH
was given) and never the `--quick-search` flag: a quick search is for
looking, the follow-up runs on full evidence. After a quick search that
saw images or videos, a `full` line names the full run. A hint's note is
shown whole or not at all; its command is never cut. An empty scan prints
SCAN and the totals only; a scan where nothing shares a size stops its
stage lines at SIZE.

Near groups follow the exact groups and continue the numbering:

```
 NEAR 003   4 similar  +1 kin   542 KB reclaimable
   keep  photos\beach.png                  1000×750   436 KB   2019-06-12
   near  photos\export\beach_mirror.png    1000×750   444 KB   mirrored                d  0  p  0  q   8
   near  photos\export\beach.jpg           1000×750   45.9 KB  same picture            d  0  p  0  q   2  +GROUP 002
   near  photos\export\beach_crop.jpg      950×712    41.7 KB  cropped 95%             d 10  p  6  q  32  fit 1.00
   near  photos\beach_small.jpg             500×375    10.6 KB  resized 50%             d  0  p  0  q   4
   kin   photos\beach_next.jpg              1000×750   440 KB   another shot, 14:02:11  d  1  p  2  q   6
 ───────────────────────────────────────────────────────────────
 2 groups   3 duplicates   646 KB reclaimable   keep: oldest copy
 1 kin   taken at a different moment: another photograph, left alone
 1 near group   4 similar   542 KB reclaimable
   keep: shared moment, most matched, then most pixels   dHash ≤10  pHash ≤14  PDQ ≤31 or fit ≥0.92 to act
```

The header counts the members beside the keeper, as the totals line does
(`+2 deep` when the deep tier added members, `+1 kin` for members taken
at another moment). Each member line says **what the member is**, in
words, before any number **[CLAUDE, Phase 8]**: `same picture` (the same
pixels in another file — an EXIF strip, a lossless re-save), `re-encoded`,
`near copy` (the hashes agree, the pixels moved a little — a consecutive
frame, a touch-up), `resized 50%`, `cropped 80%`, `letterboxed`, `with a
border`, `rotated 90°`, `rotated +4°`, `mirrored`, `flipped`, `rotated
and mirrored`, `edited` (in through the descriptor and the fit), `looks
alike` (the descriptor's word alone — listed, left in place), `another
shot, 14:02:11` (a different capture moment; the day instead of the time
when the day differs), `similar, unconfirmed` (the two hashes grouped it,
nothing confirmed it — listed, left in place). Two words at most. The
words are plain when the member will act and in the info colour when it
stays.

The evidence follows, dim: `d` and `p` are the dHash and pHash distances
to the keeper (of 64 bits), `q` the PDQ distance (of 256); `deep 0.83`
appears when the optional model is installed (`docs/DEEP.md`) and is the
SSCD cosine to the keeper; `fit 1.00` the geometric fit (RESEARCH §2.7)
where it was consulted. The numbers are the first column to go when the
width is short (the path keeps 32 cells before they do, 18 before the
dimensions go); `+GROUP 002` says the member is itself the keeper of an
exact group, whose copies are listed there. A member the deep tier alone
brought in prints with the verb `deep`; one the fit then confirmed prints
as `near` and acts. A member whose EXIF capture moment differs from the
keeper's prints with the verb `kin`; `milk` and `purge` leave `deep` and
`kin` members in place and the totals lines say how many of each there
are.

Video groups follow, numbered on:

```
 VIDEO 004   4 similar   8.04 MB reclaimable
   keep  a.mp4              640×360   0:20   h264  4.63 MB  2026-11-08
   near  a_mirror.mp4       640×360   0:20   h264  3.24 MB  mirrored               d  0 p  0  q   2  tmk    -
   near  a_small.mp4        320×180   0:20   h264  95.3 KB  scaled 640→320         d  1 p  2  q   8  tmk 1.00
   near  a_trim4.mp4        640×360   0:16   h264  2.81 MB  trimmed 0:16 of 0:20   d 15 p 20  q 102  tmk 0.98
   near  a_sd_bars.mp4      800×360   0:20   h264  1.90 MB  pillarboxed            d  0 p  0  q   4  tmk 1.00
 ───────────────────────────────────────────────────────────────
 1 video group   4 similar   8.04 MB reclaimable
   keep: longest, then most pixels   tmk ≥0.8 groups, ≥0.9 bins, or frames q ≤31
```

The words first, as for images: `re-encoded`, `scaled 768→480` (picture
widths, inside any bars), `trimmed 0:30 of 1:20`, `longer, 1:20 of 1:30`,
`pillarboxed` / `letterboxed` (black bars around the member's picture the
keeper has none of; the frames were compared inside them, which is why
the dimensions in the two columns need not agree), `bars removed`,
`mirrored`, `rotated 90°`, `similar, unconfirmed`. Then, dim, the
evidence: `d p q` are the medians over twelve sampled frames at fixed
fractions of the duration; `tmk` is the TMK level-2 score, `-` when the
mean features disagree so strongly that the offset search is skipped (a
mirror, or different content — the frames carry those). The keeper's
"most pixels" counts the picture inside the bars, and at a tie the file
without bars stays. The DECODE/MATCH and FRAMES/MATCH stage lines report
images hashed, undecodable, flat-and-skipped, videos hashed, frames,
similar pairs and files grouped.

`milk` and `purge` print the same scan block, then one line per moved or
binned file in condensed form, then a footer with the journal id and the
undo command. A `moved` line carries only the source path: the destination
is always `_milked/<same relative path>`, so it is printed once in the
footer (as `.\_milked` or `<PATH>\_milked`); a renamed collision is the one
case that gets a word on the line (`as IMG_0041 (2).jpg`). The detail at
the right of an action line uses the same fixed-width distance layout as
the NEAR and VIDEO blocks, so the details line up as a column; the path is
shortened before the detail ever is. `--dry-run` prints the same lines prefixed
`would` and writes no journal. A `LIKELY` header (quick search) says
`files`, not `copies`, because the match is sample-level. A near duplicate
being moved or binned gets `moved~` / `binned~` / `would~` with its words
and distances; the footer states once what `~` means.

---

## 4. Interactive workflow (`dedupe` with no arguments)

A single keyed menu driven by crossterm raw mode. No React, no widget
library. Five workflows, four steps, one confirmation that shows the exact
native command being run, so every menu session teaches the native form.

```
 ┏━ dedupe ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
 ┃  WORKFLOW                                        ┃
 ┃  ▸ again   D:\pics  .  img                       ┃
 ┃    find    list duplicates, then decide          ┃
 ┃    milk    move duplicates into _milked          ┃
 ┃    purge   send duplicates to the Recycle Bin    ┃
 ┃    undo    reverse the last change               ┃
 ┃    doctor  check this terminal, folder and tools ┃
 ┃                                                  ┃
 ┃  ↑↓ move   enter select   q quit                 ┃
 ┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛
```

`again` (Phase 9) **[CLAUDE]**: `find`, `milk` and `purge` record their
root, scope and kind in `recent.json` beside the journals (never in the
scanned folder); the menu's first line offers it and enter runs `find`
there at once — the workspace follows. It is not offered when there is
no record or its folder is gone.

Steps after a workflow is chosen (undo and doctor skip to confirm):

1. **FOLDER** — current directory preselected; typing replaces it.
2. **SCOPE** — `this folder only` / `include subfolders` (the `.` token).
3. **KIND** — `everything` / `images` / `videos`.
4. **CONFIRM** — shows `dedupe milk D:\pics . img` and runs it on enter.

`esc` steps back; at the first step it quits; `q` quits from any choice
step (in FOLDER it is a typed character). When the folder is left as the
current directory it is omitted from the composed command, so the menu
runs `dedupe find . img`, not `dedupe find C:\Users\…\Pictures . img`, and
the `next` lines it prints stay short. The menu offers five workflows,
plus `again` when there is a last folder **[SPEC: no bloat]**.

`--undo` with several changes pending under the folder, on a terminal,
lists them newest first — `1  2026-11-09 08:57  dedupe purge … . vid   3
entries` — and reverses the one whose number is typed (enter alone: the
newest; esc: none). Piped, or with one change pending, it reverses the
newest, as before.

---

## 5. Safety model

- `find` and `--dry-run` never write.
- `milk` moves within the same volume (rename); nothing is copied, nothing
  is deleted. A file that cannot be renamed is reported as `failed` and
  left where it is; the exit code becomes 2.
- `purge` verifies bytes before the bin call and never hard-deletes. Near
  duplicates are moved or binned only when two families of evidence agree
  — images: dHash and pHash with PDQ or the geometric fit, or the deep
  descriptor with the fit; videos: TMK level 2 ≥ 0.9, or the three image
  hashes agree on the sampled frames — marked `~` on their line; the rest
  print as `kept~` in `milk` as well as `purge`. Two files with different
  EXIF capture moments are `kin` and never acted on. `--undo` restores
  binned files like any other entry.
- Every change writes one journal; `--undo` reverses it. No cache, no index,
  no lock file, nothing persisted on a scan. `--undo` with nothing pending
  under its root names up to three other roots that have a pending
  journal, with the command that undoes each.
- Ctrl-C during a change stops it cleanly (§1); the journal covers every
  move made.
- Windows paths are shown and journaled without the `\\?\` prefix
  `canonicalize` produces; `_milked` is recognised whatever its letter
  case there.
- `_milked` and the journal directory are the only things `dedupe` creates.
- The optional model (`docs/DEEP.md`) admits members; on its own it only
  adds lines. A member it admitted moves or bins only when the geometric
  fit confirms it.
