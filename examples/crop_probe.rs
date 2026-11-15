//! Diagnostic (Phase 19): how far a centred crop's PDQ is from the PDQ of
//! the same window of its original, when the window is not exactly the
//! crop — the tolerance that sets how many windows a picture needs.
//! Both sides hashed from the 288-cell grid.
use dedupe::pdq::{compute_plain, distance, Luma};
use dedupe::perceptual::{grid_from_image, GRID};

fn window(grid: &[f32], fx: f32, fy: f32) -> Vec<f32> {
    let w = (GRID as f32 * fx).round() as usize;
    let h = (GRID as f32 * fy).round() as usize;
    let (x0, y0) = ((GRID - w) / 2, (GRID - h) / 2);
    let mut v = Vec::with_capacity(w * h);
    for y in y0..y0 + h {
        v.extend_from_slice(&grid[y * GRID + x0..y * GRID + x0 + w]);
    }
    v
}

fn pdq_of(v: &[f32], w: usize, h: usize) -> [u64; 4] {
    compute_plain(&Luma {
        data: v,
        width: w,
        height: h,
    })
    .hashes[0]
}

fn main() {
    let files: Vec<String> = std::env::args().skip(1).collect();
    let windows: Vec<f32> = (0..=12).map(|i| 0.95 - i as f32 * 0.05).collect();
    println!(
        "crop   distance to the window at each zoom (rows: crops; columns: {:?})",
        windows
    );
    for z10 in [90, 85, 80, 75, 70, 67, 65, 60, 55, 50, 45] {
        let z = z10 as f32 / 100.0;
        let mut best_per_window = vec![0u32; windows.len()];
        let mut n = 0;
        for f in &files {
            let img = image::open(f).unwrap();
            let (w, h) = (img.width(), img.height());
            let cw = (w as f32 * z).round() as u32;
            let ch = (h as f32 * z).round() as u32;
            let crop = img.crop_imm((w - cw) / 2, (h - ch) / 2, cw, ch);
            let g = grid_from_image(&crop);
            let full = pdq_of(&g, GRID, GRID);
            let og = grid_from_image(&img);
            for (k, &wz) in windows.iter().enumerate() {
                let win = window(&og, wz, wz);
                let ws = (GRID as f32 * wz).round() as usize;
                let d = distance(&full, &pdq_of(&win, ws, ws));
                best_per_window[k] = best_per_window[k].max(d);
            }
            n += 1;
        }
        let row: Vec<String> = best_per_window.iter().map(|d| format!("{d:>3}")).collect();
        println!("{z:.2} ({n})  max {}", row.join(" "));
    }
}
