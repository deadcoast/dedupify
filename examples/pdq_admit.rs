//! Diagnostic (Phase 19): among every pair of the pictures under the given
//! folders, the pairs PDQ alone admits (the 64-bit pair rule fails, PDQ
//! within PDQ_MAX on reliable hashes), and of those the ones the fit
//! confirms — the pairs the PDQ admission would act on.
use dedupe::near::relation;
use dedupe::perceptual::signature;
use rayon::prelude::*;

fn main() {
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    for d in std::env::args().skip(1) {
        for e in walkdir::WalkDir::new(d).into_iter().flatten() {
            let p = e.path();
            let ext = p
                .extension()
                .map(|x| x.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            if e.file_type().is_file()
                && ["jpg", "jpeg", "png", "tif", "bmp", "webp"].contains(&ext.as_str())
            {
                files.push(p.to_path_buf());
            }
        }
    }
    let sigs: Vec<_> = files.par_iter().map(|p| signature(p).ok()).collect();
    let n = files.len();
    let mut admitted = Vec::new();
    let mut hashed = 0usize;
    for i in 0..n {
        for j in i + 1..n {
            let (Some(a), Some(b)) = (&sigs[i], &sigs[j]) else {
                continue;
            };
            let r = relation(a, b);
            if r.hash_pair() {
                hashed += 1;
            } else if r.pdq_pair() {
                admitted.push((i, j, r));
            }
        }
    }
    println!(
        "{n} pictures, {} pairs: {hashed} by the 64-bit pair rule, {} by PDQ alone",
        n * (n - 1) / 2,
        admitted.len()
    );
    for (i, j, r) in admitted {
        let fit = dedupe::geometry::thumb_of(&files[i]).and_then(|a| {
            dedupe::geometry::thumb_of(&files[j]).map(|b| dedupe::geometry::fit(&a, &b))
        });
        println!(
            "  {:<40} {:<40} d {:>2} p {:>2} q {:>2}  fit {}",
            files[i].file_name().unwrap().to_string_lossy(),
            files[j].file_name().unwrap().to_string_lossy(),
            r.d,
            r.p,
            r.q,
            fit.map(|f| format!(
                "{:.2}{}",
                f.score().max(0.0),
                if f.confirmed() { " confirmed" } else { "" }
            ))
            .unwrap_or("-".into())
        );
    }
}
