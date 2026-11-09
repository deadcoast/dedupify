//! PDQ conformance against Meta's reference regression outputs.
//!
//! `cargo run --release --example pdq_ref -- <ThreatExchange>/pdq`
//!
//! Hashes the regression inputs (`data/reg-test-input/dih/*.jpg`) with
//! this crate and compares, bit for bit, with the hashes the reference
//! `pdq-photo-hasher` recorded in `cpp/reg_test/expected/out` — both the
//! eight files and the eight dihedral variants the reference derives from
//! `bridge-1-original.jpg`. Two inputs are tried for each file: the luma
//! the shipped pipeline feeds PDQ (box-prefiltered so the short side lands
//! in [576, 1152)) and the full-resolution luma the reference uses.
use std::path::Path;

use dedupe::pdq::{self, Hash256, Luma};
use dedupe::perceptual::{prefiltered_luma, Variant, VARIANT_ORDER};

/// The reference prints sixteen 16-bit words, w[15] first; word r holds
/// DCT row r with column j at bit j (LSB first). Our `Hash256` keeps bit
/// i·16+j of the row-major 16×16 at position 63 − (b mod 64) of word b/64.
/// Convert the reference's text into our layout so distances compare.
fn parse_hex(s: &str) -> Hash256 {
    let mut h = [0u64; 4];
    for r in 0..16 {
        let chunk = &s[(15 - r) * 4..(15 - r) * 4 + 4];
        let w = u16::from_str_radix(chunk, 16).unwrap();
        for j in 0..16 {
            if (w >> j) & 1 == 1 {
                let b = r * 16 + j;
                h[b / 64] |= 1u64 << (63 - (b % 64));
            }
        }
    }
    h
}

fn hamming(a: &Hash256, b: &Hash256) -> u32 {
    a.iter().zip(b).map(|(x, y)| (x ^ y).count_ones()).sum()
}

/// Full-resolution Rec. 601 luma as the reference computes it
/// (`fillFloatLumaFromRGB`: 0.299 R + 0.587 G + 0.114 B, no rounding).
fn full_luma(img: &image::DynamicImage) -> (Vec<f32>, usize, usize) {
    let rgb = img.to_rgb8();
    let (w, h) = (rgb.width() as usize, rgb.height() as usize);
    let raw = rgb.as_raw();
    let data = (0..w * h)
        .map(|i| {
            0.299 * raw[i * 3] as f32
                + 0.587 * raw[i * 3 + 1] as f32
                + 0.114 * raw[i * 3 + 2] as f32
        })
        .collect();
    (data, w, h)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let root = Path::new(&args[1]);
    let expected = std::fs::read_to_string(root.join("cpp/reg_test/expected/out")).unwrap();
    // file hashes: "<hex>,<quality>,<path>" with distinct paths (lines 8–15)
    let mut file_hashes: Vec<(String, Hash256, u32)> = Vec::new();
    // the eight dihedral hashes of bridge-1 (lines 42–49, same path each)
    let mut dih_hashes: Vec<Hash256> = Vec::new();
    let mut seen_first = false;
    for line in expected.lines() {
        let parts: Vec<&str> = line.split(',').collect();
        if parts.len() == 3
            && parts[0].len() == 64
            && parts[0].chars().all(|c| c.is_ascii_hexdigit())
        {
            let name = parts[2].rsplit('/').next().unwrap().to_string();
            let q: u32 = parts[1].parse().unwrap();
            if name == "bridge-1-original.jpg" {
                if seen_first {
                    // the second block of bridge-1 lines is the dihedral set
                    dih_hashes.push(parse_hex(parts[0]));
                } else {
                    seen_first = true;
                    file_hashes.push((name, parse_hex(parts[0]), q));
                }
            } else if !file_hashes.iter().any(|(n, _, _)| *n == name) {
                file_hashes.push((name, parse_hex(parts[0]), q));
            }
        }
    }
    // the dihedral block starts with the identity hash again; keep 8
    if dih_hashes.len() > 8 {
        dih_hashes = dih_hashes[dih_hashes.len() - 8..].to_vec();
    }
    println!(
        "{} file hashes, {} dihedral hashes in the expected output",
        file_hashes.len(),
        dih_hashes.len()
    );
    println!();
    println!(
        "{:<28} {:>7} {:>7}   {:>8} {:>8} {:>8}",
        "file", "ref q", "our q", "d(pre)", "d(full)", "d(512)"
    );
    let mut worst_pre = 0;
    let mut worst_full = 0;
    for (name, ref_hash, ref_q) in &file_hashes {
        // DEDUPE_PDQ_PNG_DIR: the same images decoded by libjpeg (PIL) and
        // saved losslessly, to separate decoder differences from ours
        let path = match std::env::var("DEDUPE_PDQ_PNG_DIR") {
            Ok(d) => Path::new(&d).join(name.replace(".jpg", ".png")),
            Err(_) => root.join("data/reg-test-input/dih").join(name),
        };
        let img = image::open(&path).unwrap();
        // (a) the shipped pipeline's input
        let (pre, pw, ph) = prefiltered_luma(&img);
        let a = pdq::compute(&Luma {
            data: &pre,
            width: pw,
            height: ph,
        });
        // (b) full-resolution luma
        let (full, fw, fh) = full_luma(&img);
        let b = pdq::compute(&Luma {
            data: &full,
            width: fw,
            height: fh,
        });
        let da = hamming(&a.hashes[0], ref_hash);
        let db = hamming(&b.hashes[0], ref_hash);
        // (c) what the reference *tool* does before hashing: nearest-
        // neighbour squash to 512×512 when either side exceeds 512 (CImg
        // resize, interpolation 1: source index floor(x·W/512)), then luma
        let c = pdq::compute_tool(&img);
        let dc = hamming(&c.hashes[0], ref_hash);
        worst_pre = worst_pre.max(da);
        worst_full = worst_full.max(db);
        println!(
            "{name:<28} {ref_q:>7} {:>7}   {da:>8} {db:>8} {dc:>8}",
            c.quality
        );
    }
    println!();
    println!("worst distance to the reference: prefiltered {worst_pre} bits, full-resolution {worst_full} bits (of 256); third column: the tool's 512 squash");

    // dihedral variants of bridge-1: our DCT-domain variants vs the
    // reference's eight, in the reference's order
    // (orig, rot90, rot180, rot270, flipx, flipy, flip+1, flip-1), on the
    // tool's input so the only difference left is the variant derivation
    let img = image::open(root.join("data/reg-test-input/dih/bridge-1-original.jpg")).unwrap();
    let ours = pdq::compute_tool(&img);
    println!();
    println!("dihedral variants of bridge-1 (our variant hash, tool input, vs the reference's derived hash):");
    for (k, v) in VARIANT_ORDER.iter().enumerate() {
        let best = dih_hashes
            .iter()
            .enumerate()
            .map(|(i, h)| (hamming(&ours.hashes[k], h), i))
            .min()
            .unwrap();
        println!(
            "  {:<12} closest reference variant #{}  distance {} bits",
            format!("{v:?}"),
            best.1,
            best.0
        );
    }
    let _ = Variant::Identity;
}
