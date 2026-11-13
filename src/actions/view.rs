//! One group on screen, whichever kind (Phase 12, SC-05): what the
//! workspace, a keeper change and the preview need from a group without
//! asking whether it is exact, near or video. `Found::views` walks them in
//! the order they are numbered on screen — exact, then near, then video —
//! so no caller counts offsets across the three lists.

use std::path::{Path, PathBuf};

use crate::exact::Group;
use crate::kind::{has_ext, IMAGE_EXT, VIDEO_EXT};
use crate::near::NearGroup;
use crate::preview::Visual;
use crate::util::fmt_bytes;
use crate::video::VideoGroup;
use crate::workspace::state::{Lock, Role};

use super::Found;

pub(crate) trait GroupView {
    /// Files in the group, the keeper first.
    fn len(&self) -> usize;
    /// File `i` (0 is the keeper).
    fn path(&self, i: usize) -> &Path;
    /// Its size in bytes.
    fn size(&self, i: usize) -> u64;
    /// What a person may do with file `i`: the program's own rule decides
    /// `Act` and `Hand`; another shot and a RAW are locked.
    fn role(&self, i: usize) -> Role;
    /// File `i` in words, against the keeper.
    fn words(&self, i: usize) -> String;
    /// File `i` as something to look at.
    fn visual(&self, i: usize) -> Option<Visual>;
    /// Make file `i` the keeper (`0 < i < len`); the others are related to
    /// it afresh.
    fn set_keeper(&mut self, i: usize);
    /// What acting on the group would free, as its header says.
    fn reclaim(&self) -> u64;
}

impl GroupView for Group {
    fn reclaim(&self) -> u64 {
        self.reclaimable()
    }

    fn len(&self) -> usize {
        self.members.len()
    }
    fn path(&self, i: usize) -> &Path {
        &self.members[i].path
    }
    fn size(&self, i: usize) -> u64 {
        self.members[i].size
    }
    fn role(&self, i: usize) -> Role {
        if i == 0 {
            Role::Keeper
        } else {
            Role::Act
        }
    }
    fn words(&self, _: usize) -> String {
        format!("same bytes, {}", fmt_bytes(self.size))
    }
    fn visual(&self, i: usize) -> Option<Visual> {
        let p: PathBuf = self.members.get(i)?.path.clone();
        if has_ext(&p, VIDEO_EXT) {
            Some(Visual::Video(p, 0.0))
        } else if has_ext(&p, IMAGE_EXT) {
            Some(Visual::Image(p))
        } else {
            None
        }
    }
    fn set_keeper(&mut self, i: usize) {
        let m = self.members.remove(i);
        self.members.insert(0, m);
    }
}

impl GroupView for NearGroup {
    fn reclaim(&self) -> u64 {
        self.reclaimable()
    }

    fn len(&self) -> usize {
        self.members.len()
    }
    fn path(&self, i: usize) -> &Path {
        &self.members[i].hashed.rep.cand.path
    }
    fn size(&self, i: usize) -> u64 {
        self.members[i].hashed.rep.cand.size
    }
    fn role(&self, i: usize) -> Role {
        match (i, self.members[i].relation) {
            (0, _) | (_, None) => Role::Keeper,
            (_, Some(r)) if r.kin => Role::Locked(Lock::Kin),
            (_, Some(r)) if r.raw => Role::Locked(Lock::Raw),
            (_, Some(r)) if r.listed_only() => Role::Hand,
            (_, Some(r)) if r.confirmed() => Role::Act,
            _ => Role::Hand,
        }
    }
    fn words(&self, i: usize) -> String {
        let k = &self.members[0].hashed;
        let m = &self.members[i];
        m.relation
            .map(|r| r.words(k, &m.hashed))
            .unwrap_or_default()
    }
    fn visual(&self, i: usize) -> Option<Visual> {
        Some(Visual::Image(
            self.members.get(i)?.hashed.rep.cand.path.clone(),
        ))
    }
    fn set_keeper(&mut self, i: usize) {
        NearGroup::set_keeper(self, i)
    }
}

impl GroupView for VideoGroup {
    fn reclaim(&self) -> u64 {
        self.reclaimable()
    }

    fn len(&self) -> usize {
        self.members.len()
    }
    fn path(&self, i: usize) -> &Path {
        &self.members[i].hashed.rep.cand.path
    }
    fn size(&self, i: usize) -> u64 {
        self.members[i].hashed.rep.cand.size
    }
    fn role(&self, i: usize) -> Role {
        match (i, self.members[i].relation) {
            (0, _) | (_, None) => Role::Keeper,
            (_, Some(r)) if r.confirmed() => Role::Act,
            _ => Role::Hand,
        }
    }
    fn words(&self, i: usize) -> String {
        let k = &self.members[0].hashed;
        let m = &self.members[i];
        m.relation
            .map(|r| r.words(&k.sig, &m.hashed.sig))
            .unwrap_or_default()
    }
    fn visual(&self, i: usize) -> Option<Visual> {
        let m = self.members.get(i)?;
        Some(Visual::Video(
            m.hashed.rep.cand.path.clone(),
            m.hashed.sig.meta.duration,
        ))
    }
    fn set_keeper(&mut self, i: usize) {
        VideoGroup::set_keeper(self, i)
    }
}

impl Found {
    /// Every group in on-screen order: exact, near, video.
    pub(crate) fn views(&self) -> impl Iterator<Item = &dyn GroupView> {
        self.groups
            .iter()
            .map(|g| g as &dyn GroupView)
            .chain(self.near.iter().map(|g| g as &dyn GroupView))
            .chain(self.video.iter().map(|g| g as &dyn GroupView))
    }

    /// On-screen group `i` (0-based).
    pub(crate) fn view(&self, i: usize) -> Option<&dyn GroupView> {
        self.views().nth(i)
    }

    /// On-screen group `i` (0-based), to change.
    pub(crate) fn view_mut(&mut self, i: usize) -> Option<&mut dyn GroupView> {
        self.groups
            .iter_mut()
            .map(|g| g as &mut dyn GroupView)
            .chain(self.near.iter_mut().map(|g| g as &mut dyn GroupView))
            .chain(self.video.iter_mut().map(|g| g as &mut dyn GroupView))
            .nth(i)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exact::Tier;
    use crate::walk::Candidate;

    fn exact(names: &[&str]) -> Group {
        Group {
            size: 10,
            hash: [0; 32],
            tier: Tier::Full,
            members: names
                .iter()
                .map(|n| Candidate {
                    path: PathBuf::from(n),
                    size: 10,
                    mtime: std::time::SystemTime::UNIX_EPOCH,
                })
                .collect(),
        }
    }

    #[test]
    fn views_follow_the_screen_and_keepers_move() {
        let mut f = Found {
            groups: vec![
                exact(&["a.jpg", "b.jpg"]),
                exact(&["c.mp4", "d.mp4", "e.mp4"]),
            ],
            near: Vec::new(),
            scan: Default::default(),
            pipe: Default::default(),
            images_seen: 0,
            video: Vec::new(),
            videos_seen: 0,
            video_tools_missing: false,
            stalled: Vec::new(),
        };
        assert_eq!(f.group_count(), 2);
        let order: Vec<&Path> = f.views().map(|g| g.path(0)).collect();
        assert_eq!(order, [Path::new("a.jpg"), Path::new("c.mp4")]);
        let g = f.view(1).unwrap();
        assert_eq!(
            (g.len(), g.role(0), g.role(2)),
            (3, Role::Keeper, Role::Act)
        );
        assert_eq!(g.words(1), "same bytes, 10 B");
        assert!(matches!(g.visual(1), Some(Visual::Video(..))));
        assert!(f.view(2).is_none());
        // through the screen's numbering, 1-based, as the workspace asks
        assert!(f.set_keeper(2, 2));
        assert_eq!(f.view(1).unwrap().path(0), Path::new("e.mp4"));
        assert!(!f.set_keeper(2, 0) && !f.set_keeper(2, 3) && !f.set_keeper(3, 1));
        assert!(!f.set_keeper(0, 1));
    }
}
