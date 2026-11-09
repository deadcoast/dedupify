//! Decode each file given through `decode::load` and report the route,
//! the upright size, the capture moment and the time taken; with
//! `--png DIR`, also write each picture (long side 320) for a look.
//!
//! cargo run --release --example formats_probe -- [--png DIR] FILE...
use std::path::PathBuf;
use std::time::Instant;

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut png: Option<PathBuf> = None;
    if args.first().map(|a| a == "--png").unwrap_or(false) {
        png = Some(PathBuf::from(args.remove(1)));
        args.remove(0);
    }
    for a in args {
        let p = PathBuf::from(&a);
        let name = p.file_name().unwrap().to_string_lossy().to_string();
        let t = Instant::now();
        match dedupe::decode::load(&p) {
            Ok(d) => {
                let ms = t.elapsed().as_millis();
                let cap = match &d.capture {
                    Some(Some(c)) => c.display(),
                    Some(None) => "-".to_string(),
                    None => dedupe::capture::read(&p)
                        .map(|c| c.display())
                        .unwrap_or("-".into()),
                };
                println!(
                    "{:<44} {:<12} {:>5}x{:<5} {:<23} {:>5} ms",
                    name,
                    d.route.label().unwrap_or("image"),
                    d.img.width(),
                    d.img.height(),
                    cap,
                    ms
                );
                if let Some(dir) = &png {
                    let s = d.img.thumbnail(320, 320);
                    let _ = s.to_rgb8().save(dir.join(format!("{name}.png")));
                }
            }
            Err(e) => println!("{:<44} FAILED  {e}", name),
        }
    }
}
