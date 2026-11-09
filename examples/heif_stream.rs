//! Write the elementary stream of a HEIF file's primary picture and print
//! its layout: `heif_stream FILE OUT` (a diagnostic for decode.rs).
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let b = std::fs::read(&a[1]).unwrap();
    let h = dedupe::heif::read(&b).unwrap();
    println!(
        "{:?} tiles {} grid {:?} size {}x{} transforms {:?}",
        h.codec,
        h.tiles.len(),
        h.grid,
        h.width,
        h.height,
        h.transforms
    );
    std::fs::write(&a[2], h.stream()).unwrap();
}
