//! Diagnostic: relations between every pair of videos in a directory.
use dedupe::video::*;
use std::time::Instant;
fn main() {
    let dir = std::env::args().nth(1).unwrap();
    let mut paths: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .collect();
    paths.sort();
    let t = Instant::now();
    let mut sigs = Vec::new();
    for p in &paths {
        let m = probe(p).unwrap();
        let t1 = Instant::now();
        let s = hash_video(p, &m).unwrap();
        println!(
            "{:<16} {}x{} {:.1}s {} fps={:.1} frames={} pos={}  {:.2}s",
            p.file_name().unwrap().to_string_lossy(),
            m.width,
            m.height,
            m.duration,
            m.codec,
            m.fps,
            s.frames,
            s.positional.len(),
            t1.elapsed().as_secs_f64()
        );
        sigs.push(s);
    }
    println!("hashed {} in {:.2}s", sigs.len(), t.elapsed().as_secs_f64());
    for i in 0..sigs.len() {
        for j in i + 1..sigs.len() {
            let r = relate(&sigs[i], &sigs[j]);
            let pos = r
                .pos
                .map(|(d, p, q)| format!("d {d:>2} p {p:>2} q {q:>3}"))
                .unwrap_or("   no positions  ".into());
            println!(
                "{:<14} {:<14} {pos}  {:<9} l1 {:>6.3}  l2 {:>6.3}  similar {}  confirmed {}",
                paths[i].file_name().unwrap().to_string_lossy(),
                paths[j].file_name().unwrap().to_string_lossy(),
                r.variant.label(),
                r.level1,
                r.level2,
                r.similar(),
                r.confirmed()
            );
        }
    }
}
