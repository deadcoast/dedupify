//! The deep tier's band (Phase 11, CORE-04): under `DEEP_MIN`, down to
//! `DEEP_FIT_MIN`, a picture the descriptor alone brings in stays only
//! when the geometric fit confirms it — and then it is a confirmed copy —
//! and a band attachment never takes a picture away from a pair at
//! `DEEP_MIN`, which lists. The embeddings are set by hand (unit vectors
//! at chosen cosines), so this runs without the model, in both builds.

mod common;
use common::*;

use std::path::Path;

use image::DynamicImage;

use dedupe::deep::{cosine, Embedding, DEEP_MIN, DIM};
use dedupe::exact::NoProgress;
use dedupe::near::{group, Hashed, NearGroup, NearStats, Rep};
use dedupe::perceptual::signature_from_image;
use dedupe::walk::Candidate;

/// A unit vector from (axis, weight) parts whose squares sum to one.
fn unit(parts: &[(usize, f32)]) -> Embedding {
    let mut e = [0.0f32; DIM];
    for &(i, v) in parts {
        e[i] = v;
    }
    let n: f32 = e.iter().map(|v| v * v).sum::<f32>().sqrt();
    for v in &mut e {
        *v /= n;
    }
    e
}

fn hashed(dir: &Path, name: &str, img: &image::RgbImage, emb: Embedding) -> Hashed {
    let path = dir.join(name);
    img.save(&path).unwrap();
    let meta = std::fs::metadata(&path).unwrap();
    Hashed {
        rep: Rep {
            cand: Candidate {
                path: path.clone(),
                size: meta.len(),
                mtime: meta.modified().unwrap(),
            },
            exact_group: None,
        },
        sig: signature_from_image(&DynamicImage::ImageRgb8(img.clone())),
        deep: Some(emb),
        capture: None,
        raw: false,
    }
}

fn holding<'a>(groups: &'a [NearGroup], name: &str) -> Option<&'a NearGroup> {
    groups.iter().find(|g| {
        g.members
            .iter()
            .any(|m| m.hashed.rep.cand.path.file_name().unwrap() == name)
    })
}

#[test]
fn the_band_admits_with_the_fit_and_never_takes_a_listing_pair() {
    let d = tempfile::tempdir().unwrap();
    let a = photo(11);
    let z = (1.0f32 - 0.62 * 0.62 - 0.75 * 0.75).sqrt();
    let set = vec![
        // a and its half-size copy: grouped by the hashes, a keeps
        hashed(d.path(), "a.png", &a, unit(&[(0, 1.0)])),
        hashed(d.path(), "a_half.png", &scale(&a, 0.5), unit(&[(0, 1.0)])),
        // a crop to half the area: the hashes do not reach it; the
        // descriptor in the band, the fit confirms
        hashed(
            d.path(),
            "a_crop.png",
            &crop_center(&a, 0.7),
            unit(&[(0, 0.65), (1, (1.0f32 - 0.65 * 0.65).sqrt())]),
        ),
        // another picture, the descriptor in the band, the fit refuses
        hashed(
            d.path(),
            "b.png",
            &photo(12),
            unit(&[(0, 0.65), (2, (1.0f32 - 0.65 * 0.65).sqrt())]),
        ),
        // s is in a's band, and at 0.75 with t: the pair lists, s stays with t
        hashed(
            d.path(),
            "s.png",
            &photo(13),
            unit(&[(0, 0.62), (3, 0.75), (4, z)]),
        ),
        hashed(d.path(), "t.png", &photo(14), unit(&[(3, 1.0)])),
    ];
    let e = |n: usize| set[n].deep.unwrap();
    for (x, y, want) in [(0, 2, 0.65), (0, 3, 0.65), (0, 4, 0.62), (4, 5, 0.75)] {
        assert!((cosine(&e(x), &e(y)) - want).abs() < 1e-3);
    }

    let mut stats = NearStats {
        deep: true,
        ..Default::default()
    };
    let groups = group(set, &mut stats, &NoProgress);
    let names = |g: &NearGroup| -> Vec<String> {
        g.members
            .iter()
            .map(|m| {
                m.hashed
                    .rep
                    .cand
                    .path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect()
    };

    let ga = holding(&groups, "a.png").expect("a's group");
    assert_eq!(names(ga)[0], "a.png", "{:?}", names(ga));
    let crop = ga
        .members
        .iter()
        .find(|m| m.hashed.rep.cand.path.ends_with("a_crop.png"))
        .unwrap_or_else(|| panic!("the crop is in a's group: {:?}", names(ga)));
    let r = crop.relation.unwrap();
    assert!(
        !r.hash_pair(),
        "the hashes alone would have grouped it: {r:?}"
    );
    assert!(
        r.deep_band() && r.fit_ok() && r.confirmed() && !r.listed_only(),
        "{r:?}"
    );

    // b: the fit refused it in the band, so it is in no group at all
    assert!(
        holding(&groups, "b.png").is_none(),
        "{:?}",
        groups.iter().map(names).collect::<Vec<_>>()
    );

    // s went with t at 0.75, not into a's group at 0.62
    let gs = holding(&groups, "s.png").expect("s's group");
    let mut n = names(gs);
    n.sort();
    assert_eq!(n, ["s.png", "t.png"]);
    let r = gs.members[1].relation.unwrap();
    assert!(
        r.deep.is_some_and(|c| c >= DEEP_MIN) && !r.deep_band(),
        "{r:?}"
    );
    assert_eq!(groups.len(), 2);
}
