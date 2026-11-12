//! The deep tier's throughput (Phase 14, TECH-15): pictures per second on
//! one thread at batch 1, 2, 4 and 8, and how far a batched embedding is
//! from the single-picture one. The export writes its batch into the
//! graph (a reshape to `[1, 2048, 100]` before the pooling); the measure
//! rewrites that constant. Here (2 cores): batching is bit-identical and
//! slower per picture, so `deep` runs one picture per inference, one
//! inference per thread (VERIFICATION § Phase 14).
//!
//! ```text
//! DEDUPE_MODEL=<sscd_disc_mixup.onnx> cargo test --release --test deep_batch -- --ignored --nocapture
//! ```
#![cfg(feature = "deep")]

mod common;
use common::*;

use std::time::Instant;
use tract_onnx::prelude::*;

fn plan(path: &std::path::Path, b: usize) -> std::sync::Arc<TypedRunnableModel> {
    let mut m = tract_onnx::onnx().model_for_path(path).unwrap();
    // the export reshapes to [1, 2048, 100] before GeM pooling: the batch
    // is written into the graph
    let id = m.node_by_name("/embeddings/0/Constant").unwrap().id;
    m.node_mut(id).op = Box::new(
        tract_onnx::tract_hir::ops::konst::Const::new(
            tensor1(&[b as i64, 2048, 100]).into_arc_tensor(),
        )
        .unwrap(),
    );
    m.set_outlet_fact(OutletId::new(id, 0), InferenceFact::default())
        .unwrap();
    m.with_input_fact(
        0,
        f32::fact([b, 3, dedupe::deep::SIZE, dedupe::deep::SIZE]).into(),
    )
    .unwrap()
    .into_optimized()
    .unwrap()
    .into_runnable()
    .unwrap()
}

fn batch(ts: &[Tensor]) -> Tensor {
    let views: Vec<_> = ts
        .iter()
        .map(|t| t.to_plain_array_view::<f32>().unwrap())
        .collect();
    tract_ndarray::concatenate(tract_ndarray::Axis(0), &views)
        .unwrap()
        .into()
}

#[test]
#[ignore]
fn batched_against_single() {
    let Some(path) = dedupe::deep::installed() else {
        eprintln!("no model");
        return;
    };
    let n = 16;
    let inputs: Vec<Tensor> = (0..n)
        .map(|s| dedupe::deep::Deep::preprocess(&image::DynamicImage::ImageRgb8(photo(s))))
        .collect();
    let single = plan(&path, 1);
    let reference: Vec<Vec<f32>> = inputs
        .iter()
        .map(|t| {
            let o = single.run(tvec!(t.clone().into())).unwrap();
            o[0].to_plain_array_view::<f32>()
                .unwrap()
                .iter()
                .copied()
                .collect()
        })
        .collect();
    for b in [1usize, 2, 4, 8] {
        let p = plan(&path, b);
        // warm
        let _ = p.run(tvec!(batch(&inputs[..b]).into())).unwrap();
        let t = Instant::now();
        let mut diff = 0f32;
        for (k, chunk) in inputs.chunks(b).enumerate() {
            let o = p.run(tvec!(batch(chunk).into())).unwrap();
            let v = o[0].to_plain_array_view::<f32>().unwrap();
            for (j, row) in v.outer_iter().enumerate() {
                for (x, y) in row.iter().zip(&reference[k * b + j]) {
                    diff = diff.max((x - y).abs());
                }
            }
        }
        let secs = t.elapsed().as_secs_f64();
        eprintln!(
            "batch {b}: {:.2} pictures a second on one thread   largest difference {diff:e}",
            n as f64 / secs
        );
    }
}
