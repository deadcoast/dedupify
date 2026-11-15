//! The batch pipeline (docs/CLI.md §2b) [CLAUDE; the owner's direction of
//! 2026-11-14: a folder of thousands of pictures wants its duplicates
//! dealt with, not two thousand comparisons]. One folder in; the whole
//! tree below it scanned (pictures and videos); its byte-for-byte copies
//! and the lookalikes two families confirm moved into `_milked`, under a
//! journal; then where they came from, folder by folder.
//!
//! Two things differ from `milk`. Keepers hold to one side of a pair of
//! folders (`cohere`): where two folders share copies, the side most of
//! their groups keep keeps all of them, so a backup is emptied whole
//! rather than half of it kept file by file. And the result is said by
//! folder (`from_lines`): what left each folder, how much of the folder
//! that was, what it was, and where the files that stayed are.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use crate::cli::Invocation;
use crate::render::fold::{self, Hue, DIM, GROOVE, PLATE};
use crate::render::Renderer;
use crate::theme::Theme;
use crate::util::{fmt_bytes, fmt_count, shorten_middle, visible_width, width};
use crate::MILKED_DIR;

use super::{act_milk, default_items, report, scan_and_group, Found, EXIT_OK};

/// The pipeline: scan, keepers to one side, report, move, say by folder,
/// and on a terminal an end bar (open `_milked`, undo, done).
pub(crate) fn batch(inv: &Invocation, r: &Renderer) -> i32 {
    r.header(inv);
    let mut f = scan_and_group(inv, r);
    cohere(&mut f);
    report(&f, inv, r);
    if f.empty() {
        return EXIT_OK;
    }
    if !r.folding() {
        r.blank();
    }
    let list = default_items(&f);
    let acted = act_milk(&f, &list, inv, r, inv.dry_run);
    // what left: a file the move could not take, or one it did not reach
    // (an interrupt), is where it was
    let failed: HashSet<&Path> = acted.failed.iter().map(|(p, _)| p.as_path()).collect();
    let moved: Vec<Moved> = list
        .iter()
        .filter(|it| it.taken() && !failed.contains(it.path))
        .filter(|it| inv.dry_run || !it.path.exists())
        .map(|it| Moved {
            path: it.path.to_path_buf(),
            keeper: it.keeper.to_path_buf(),
            size: it.size,
            words: it
                .near
                .as_deref()
                .map(|n| n.split("  ").next().unwrap_or("").to_string()),
        })
        .collect();
    if !moved.is_empty() {
        let lines = from_lines(&r.theme, &moved, &inv.root, inv.dry_run);
        r.blank();
        for l in lines {
            r.line(&l);
        }
    }
    if acted.journal.is_some() {
        if r.theme.live && std::io::stdin().is_terminal() {
            return end_bar(inv, r, acted.journal.as_deref(), acted.code);
        }
        r.blank();
        r.undo_hint(inv);
    }
    acted.code
}

/// On a terminal, after a batch that moved something: look in `_milked`,
/// take it all back, or be done. Not a question: enter ends it.
fn end_bar(inv: &Invocation, r: &Renderer, journal: Option<&str>, code: i32) -> i32 {
    use crate::workspace::{pause, state::Key};
    let milked = inv.root.join(MILKED_DIR);
    loop {
        let k = pause(
            r,
            &[
                ("o", &format!("open {MILKED_DIR}")),
                ("u", "undo it all"),
                ("enter", "done"),
            ],
        );
        match k {
            Key::Char('o') => {
                let _ = crate::preview::open(&milked);
            }
            Key::Char('u') => {
                r.blank();
                return super::act_undo_which(inv, r, journal);
            }
            _ => return code,
        }
    }
}

/// A file the batch moved (or, dry, would): where it was, the file that
/// stayed for it, its size, and for a lookalike its words.
pub(crate) struct Moved {
    pub(crate) path: PathBuf,
    pub(crate) keeper: PathBuf,
    pub(crate) size: u64,
    pub(crate) words: Option<String>,
}

fn parent_of(p: &Path) -> PathBuf {
    p.parent().map(Path::to_path_buf).unwrap_or_default()
}

/// Keepers to one side of every pair of folders that share copies. For
/// each pair, every exact group with members in both votes for the side
/// its keeper is in (by `keeper::order`); the side with more votes keeps
/// all of them, the keeper of a group on the other side moving to the
/// group's first member on the winning side. A tie changes nothing. Pairs
/// are settled strongest first and a group's keeper moves once. Only
/// byte-for-byte groups: in a lookalike group the keeper is the better
/// picture, which a folder does not outrank. Returns the keepers moved.
pub(crate) fn cohere(f: &mut Found) -> usize {
    let key = |a: &Path, b: &Path| -> (PathBuf, PathBuf) {
        if a <= b {
            (a.to_path_buf(), b.to_path_buf())
        } else {
            (b.to_path_buf(), a.to_path_buf())
        }
    };
    // votes per pair: (for the first, for the second)
    let mut votes: HashMap<(PathBuf, PathBuf), (u32, u32)> = HashMap::new();
    for g in &f.groups {
        let kd = parent_of(&g.members[0].path);
        let dirs: BTreeSet<PathBuf> = g.members.iter().map(|m| parent_of(&m.path)).collect();
        for d in dirs.iter().filter(|d| **d != kd) {
            let k = key(&kd, d);
            let e = votes.entry(k.clone()).or_default();
            if kd == k.0 {
                e.0 += 1;
            } else {
                e.1 += 1;
            }
        }
    }
    let mut pairs: Vec<((PathBuf, PathBuf), (u32, u32))> = votes.into_iter().collect();
    pairs.sort_by(|a, b| {
        (b.1 .0 + b.1 .1)
            .cmp(&(a.1 .0 + a.1 .1))
            .then(a.0.cmp(&b.0))
    });
    let winner: HashMap<(PathBuf, PathBuf), PathBuf> = pairs
        .iter()
        .filter(|(_, (a, b))| a != b)
        .map(|(k, (a, b))| (k.clone(), if a > b { k.0.clone() } else { k.1.clone() }))
        .collect();
    let strength: HashMap<(PathBuf, PathBuf), usize> = pairs
        .iter()
        .enumerate()
        .map(|(i, (k, _))| (k.clone(), i))
        .collect();
    let mut moves: Vec<(usize, usize)> = Vec::new();
    for (gi, g) in f.groups.iter().enumerate() {
        let kd = parent_of(&g.members[0].path);
        // the other folders of the group, by the strength of their pair
        let mut others: Vec<(usize, PathBuf)> = g
            .members
            .iter()
            .map(|m| parent_of(&m.path))
            .filter(|d| *d != kd)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .filter_map(|d| strength.get(&key(&kd, &d)).map(|&s| (s, d)))
            .collect();
        others.sort();
        for (_, d) in others {
            if winner.get(&key(&kd, &d)) == Some(&d) {
                if let Some(i) = g.members.iter().position(|m| parent_of(&m.path) == d) {
                    moves.push((gi, i));
                }
                break;
            }
        }
    }
    for &(gi, i) in &moves {
        // exact groups come first in the screen's numbering
        f.set_keeper(gi + 1, i);
    }
    moves.len()
}

/// A folder and the folder its files' keepers are in, raised together
/// while they end in the same name: `Backup/2019/01-Jan` beside
/// `2019/01-Jan` is `Backup` beside the root, a mirror. Relative to the
/// root (`""` is the root). Says whether it was raised.
fn lift(d: &Path, k: &Path) -> (PathBuf, PathBuf, bool) {
    let (mut d, mut k) = (d.to_path_buf(), k.to_path_buf());
    let mut lifted = false;
    while d != k {
        let same = matches!((d.file_name(), k.file_name()), (Some(a), Some(b)) if a == b);
        if !same {
            break;
        }
        let (Some(pd), Some(pk)) = (d.parent(), k.parent()) else {
            break;
        };
        d = pd.to_path_buf();
        k = pk.to_path_buf();
        lifted = true;
    }
    (d, k, lifted)
}

/// One folder's line in the batch's account.
#[derive(Default)]
struct From {
    count: u64,
    bytes: u64,
    exact: u64,
    /// the folders the keepers are in (relative), and whether the folder
    /// mirrors them
    keepers: BTreeSet<PathBuf>,
    mirror: bool,
    words: Vec<String>,
}

/// The files under `d` (relative to `root`): its whole tree when it was
/// raised as a mirror, else the folder's own files. `_milked` not counted.
fn files_in(root: &Path, d: &Path, tree: bool) -> u64 {
    let abs = root.join(d);
    let milked = root.join(MILKED_DIR);
    walkdir::WalkDir::new(&abs)
        .max_depth(if tree { usize::MAX } else { 1 })
        .into_iter()
        .filter_entry(|e| e.path() != milked)
        .flatten()
        .filter(|e| e.file_type().is_file())
        .count() as u64
}

fn shown(p: &Path, root: &Path) -> String {
    if p.as_os_str().is_empty() {
        root.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| root.display().to_string())
    } else {
        p.display().to_string()
    }
}

/// A folder's path in `n` cells, its last folder kept whole where it can
/// be: `Pictures\…\Camera`, then `…\Backup (old drive)`, then that cut
/// in its middle.
fn fit_path(p: &str, n: usize, ell: &str) -> String {
    if width(p) <= n {
        return p.to_string();
    }
    let sep = std::path::MAIN_SEPARATOR;
    let parts: Vec<&str> = p.split(['/', '\\']).filter(|x| !x.is_empty()).collect();
    if parts.len() >= 2 {
        let last = parts[parts.len() - 1];
        let first = parts[0];
        let both = format!("{first}{sep}{ell}{sep}{last}");
        if parts.len() > 2 && width(&both) <= n {
            return both;
        }
        let tail = format!("{ell}{sep}{last}");
        if width(&tail) <= n {
            return tail;
        }
    }
    shorten_middle(p, n, ell)
}

/// Where the keepers are, as short as says it: up to three folders by
/// name (those under one parent as `parent\a, b, c`), else their top
/// folders and how many (`2019, 2020 · 24 folders`).
fn keepers_words(k: &BTreeSet<PathBuf>, root: &Path) -> String {
    let sep = std::path::MAIN_SEPARATOR;
    let v: Vec<&PathBuf> = k.iter().collect();
    let parents: BTreeSet<PathBuf> = v.iter().map(|p| parent_of(p)).collect();
    if v.len() > 1 && v.len() <= 4 && parents.len() == 1 && !v[0].as_os_str().is_empty() {
        let p = parents.into_iter().next().unwrap_or_default();
        let leaves: Vec<String> = v
            .iter()
            .map(|x| {
                x.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            })
            .collect();
        return if p.as_os_str().is_empty() {
            leaves.join(", ")
        } else {
            format!("{}{sep}{}", p.display(), leaves.join(", "))
        };
    }
    if v.len() <= 3 {
        return v
            .iter()
            .map(|p| shown(p, root))
            .collect::<Vec<_>>()
            .join(", ");
    }
    let tops: BTreeSet<String> = v
        .iter()
        .map(|p| {
            p.components()
                .next()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .unwrap_or_else(|| shown(Path::new(""), root))
        })
        .collect();
    if tops.len() <= 3 {
        format!(
            "{} · {} folders",
            tops.into_iter().collect::<Vec<_>>().join(", "),
            fmt_count(v.len() as u64)
        )
    } else {
        format!("{} folders", fmt_count(v.len() as u64))
    }
}

/// Where the keepers are in a few words: their top folders and how many.
fn keepers_short(k: &BTreeSet<PathBuf>, root: &Path) -> String {
    let tops: BTreeSet<String> = k
        .iter()
        .map(|p| {
            p.components()
                .next()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .unwrap_or_else(|| shown(Path::new(""), root))
        })
        .collect();
    let n = k.len();
    let tops: Vec<String> = tops.into_iter().collect();
    match (tops.len(), n) {
        (1, 1) => tops[0].clone(),
        (1, _) => format!("{} · {} folders", tops[0], fmt_count(n as u64)),
        (t, _) if t <= 2 => format!("{} · {} folders", tops.join(", "), fmt_count(n as u64)),
        _ => format!("{} folders", fmt_count(n as u64)),
    }
}

/// The batch's account, by folder: a heading, then a line for each
/// folder files left (the most first, eight at most, the rest summed),
/// then the copies that sat beside their keepers. Each line: the folder;
/// how many left it; a bar of that against the files it held (`all` when
/// it is empty now); what they were; where their keepers are.
pub(crate) fn from_lines(t: &Theme, moved: &[Moved], root: &Path, dry: bool) -> Vec<String> {
    let rel = |p: &Path| -> PathBuf {
        p.strip_prefix(root)
            .map(Path::to_path_buf)
            .unwrap_or_else(|_| p.to_path_buf())
    };
    let mut by: HashMap<(PathBuf, bool), From> = HashMap::new();
    let mut beside = From::default();
    let mut beside_dirs: BTreeSet<PathBuf> = BTreeSet::new();
    for m in moved {
        let d = parent_of(&rel(&m.path));
        let k = parent_of(&rel(&m.keeper));
        let e = if d == k {
            beside_dirs.insert(d.clone());
            &mut beside
        } else {
            let (d2, k2, lifted) = lift(&d, &k);
            let e = by.entry((d2, lifted)).or_default();
            e.keepers.insert(k2);
            e.mirror |= lifted;
            e
        };
        e.count += 1;
        e.bytes += m.size;
        match &m.words {
            None => e.exact += 1,
            Some(w) => e.words.push(w.clone()),
        }
    }
    let mut rows: Vec<(PathBuf, bool, From)> =
        by.into_iter().map(|((d, l), f)| (d, l, f)).collect();
    rows.sort_by(|a, b| b.2.count.cmp(&a.2.count).then(a.0.cmp(&b.0)));

    // what they were: `copies` for bytes, the lookalikes' words tallied;
    // the first of them, where the keepers are, then the rest
    let what = |f: &From| -> Vec<String> {
        let mut v = Vec::new();
        if f.exact > 0 {
            v.push(if f.exact == f.count {
                t.paint_rgb(Hue::Copies.rgb(), "copies")
            } else {
                format!(
                    "{} {}",
                    t.paint_rgb(Hue::Copies.rgb(), "copies"),
                    t.paint_rgb_bold(PLATE, &fmt_count(f.exact))
                )
            });
        }
        for (w, n) in fold::tally(f.words.iter().cloned()).into_iter().take(2) {
            v.push(format!(
                "{} {}",
                t.paint_rgb(Hue::Similar.rgb(), &w),
                t.paint_rgb_bold(PLATE, &fmt_count(n))
            ));
        }
        v
    };
    let hue_of = |f: &From| {
        if f.exact * 2 >= f.count {
            Hue::Copies.rgb()
        } else {
            Hue::Similar.rgb()
        }
    };

    let shown_rows = rows.len().min(8);
    let names: Vec<String> = rows
        .iter()
        .take(shown_rows)
        .map(|(d, _, _)| shown(d, root))
        .collect();
    let beside_label = "beside their keepers";
    let w = t.width.saturating_sub(1);
    // the bar where there is room for it; the share's word always
    let bar_w = if w >= 110 {
        10
    } else if w >= 90 {
        6
    } else {
        0
    };
    let name_w = names
        .iter()
        .map(|n| width(n))
        .chain(std::iter::once(if beside.count > 0 {
            width(beside_label)
        } else {
            0
        }))
        .max()
        .unwrap_or(8)
        // without the bar the names have more of the line
        .clamp(8, (w * if bar_w > 0 { 30 } else { 40 } / 100).max(12))
        // the copies' own line keeps its words whole
        .max(if beside.count > 0 {
            width(beside_label).min(w.saturating_sub(18))
        } else {
            0
        });
    let count_w = rows
        .iter()
        .map(|r| fmt_count(r.2.count).len())
        .chain(std::iter::once(fmt_count(beside.count).len()))
        .max()
        .unwrap_or(1);
    let (on, off) = if t.unicode {
        ("━", "─")
    } else {
        ("=", "-")
    };

    let mut out = Vec::new();
    let folders = rows.len() + usize::from(beside.count > 0);
    out.push(format!(
        " {}  {}",
        t.paint_rgb_bold(PLATE, "FROM"),
        t.paint_rgb(
            DIM,
            &if folders == 1 {
                "1 folder".to_string()
            } else {
                format!("{} folders", fmt_count(folders as u64))
            }
        )
    ));
    // the files each folder held: what is there now and what left it
    let held: Vec<u64> = rows
        .iter()
        .take(shown_rows)
        .map(|(d, lifted, f)| {
            let now = files_in(root, d, *lifted);
            if dry {
                now
            } else {
                now + f.count
            }
        })
        .collect();
    // the widest share word on screen: `all`, or `of` and the count
    let share_word_w = rows
        .iter()
        .zip(&held)
        .map(|(r, &of)| {
            if r.2.count >= of {
                3
            } else {
                3 + fmt_count(of).len()
            }
        })
        .max()
        .unwrap_or(3);
    let line = |name: &str,
                f: &From,
                share: Option<(u64, u64)>,
                rest: Vec<String>,
                short: Vec<String>|
     -> String {
        let mut s = format!(
            " {}",
            t.paint_rgb(
                PLATE,
                &crate::util::fit(&fit_path(name, name_w, t.glyphs.ellipsis), name_w)
            )
        );
        s.push_str(&format!(
            "  {}",
            t.paint_rgb_bold(PLATE, &format!("{:>count_w$}", fmt_count(f.count)))
        ));
        let mut used = 1 + name_w + 2 + count_w;
        // the share: a bar where there is room, then `all` or `of N`
        let share_w = (if bar_w > 0 { 1 + bar_w } else { 0 }) + 1 + share_word_w;
        match share {
            Some((n, of)) => {
                if bar_w > 0 {
                    let fill = if of == 0 {
                        bar_w
                    } else {
                        ((n as f64 / of as f64) * bar_w as f64)
                            .round()
                            .clamp(1.0, bar_w as f64) as usize
                    };
                    s.push_str(&format!(
                        "  {}{}",
                        t.paint_rgb(hue_of(f), &on.repeat(fill)),
                        t.paint_rgb(GROOVE, &off.repeat(bar_w - fill))
                    ));
                }
                let (word, word_w) = if n >= of {
                    (t.paint_rgb_bold(PLATE, "all"), 3)
                } else {
                    let w = format!("of {}", fmt_count(of));
                    let ww = width(&w);
                    (t.paint_rgb(DIM, &w), ww)
                };
                s.push_str(&format!(
                    " {word}{}",
                    " ".repeat(share_word_w.saturating_sub(word_w))
                ));
            }
            None => s.push_str(&" ".repeat(share_w)),
        }
        used += share_w;
        let room = w.saturating_sub(used + 2);
        // where the keepers are keeps its place: its short form before
        // it would be dropped
        let mut rest = rest;
        if rest.len() >= 2 && visible_width(&rest[0]) + 3 + visible_width(&rest[1]) > room {
            if let Some(s) = short
                .iter()
                .find(|s| visible_width(&rest[0]) + 3 + visible_width(s) <= room)
            {
                rest[1] = s.clone();
            }
        }
        s.push_str("  ");
        s.push_str(&fold::fit_segments(t, &rest, room));
        s
    };
    for (i, (_, _, f)) in rows.iter().take(shown_rows).enumerate() {
        let of = held[i];
        let mut rest = what(f);
        let say = |kw: String| {
            t.paint_rgb(
                DIM,
                &if f.mirror {
                    format!("mirrors {kw}")
                } else {
                    format!("kept in {kw}")
                },
            )
        };
        rest.insert(1.min(rest.len()), say(keepers_words(&f.keepers, root)));
        // shorter ways to say where: their top folders, then how many
        let mut short = vec![say(keepers_short(&f.keepers, root))];
        if f.keepers.len() > 1 {
            short.push(say(format!(
                "{} folders",
                fmt_count(f.keepers.len() as u64)
            )));
        }
        out.push(line(&names[i], f, Some((f.count, of)), rest, short));
    }
    if rows.len() > shown_rows {
        let more: Vec<&From> = rows.iter().skip(shown_rows).map(|r| &r.2).collect();
        let n: u64 = more.iter().map(|f| f.count).sum();
        let b: u64 = more.iter().map(|f| f.bytes).sum();
        out.push(format!(
            " {}",
            t.paint_rgb(
                DIM,
                &format!(
                    "and {} more folders · {} files · {}",
                    fmt_count(more.len() as u64),
                    fmt_count(n),
                    fmt_bytes(b)
                )
            )
        ));
    }
    if beside.count > 0 {
        let mut rest = what(&beside);
        rest.insert(
            1.min(rest.len()),
            t.paint_rgb(
                DIM,
                &format!(
                    "in {}",
                    if beside_dirs.len() == 1 {
                        shown(
                            beside_dirs
                                .iter()
                                .next()
                                .map(PathBuf::as_path)
                                .unwrap_or(Path::new("")),
                            root,
                        )
                    } else {
                        format!("{} folders", fmt_count(beside_dirs.len() as u64))
                    }
                ),
            ),
        );
        out.push(line(beside_label, &beside, None, rest, Vec::new()));
    }
    debug_assert!(out.iter().all(|l| visible_width(l) <= t.width));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mirror_is_raised_to_where_its_names_part() {
        let (d, k, l) = lift(Path::new("Backup/2019/01-Jan"), Path::new("2019/01-Jan"));
        assert_eq!(
            (d.as_path(), k.as_path(), l),
            (Path::new("Backup"), Path::new(""), true)
        );
        let (d, k, l) = lift(Path::new("Phone/DCIM/Camera"), Path::new("2020/01-Jan"));
        assert_eq!(
            (d.as_path(), k.as_path(), l),
            (
                Path::new("Phone/DCIM/Camera"),
                Path::new("2020/01-Jan"),
                false
            )
        );
        let (d, k, l) = lift(Path::new("old/Pictures/2019"), Path::new("Pictures/2019"));
        assert_eq!(
            (d.as_path(), k.as_path(), l),
            (Path::new("old"), Path::new(""), true)
        );
    }

    #[test]
    fn keepers_are_said_as_short_as_says_them() {
        let root = Path::new("/x/library");
        let set = |v: &[&str]| -> BTreeSet<PathBuf> { v.iter().map(PathBuf::from).collect() };
        let sep = std::path::MAIN_SEPARATOR;
        assert_eq!(
            keepers_words(&set(&["2020/01-Jan", "2020/02-Feb", "2020/03-Mar"]), root),
            format!("2020{sep}01-Jan, 02-Feb, 03-Mar")
        );
        assert_eq!(keepers_words(&set(&[""]), root), "library");
        // paths as the walk gives them: in the system's separator
        let two: BTreeSet<PathBuf> = [PathBuf::from("a"), Path::new("b").join("c")]
            .into_iter()
            .collect();
        assert_eq!(keepers_words(&two, root), format!("a, b{sep}c"));
        let many: Vec<String> = (1..=12)
            .flat_map(|m| [format!("2019/{m:02}"), format!("2020/{m:02}")])
            .collect();
        let many: Vec<&str> = many.iter().map(String::as_str).collect();
        assert_eq!(keepers_words(&set(&many), root), "2019, 2020 · 24 folders");
    }

    #[test]
    fn the_account_fits_every_width_and_says_the_mirror() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("lib");
        let put = |rel: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, rel.as_bytes()).unwrap();
            p
        };
        let mut moved = Vec::new();
        for m in ["01-Jan", "02-Feb"] {
            for n in 0..3 {
                let k = put(&format!("Pictures/2019/{m}/IMG_{n}.jpg"));
                moved.push(Moved {
                    path: root.join(format!("Pictures/Backup (old drive)/2019/{m}/IMG_{n}.jpg")),
                    keeper: k,
                    size: 1000,
                    words: None,
                });
            }
        }
        let k = put("Pictures/2020/IMG_9.jpg");
        put("Downloads/other.jpg");
        moved.push(Moved {
            path: root.join("Downloads/received.jpg"),
            keeper: k.clone(),
            size: 500,
            words: Some("resized 50%".into()),
        });
        moved.push(Moved {
            path: root.join("Pictures/2020/IMG_9 - Copy.jpg"),
            keeper: k,
            size: 900,
            words: None,
        });
        for unicode in [true, false] {
            for w in 40..=200 {
                let t = Theme::custom(true, unicode, w, true);
                let lines = from_lines(&t, &moved, &root, false);
                assert_eq!(lines.len(), 4, "{w}: {lines:#?}");
                for l in &lines {
                    assert!(visible_width(l) < w, "{w}: {l:?}");
                }
                let plain: Vec<String> = lines.iter().map(|l| crate::util::strip_sgr(l)).collect();
                assert!(plain[0].contains("FROM") && plain[0].contains("3 folders"));
                // the backup: six files, all it held, a mirror of Pictures
                assert!(
                    plain[1].contains(" 6 ") && plain[1].contains("all"),
                    "{w}: {}",
                    plain[1]
                );
                if w >= 80 {
                    assert!(plain[1].contains("mirrors Pictures"), "{w}: {}", plain[1]);
                }
                // Downloads held two: one left
                assert!(plain[2].contains("of 2"), "{w}: {}", plain[2]);
                assert!(
                    plain[3].contains("beside their keepers"),
                    "{w}: {}",
                    plain[3]
                );
            }
        }
    }
}
