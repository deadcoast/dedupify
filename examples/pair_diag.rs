//! Diagnostic: the relation of each `a b` pair of files on the command
//! line, as the near tier computes it (d, p, q, fit, quality).
use dedupe::near::relation;
use dedupe::perceptual::signature;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    for pair in args.chunks(2) {
        let (a, b) = (&pair[0], &pair[1]);
        let ka = signature(std::path::Path::new(a)).unwrap();
        let kb = signature(std::path::Path::new(b)).unwrap();
        let r = relation(&ka, &kb);
        let fit = dedupe::geometry::thumb_of(std::path::Path::new(a)).and_then(|ta| {
            dedupe::geometry::thumb_of(std::path::Path::new(b))
                .map(|tb| dedupe::geometry::fit(&ta, &tb))
        });
        println!(
            "{:<28} {:<28} d {:>2} p {:>2} q {:>3}  qa {:>3} qb {:>3}  pdqq {:>3}/{:>3}  fit {:?}  pair {} confirmed {}",
            std::path::Path::new(a).file_name().unwrap().to_string_lossy(),
            std::path::Path::new(b).file_name().unwrap().to_string_lossy(),
            r.d, r.p, r.q, ka.quality, kb.quality, ka.pdq_quality, kb.pdq_quality,
            fit.map(|f| ((f.ncc * 100.0).round() / 100.0, (f.scale * 100.0).round() / 100.0)), r.hash_pair(), r.confirmed()
        );
    }
}
