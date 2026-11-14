//! Pictures for the workspace's comparison panel (Phase 17, OUT-06)
//! [CLAUDE]: decoded off the key loop by two workers, the newest request
//! first (holding a key down asks for many; the one the cursor stops on is
//! decoded next), each kept for the sitting at the size it was asked for.
//! A slot says whether its picture is still coming, came (and when, so it
//! can fade in), or could not be read.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use image::RgbImage;

use crate::preview::{self, Visual};

/// One picture's state.
#[derive(Debug, Clone)]
pub enum Thumb {
    /// Asked for at this moment; a worker has it or will.
    Coming(Instant),
    /// Decoded and scaled, and when it arrived.
    Ready(Arc<RgbImage>, Instant),
    /// The file did not decode (or a video without ffmpeg).
    Unreadable,
}

/// A picture at a size: the panel asks again when it is resized.
type Key = (PathBuf, u32, u32);

/// Requests kept waiting: older ones are dropped (their slots cleared, so
/// coming back to them asks again).
const QUEUE: usize = 8;

struct Shared {
    slots: Mutex<HashMap<Key, Thumb>>,
    queue: Mutex<Vec<(Key, Visual)>>,
    wake: Condvar,
}

pub struct Thumbs {
    shared: Arc<Shared>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Thumbs {
    pub fn new() -> Thumbs {
        let shared = Arc::new(Shared {
            slots: Mutex::new(HashMap::new()),
            queue: Mutex::new(Vec::new()),
            wake: Condvar::new(),
        });
        for n in 0..2 {
            let s = Arc::clone(&shared);
            let _ = std::thread::Builder::new()
                .name(format!("thumb{n}"))
                .spawn(move || work(&s));
        }
        Thumbs { shared }
    }

    /// The picture of `v` fitted to `w` × `h` pixels: as it is, or asked
    /// for now.
    pub fn get(&self, v: &Visual, w: u32, h: u32) -> Thumb {
        let key = (v.path().to_path_buf(), w, h);
        let now = Instant::now();
        {
            let mut slots = lock(&self.shared.slots);
            if let Some(t) = slots.get(&key) {
                return t.clone();
            }
            slots.insert(key.clone(), Thumb::Coming(now));
        }
        let dropped: Vec<Key> = {
            let mut q = lock(&self.shared.queue);
            q.push((key, v.clone()));
            let over = q.len().saturating_sub(QUEUE);
            q.drain(..over).map(|j| j.0).collect()
        };
        if !dropped.is_empty() {
            let mut slots = lock(&self.shared.slots);
            for k in dropped {
                slots.remove(&k);
            }
        }
        self.shared.wake.notify_one();
        Thumb::Coming(now)
    }

    /// A picture still coming, or one that arrived within `fade`: the
    /// panel is moving and is drawn again on the next frame.
    pub fn moving(&self, fade: Duration) -> bool {
        lock(&self.shared.slots).values().any(|t| match t {
            Thumb::Coming(_) => true,
            Thumb::Ready(_, at) => at.elapsed() < fade,
            Thumb::Unreadable => false,
        })
    }
}

impl Default for Thumbs {
    fn default() -> Self {
        Thumbs::new()
    }
}

fn work(s: &Shared) {
    loop {
        let (key, visual) = {
            let mut q = lock(&s.queue);
            loop {
                if let Some(job) = q.pop() {
                    break job;
                }
                q = s.wake.wait(q).unwrap_or_else(|e| e.into_inner());
            }
        };
        // dropped from the queue while waiting, or already decoded
        if !matches!(lock(&s.slots).get(&key), Some(Thumb::Coming(_))) {
            continue;
        }
        let thumb = match visual.load() {
            Some(img) => Thumb::Ready(
                Arc::new(preview::fit_pixels(&img, key.1, key.2)),
                Instant::now(),
            ),
            None => Thumb::Unreadable,
        };
        lock(&s.slots).insert(key, thumb);
    }
}
