//! The workspace's state, with no terminal in it (TECH-03).
//!
//! Everything a key does to the workspace happens here: the cursor, the
//! scroll, the selection, the modes (browse, go-to, confirm), and the
//! effects a key asks the shell to carry out (act, preview, reveal,
//! change a keeper, undo, quit). The shell (`super`) owns the terminal and
//! the files; it feeds keys in, runs the effects, and hands back a new
//! `Layout` when the groups change. The screen composed from this state
//! is `screen.rs` (Phase 16, SC-12).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// What the workspace may do with a member.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// The group's keeper: stays.
    Keeper,
    /// Confirmed by the program's rule: selected unless deselected.
    Act,
    /// Grouped but not confirmed (or the descriptor's word alone):
    /// selectable by hand, off until chosen.
    Hand,
    /// Never acted on, not even by hand: another capture moment (another
    /// photograph), or a camera RAW.
    Locked(Lock),
}

/// Why a member is locked; the word the workspace says when space is
/// pressed on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lock {
    Kin,
    Raw,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MemberInfo {
    pub path: PathBuf,
    pub size: u64,
    pub role: Role,
}

/// One group as the workspace sees it, keeper first.
#[derive(Debug, Clone, PartialEq)]
pub struct GroupInfo {
    pub members: Vec<MemberInfo>,
}

/// The groups on screen, in listing order (exact, near, video).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Layout {
    pub groups: Vec<GroupInfo>,
}

/// One line of the body: a group's header or one of its members.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    Head(usize),
    Member(usize, usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Act {
    Milk,
    Purge,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    Browse,
    GoTo(String),
    Confirm(Act),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    PageUp,
    PageDown,
    Home,
    End,
    Space,
    Enter,
    Esc,
    Backspace,
    Char(char),
    /// Ctrl-C.
    Quit,
}

/// What the shell must do after a key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Redraw only.
    None,
    /// Act on `State::selection()`.
    Act(Act),
    /// The `would` lines for the selection, as `milk --dry-run` prints them.
    DryRun,
    /// Make member `.1` of group `.0` the keeper.
    Keeper(usize, usize),
    /// Show the group's keeper beside member `.1`.
    View(usize, usize),
    /// Reveal member `.1` of group `.0` in the file manager.
    Reveal(usize, usize),
    /// Reverse the newest change of this sitting.
    Undo,
    Quit,
}

#[derive(Debug, Clone)]
pub struct State {
    pub layout: Layout,
    pub rows: Vec<Row>,
    pub selected: HashSet<PathBuf>,
    /// Each member's role when last laid out: a path whose role changes
    /// (a new keeper re-relates its group) goes back to its default.
    roles: HashMap<PathBuf, Role>,
    pub cursor: usize,
    pub top: usize,
    /// Body rows visible.
    pub height: usize,
    pub mode: Mode,
    /// Changes made this sitting and not undone.
    pub changes: usize,
    /// One status note, cleared by the next key.
    pub message: Option<String>,
}

fn rows_of(layout: &Layout) -> Vec<Row> {
    let mut rows = Vec::new();
    for (g, group) in layout.groups.iter().enumerate() {
        rows.push(Row::Head(g));
        for m in 0..group.members.len() {
            rows.push(Row::Member(g, m));
        }
    }
    rows
}

impl State {
    pub fn new(layout: Layout, height: usize) -> State {
        let mut st = State {
            rows: rows_of(&layout),
            layout,
            selected: HashSet::new(),
            roles: HashMap::new(),
            cursor: 0,
            top: 0,
            height: height.max(1),
            mode: Mode::Browse,
            changes: 0,
            message: None,
        };
        for g in &st.layout.groups {
            for m in &g.members {
                if m.role == Role::Act {
                    st.selected.insert(m.path.clone());
                }
                st.roles.insert(m.path.clone(), m.role);
            }
        }
        // start on the first member that would be acted on, if any
        if let Some(i) = st.rows.iter().position(|r| match r {
            Row::Member(g, m) => st.layout.groups[*g].members[*m].role == Role::Act,
            Row::Head(_) => false,
        }) {
            st.cursor = i;
        }
        st.scroll();
        st
    }

    /// New groups after a change: decisions on members whose role did not
    /// change are kept; new members and members whose role changed take
    /// their default; the cursor stays on its file when the file is still
    /// listed, else on the nearest row.
    pub fn relayout(&mut self, layout: Layout) {
        let at = self.cursor_path();
        let at_group = self.cursor_group();
        let mut selected = HashSet::new();
        let mut roles = HashMap::new();
        for g in &layout.groups {
            for m in &g.members {
                let keep_choice = self.roles.get(&m.path) == Some(&m.role);
                let on = match m.role {
                    // another shot stays chosen only by the person's own `c`
                    Role::Locked(Lock::Kin) if keep_choice => self.selected.contains(&m.path),
                    Role::Keeper | Role::Locked(_) => false,
                    Role::Act | Role::Hand if keep_choice => self.selected.contains(&m.path),
                    Role::Act => true,
                    Role::Hand => false,
                };
                if on {
                    selected.insert(m.path.clone());
                }
                roles.insert(m.path.clone(), m.role);
            }
        }
        self.rows = rows_of(&layout);
        self.layout = layout;
        self.selected = selected;
        self.roles = roles;
        self.cursor = at
            .and_then(|p| {
                self.rows.iter().position(|r| match r {
                    Row::Member(g, m) => self.layout.groups[*g].members[*m].path == p,
                    Row::Head(_) => false,
                })
            })
            .or_else(|| {
                at_group.and_then(|g| {
                    let g = g.min(self.layout.groups.len().saturating_sub(1));
                    self.rows.iter().position(|r| *r == Row::Head(g))
                })
            })
            .unwrap_or(0)
            .min(self.rows.len().saturating_sub(1));
        self.scroll();
    }

    pub fn set_height(&mut self, height: usize) {
        self.height = height.max(1);
        self.scroll();
    }

    fn cursor_path(&self) -> Option<PathBuf> {
        match self.rows.get(self.cursor)? {
            Row::Member(g, m) => Some(self.layout.groups[*g].members[*m].path.clone()),
            Row::Head(_) => None,
        }
    }

    pub(super) fn cursor_group(&self) -> Option<usize> {
        match self.rows.get(self.cursor)? {
            Row::Member(g, _) | Row::Head(g) => Some(*g),
        }
    }

    fn member(&self, g: usize, m: usize) -> &MemberInfo {
        &self.layout.groups[g].members[m]
    }

    /// Keep the cursor inside the visible window.
    fn scroll(&mut self) {
        if self.rows.is_empty() {
            self.cursor = 0;
            self.top = 0;
            return;
        }
        self.cursor = self.cursor.min(self.rows.len() - 1);
        if self.cursor < self.top {
            self.top = self.cursor;
        }
        if self.cursor >= self.top + self.height {
            self.top = self.cursor + 1 - self.height;
        }
        // a group header just above the window comes into view with its
        // first member, so a member is never shown without its group
        if let Some(Row::Member(_, 0)) = self.rows.get(self.top) {
            if self.top > 0 && self.cursor + 1 < self.top + self.height {
                self.top -= 1;
            }
        }
        let max_top = self.rows.len().saturating_sub(self.height);
        self.top = self.top.min(max_top);
    }

    /// The paths to act on.
    pub fn selection(&self) -> &HashSet<PathBuf> {
        &self.selected
    }

    /// Selected files, their bytes, and how many of them only a person's
    /// choice put there.
    ///
    /// A file can sit in two groups — the keeper of an exact group is also
    /// the member that represents it in a near group (`+GROUP` on its
    /// line) — so files are counted once, by path, where they may be
    /// acted on.
    pub fn stats(&self) -> (u64, u64, u64) {
        let mut seen: HashSet<&PathBuf> = HashSet::new();
        let mut n = 0;
        let mut bytes = 0;
        let mut hand = 0;
        for g in &self.layout.groups {
            for m in &g.members {
                if matches!(m.role, Role::Act | Role::Hand | Role::Locked(Lock::Kin))
                    && self.selected.contains(&m.path)
                    && seen.insert(&m.path)
                {
                    n += 1;
                    bytes += m.size;
                    if m.role != Role::Act {
                        hand += 1;
                    }
                }
            }
        }
        (n, bytes, hand)
    }

    /// The mark beside a row.
    pub fn mark(&self, row: Row) -> Mark {
        match row {
            Row::Member(g, m) => {
                let mi = self.member(g, m);
                match mi.role {
                    Role::Keeper => Mark::None,
                    Role::Locked(Lock::Kin) if self.selected.contains(&mi.path) => Mark::On,
                    Role::Locked(_) => Mark::Lock,
                    _ if self.selected.contains(&mi.path) => Mark::On,
                    _ => Mark::Off,
                }
            }
            Row::Head(g) => {
                let choosable: Vec<&MemberInfo> = self.layout.groups[g]
                    .members
                    .iter()
                    .filter(|m| matches!(m.role, Role::Act | Role::Hand))
                    .collect();
                let on = choosable
                    .iter()
                    .filter(|m| self.selected.contains(&m.path))
                    .count();
                if choosable.is_empty() {
                    Mark::None
                } else if on == 0 {
                    Mark::Off
                } else if on == choosable.len() {
                    Mark::On
                } else {
                    Mark::Part
                }
            }
        }
    }

    fn toggle_member(&mut self, g: usize, m: usize) {
        let mi = self.member(g, m).clone();
        match mi.role {
            Role::Keeper => {
                self.message =
                    Some("the keeper stays; k on another file makes it the keeper".into())
            }
            Role::Locked(Lock::Kin) => {
                if !self.selected.remove(&mi.path) {
                    self.message = Some(
                        "another shot: a different photograph; c chooses a group's other shots"
                            .into(),
                    )
                }
            }
            Role::Locked(Lock::Raw) => {
                self.message = Some("a camera RAW: never moved by the near tier".into())
            }
            Role::Act | Role::Hand => {
                if !self.selected.remove(&mi.path) {
                    self.selected.insert(mi.path.clone());
                    if mi.role == Role::Hand {
                        self.message =
                            Some("by hand: nothing but your look confirms this one".into());
                    }
                }
            }
        }
    }

    /// On a header: anything chosen → clear the group; nothing chosen →
    /// choose its confirmed members, or, when it has none, the rest.
    fn toggle_group(&mut self, g: usize) {
        let members = self.layout.groups[g].members.clone();
        let any_on = members.iter().any(|m| self.selected.contains(&m.path));
        if any_on {
            for m in &members {
                self.selected.remove(&m.path);
            }
            return;
        }
        let acts: Vec<&MemberInfo> = members.iter().filter(|m| m.role == Role::Act).collect();
        let pick: Vec<&MemberInfo> = if acts.is_empty() {
            members.iter().filter(|m| m.role == Role::Hand).collect()
        } else {
            acts
        };
        if pick.iter().any(|m| m.role == Role::Hand) {
            self.message = Some("by hand: nothing but your look confirms these".into());
        }
        for m in pick {
            self.selected.insert(m.path.clone());
        }
    }

    /// The other shots of a group (kin: another capture moment, or another
    /// frame of the same second).
    pub(super) fn shots(&self, g: usize) -> Vec<PathBuf> {
        self.layout.groups[g]
            .members
            .iter()
            .filter(|m| m.role == Role::Locked(Lock::Kin))
            .map(|m| m.path.clone())
            .collect()
    }

    /// `c` (NF-05): a group's other shots in or out at once — a burst or a
    /// series culled by the person, the keeper kept (`k` makes another
    /// shot the keeper first). The only way another photograph is ever
    /// chosen; the program never does. [CLAUDE, Phase 11]
    fn toggle_shots(&mut self, g: usize) {
        let shots = self.shots(g);
        if shots.is_empty() {
            self.message = Some("no other shots in this group".into());
            return;
        }
        if shots.iter().any(|p| self.selected.contains(p)) {
            for p in &shots {
                self.selected.remove(p);
            }
            return;
        }
        for p in &shots {
            self.selected.insert(p.clone());
        }
        self.message = Some(format!(
            "{} other shot{} chosen by you: different photographs, the keeper stays",
            shots.len(),
            if shots.len() == 1 { "" } else { "s" }
        ));
    }

    /// Chosen members that are other shots.
    pub fn chosen_shots(&self) -> usize {
        let mut seen: HashSet<&PathBuf> = HashSet::new();
        self.layout
            .groups
            .iter()
            .flat_map(|g| g.members.iter())
            .filter(|m| m.role == Role::Locked(Lock::Kin) && self.selected.contains(&m.path))
            .filter(|m| seen.insert(&m.path))
            .count()
    }

    /// `a`: anything chosen → nothing; nothing chosen → the program's own
    /// choice (every confirmed member).
    fn toggle_all(&mut self) {
        if !self.selected.is_empty() {
            self.selected.clear();
            return;
        }
        for g in &self.layout.groups {
            for m in &g.members {
                if m.role == Role::Act {
                    self.selected.insert(m.path.clone());
                }
            }
        }
    }

    fn move_by(&mut self, delta: isize) {
        if self.rows.is_empty() {
            return;
        }
        let last = self.rows.len() as isize - 1;
        self.cursor = (self.cursor as isize + delta).clamp(0, last) as usize;
        self.scroll();
    }

    /// The (group, member) a per-file key works on: the member under the
    /// cursor, or on a header the group's first member after the keeper.
    fn target(&self) -> Option<(usize, usize)> {
        match self.rows.get(self.cursor)? {
            Row::Member(g, m) => Some((*g, *m)),
            Row::Head(g) => (self.layout.groups[*g].members.len() > 1).then_some((*g, 1)),
        }
    }

    pub fn handle(&mut self, key: Key) -> Effect {
        self.message = None;
        match self.mode.clone() {
            Mode::GoTo(mut typed) => {
                match key {
                    Key::Char(c) if c.is_ascii_digit() && typed.len() < 6 => {
                        typed.push(c);
                        self.mode = Mode::GoTo(typed);
                    }
                    Key::Backspace => {
                        typed.pop();
                        self.mode = Mode::GoTo(typed);
                    }
                    Key::Enter => {
                        self.mode = Mode::Browse;
                        match typed.parse::<usize>() {
                            Ok(n) if n >= 1 && n <= self.layout.groups.len() => {
                                if let Some(i) =
                                    self.rows.iter().position(|r| *r == Row::Head(n - 1))
                                {
                                    self.cursor = i;
                                    // the group at the top of the window
                                    self.top = i;
                                    self.scroll();
                                }
                            }
                            _ => {
                                self.message = Some(format!(
                                    "no group {typed}; 1-{} on screen",
                                    self.layout.groups.len()
                                ))
                            }
                        }
                    }
                    Key::Esc | Key::Quit => self.mode = Mode::Browse,
                    _ => {}
                }
                Effect::None
            }
            Mode::Confirm(act) => {
                self.mode = Mode::Browse;
                if key == Key::Enter {
                    Effect::Act(act)
                } else {
                    Effect::None
                }
            }
            Mode::Browse => match key {
                Key::Up => {
                    self.move_by(-1);
                    Effect::None
                }
                Key::Down => {
                    self.move_by(1);
                    Effect::None
                }
                Key::PageUp => {
                    self.move_by(-(self.height as isize));
                    Effect::None
                }
                Key::PageDown => {
                    self.move_by(self.height as isize);
                    Effect::None
                }
                Key::Home => {
                    self.cursor = 0;
                    self.scroll();
                    Effect::None
                }
                Key::End => {
                    self.cursor = self.rows.len().saturating_sub(1);
                    self.scroll();
                    Effect::None
                }
                Key::Space => {
                    match self.rows.get(self.cursor).copied() {
                        Some(Row::Member(g, m)) => self.toggle_member(g, m),
                        Some(Row::Head(g)) => self.toggle_group(g),
                        None => {}
                    }
                    Effect::None
                }
                Key::Char('a') => {
                    self.toggle_all();
                    Effect::None
                }
                Key::Char('c') => {
                    match self.rows.get(self.cursor).copied() {
                        Some(Row::Head(g)) | Some(Row::Member(g, _)) => self.toggle_shots(g),
                        None => {}
                    }
                    Effect::None
                }
                Key::Char('k') => match self.rows.get(self.cursor).copied() {
                    Some(Row::Member(g, m)) if m > 0 => Effect::Keeper(g, m),
                    Some(Row::Member(_, _)) => {
                        self.message = Some("already the keeper".into());
                        Effect::None
                    }
                    _ => {
                        self.message = Some("k on a file makes it the group's keeper".into());
                        Effect::None
                    }
                },
                Key::Char('v') => match self.target() {
                    Some((g, m)) if m > 0 => Effect::View(g, m),
                    Some((g, _)) if self.layout.groups[g].members.len() > 1 => Effect::View(g, 1),
                    _ => Effect::None,
                },
                Key::Char('o') => match self.rows.get(self.cursor).copied() {
                    Some(Row::Member(g, m)) => Effect::Reveal(g, m),
                    Some(Row::Head(g)) => Effect::Reveal(g, 0),
                    None => Effect::None,
                },
                Key::Char('g') => {
                    self.mode = Mode::GoTo(String::new());
                    Effect::None
                }
                Key::Char('n') => {
                    if self.selected.is_empty() {
                        self.message = Some("nothing selected".into());
                        Effect::None
                    } else {
                        Effect::DryRun
                    }
                }
                Key::Char('m') | Key::Char('p') => {
                    if self.selected.is_empty() {
                        self.message = Some("nothing selected; space chooses a file".into());
                    } else {
                        self.mode = Mode::Confirm(if key == Key::Char('m') {
                            Act::Milk
                        } else {
                            Act::Purge
                        });
                    }
                    Effect::None
                }
                Key::Char('u') => {
                    if self.changes > 0 {
                        Effect::Undo
                    } else {
                        self.message = Some("nothing changed this sitting".into());
                        Effect::None
                    }
                }
                Key::Char('q') | Key::Esc | Key::Quit => Effect::Quit,
                _ => Effect::None,
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    None,
    On,
    Off,
    Part,
    Lock,
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::theme::{Theme, UNICODE};

    pub(crate) fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    pub(crate) fn mi(path: &str, size: u64, role: Role) -> MemberInfo {
        MemberInfo {
            path: p(path),
            size,
            role,
        }
    }

    /// Three groups: an exact pair; a near group with a confirmed member,
    /// an unconfirmed one and another shot; a near group with only an
    /// unconfirmed member.
    pub(crate) fn layout() -> Layout {
        Layout {
            groups: vec![
                GroupInfo {
                    members: vec![
                        mi("a.jpg", 100, Role::Keeper),
                        mi("a (1).jpg", 100, Role::Act),
                    ],
                },
                GroupInfo {
                    members: vec![
                        mi("b.png", 300, Role::Keeper),
                        mi("b_small.jpg", 50, Role::Act),
                        mi("b_crop.jpg", 70, Role::Hand),
                        mi("b_next.jpg", 290, Role::Locked(Lock::Kin)),
                    ],
                },
                GroupInfo {
                    members: vec![mi("c.jpg", 40, Role::Keeper), mi("c2.jpg", 30, Role::Hand)],
                },
            ],
        }
    }

    pub(crate) fn theme(w: usize) -> Theme {
        Theme {
            color: false,
            unicode: true,
            width: w,
            live: true,
            glyphs: UNICODE,
        }
    }

    #[test]
    fn c_chooses_a_groups_other_shots_and_nothing_else_does() {
        let mut st = State::new(layout(), 20);
        let before = st.stats();
        // a, space on the group, space on the shot: never the other shot
        st.handle(Key::Char('a'));
        st.handle(Key::Char('a'));
        assert!(!st.selected.contains(&p("b_next.jpg")));
        st.cursor = st.rows.iter().position(|r| *r == Row::Head(1)).unwrap();
        for _ in 0..2 {
            st.handle(Key::Space);
            assert!(!st.selected.contains(&p("b_next.jpg")));
        }
        st.cursor = st
            .rows
            .iter()
            .position(|r| *r == Row::Member(1, 3))
            .unwrap();
        st.handle(Key::Space);
        assert!(!st.selected.contains(&p("b_next.jpg")));
        assert_eq!(st.mark(Row::Member(1, 3)), Mark::Lock);
        // c on any row of the group chooses its other shots, by hand
        st.cursor = st
            .rows
            .iter()
            .position(|r| *r == Row::Member(1, 1))
            .unwrap();
        st.handle(Key::Char('c'));
        assert!(st.selected.contains(&p("b_next.jpg")));
        assert_eq!(st.mark(Row::Member(1, 3)), Mark::On);
        assert_eq!(st.chosen_shots(), 1);
        assert_eq!(st.stats(), (before.0 + 1, before.1 + 290, before.2 + 1));
        assert!(st
            .message
            .as_deref()
            .unwrap()
            .contains("1 other shot chosen by you"));
        // the confirm names them
        st.handle(Key::Char('m'));
        let screen = st.screen(&theme(120), "t", &[], 12).join("\n");
        assert!(screen.contains("1 other shot"), "{screen}");
        st.handle(Key::Esc);
        // kept through a relayout with the same roles; space takes it back out
        st.relayout(layout());
        assert!(st.selected.contains(&p("b_next.jpg")));
        st.cursor = st
            .rows
            .iter()
            .position(|r| *r == Row::Member(1, 3))
            .unwrap();
        st.handle(Key::Space);
        assert!(!st.selected.contains(&p("b_next.jpg")));
        // a group with no other shots says so; c again clears
        st.cursor = st.rows.iter().position(|r| *r == Row::Head(0)).unwrap();
        st.handle(Key::Char('c'));
        assert!(st.message.as_deref().unwrap().contains("no other shots"));
        st.cursor = st.rows.iter().position(|r| *r == Row::Head(1)).unwrap();
        st.handle(Key::Char('c'));
        st.handle(Key::Char('c'));
        assert!(!st.selected.contains(&p("b_next.jpg")));
    }

    #[test]
    fn defaults_select_what_the_program_would_act_on() {
        let st = State::new(layout(), 20);
        assert_eq!(st.stats(), (2, 150, 0));
        // the cursor starts on the first file that would be acted on
        assert_eq!(st.rows[st.cursor], Row::Member(0, 1));
        assert_eq!(st.mark(Row::Head(0)), Mark::On);
        assert_eq!(st.mark(Row::Head(1)), Mark::Part);
        assert_eq!(st.mark(Row::Head(2)), Mark::Off);
        assert_eq!(st.mark(Row::Member(1, 0)), Mark::None);
        assert_eq!(st.mark(Row::Member(1, 3)), Mark::Lock);
    }

    #[test]
    fn a_file_in_two_groups_counts_once() {
        // e.jpg keeps an exact group and stands for it in a near group
        let l = Layout {
            groups: vec![
                GroupInfo {
                    members: vec![
                        mi("e.jpg", 10, Role::Keeper),
                        mi("e (1).jpg", 10, Role::Act),
                    ],
                },
                GroupInfo {
                    members: vec![mi("e.png", 40, Role::Keeper), mi("e.jpg", 10, Role::Act)],
                },
            ],
        };
        let st = State::new(l, 10);
        assert_eq!(st.stats(), (2, 20, 0));
        assert_eq!(
            st.mark(Row::Member(0, 0)),
            Mark::None,
            "a keeper shows no mark"
        );
    }

    #[test]
    fn space_toggles_members_groups_and_refuses_the_keeper_and_kin() {
        let mut st = State::new(layout(), 20);
        st.handle(Key::Space); // a (1).jpg off
        assert_eq!(st.stats().0, 1);
        st.handle(Key::Space); // back on
        assert_eq!(st.stats().0, 2);
        // the keeper refuses, with a word
        st.handle(Key::Up);
        st.handle(Key::Space);
        assert_eq!(st.stats().0, 2);
        assert!(st.message.as_deref().unwrap().contains("keeper"));
        // b_crop by hand
        st.cursor = st
            .rows
            .iter()
            .position(|r| *r == Row::Member(1, 2))
            .unwrap();
        st.handle(Key::Space);
        assert_eq!(st.stats(), (3, 220, 1));
        assert!(st.message.as_deref().unwrap().contains("by hand"));
        // another shot refuses
        st.handle(Key::Down);
        st.handle(Key::Space);
        assert_eq!(st.stats().0, 3);
        assert!(st.message.as_deref().unwrap().contains("c chooses"));
        // a header with something on clears the group; again, confirmed only
        st.cursor = st.rows.iter().position(|r| *r == Row::Head(1)).unwrap();
        st.handle(Key::Space);
        assert_eq!(st.mark(Row::Head(1)), Mark::Off);
        st.handle(Key::Space);
        assert!(st.selected.contains(&p("b_small.jpg")));
        assert!(!st.selected.contains(&p("b_crop.jpg")));
        // a header of a group with nothing confirmed chooses the rest
        st.cursor = st.rows.iter().position(|r| *r == Row::Head(2)).unwrap();
        st.handle(Key::Space);
        assert!(st.selected.contains(&p("c2.jpg")));
        // a: something on → nothing; again → the program's choice
        st.handle(Key::Char('a'));
        assert_eq!(st.stats().0, 0);
        st.handle(Key::Char('a'));
        assert_eq!(st.stats(), (2, 150, 0));
    }

    #[test]
    fn act_needs_a_selection_and_a_confirmation() {
        let mut st = State::new(layout(), 20);
        assert_eq!(st.handle(Key::Char('m')), Effect::None);
        assert_eq!(st.mode, Mode::Confirm(Act::Milk));
        assert_eq!(st.handle(Key::Esc), Effect::None);
        assert_eq!(st.mode, Mode::Browse);
        st.handle(Key::Char('p'));
        assert_eq!(st.handle(Key::Enter), Effect::Act(Act::Purge));
        assert_eq!(st.handle(Key::Char('n')), Effect::DryRun);
        st.handle(Key::Char('a')); // nothing selected
        assert_eq!(st.handle(Key::Char('m')), Effect::None);
        assert_eq!(st.mode, Mode::Browse);
        assert!(st.message.is_some());
        assert_eq!(st.handle(Key::Char('n')), Effect::None);
        // undo only after a change
        assert_eq!(st.handle(Key::Char('u')), Effect::None);
        st.changes = 1;
        assert_eq!(st.handle(Key::Char('u')), Effect::Undo);
        assert_eq!(st.handle(Key::Char('q')), Effect::Quit);
        assert_eq!(st.handle(Key::Quit), Effect::Quit);
    }

    #[test]
    fn keeper_view_reveal_target_the_cursor() {
        let mut st = State::new(layout(), 20);
        assert_eq!(st.handle(Key::Char('k')), Effect::Keeper(0, 1));
        assert_eq!(st.handle(Key::Char('v')), Effect::View(0, 1));
        assert_eq!(st.handle(Key::Char('o')), Effect::Reveal(0, 1));
        st.handle(Key::Up); // the keeper
        assert_eq!(st.handle(Key::Char('k')), Effect::None);
        assert_eq!(st.handle(Key::Char('v')), Effect::View(0, 1));
        st.handle(Key::Up); // the header
        assert_eq!(st.handle(Key::Char('k')), Effect::None);
        assert_eq!(st.handle(Key::Char('o')), Effect::Reveal(0, 0));
    }

    #[test]
    fn relayout_keeps_choices_resets_changed_roles_follows_the_file() {
        let mut st = State::new(layout(), 20);
        st.selected.remove(&p("a (1).jpg")); // deselected by the person
        st.selected.insert(p("b_crop.jpg")); // chosen by hand
        st.cursor = st
            .rows
            .iter()
            .position(|r| *r == Row::Member(1, 2))
            .unwrap();
        // k on b_small: it becomes the keeper; b.png becomes a confirmed
        // member (new role → default on); b_crop now confirmed (role
        // changed → default on); the choice on a (1).jpg survives
        let mut l = layout();
        l.groups[1].members = vec![
            mi("b_small.jpg", 50, Role::Keeper),
            mi("b.png", 300, Role::Act),
            mi("b_crop.jpg", 70, Role::Act),
            mi("b_next.jpg", 290, Role::Locked(Lock::Kin)),
        ];
        st.relayout(l);
        assert!(!st.selected.contains(&p("a (1).jpg")));
        assert!(!st.selected.contains(&p("b_small.jpg")));
        assert!(st.selected.contains(&p("b.png")));
        assert!(st.selected.contains(&p("b_crop.jpg")));
        assert_eq!(
            st.rows[st.cursor],
            Row::Member(1, 2),
            "cursor stays on b_crop"
        );
        // after an action group 0 is gone: rows renumber, choices stay
        let mut l = st.layout.clone();
        l.groups.remove(0);
        st.relayout(l);
        assert_eq!(st.layout.groups.len(), 2);
        assert_eq!(st.rows[st.cursor], Row::Member(0, 2));
    }

    #[test]
    fn go_to_and_scrolling_keep_the_cursor_visible() {
        let mut big = Layout::default();
        for g in 0..30 {
            big.groups.push(GroupInfo {
                members: vec![
                    mi(&format!("k{g}"), 1, Role::Keeper),
                    mi(&format!("d{g}"), 1, Role::Act),
                ],
            });
        }
        let mut st = State::new(big, 10);
        assert_eq!((st.cursor, st.top), (2, 0));
        st.handle(Key::PageDown);
        assert_eq!(st.cursor, 12);
        assert!(st.cursor >= st.top && st.cursor < st.top + st.height);
        st.handle(Key::Char('g'));
        st.handle(Key::Char('2'));
        st.handle(Key::Char('5'));
        assert_eq!(st.handle(Key::Enter), Effect::None);
        assert_eq!(st.rows[st.cursor], Row::Head(24));
        assert_eq!(st.top, st.cursor.min(st.rows.len() - st.height));
        st.handle(Key::Char('g'));
        st.handle(Key::Char('9'));
        st.handle(Key::Char('9'));
        st.handle(Key::Enter);
        assert!(st.message.as_deref().unwrap().starts_with("no group 99"));
        st.handle(Key::End);
        assert_eq!(st.cursor, st.rows.len() - 1);
        assert_eq!(st.top, st.rows.len() - st.height);
        st.handle(Key::Home);
        assert_eq!((st.cursor, st.top), (0, 0));
    }
}
