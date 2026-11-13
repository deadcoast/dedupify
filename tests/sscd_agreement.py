"""TECH-14 (Phase 15): SSCD's ONNX export against Meta's TorchScript
original, and the deep tier (tract, `ours.tsv`) against ONNX Runtime.

    python -I tests/sscd_agreement.py <folder> <onnx> <torchscript>

<folder> holds the 200 pictures (320 x 320 PNG) and `ours.tsv` that
`tests/sscd_agreement.rs` wrote. Each picture becomes one tensor —
RGB / 255, ImageNet mean and std, NCHW — fed to both models; every
embedding is L2-normalised. Printed (and added to the job's summary):
per picture, the cosine between engines; per pair (a photograph and its
copy), how far the engines' pair cosines differ, which is what the deep
tier's thresholds read.
"""

import os
import sys

import numpy as np
import onnxruntime as ort
import torch
from PIL import Image

MEAN = np.array([0.485, 0.456, 0.406], dtype=np.float32)
STD = np.array([0.229, 0.224, 0.225], dtype=np.float32)


def tensor(path):
    a = np.asarray(Image.open(path).convert("RGB"), dtype=np.float32) / 255.0
    a = (a - MEAN) / STD
    return np.ascontiguousarray(a.transpose(2, 0, 1)[None])


def unit(v):
    v = np.asarray(v, dtype=np.float64).reshape(-1)
    return v / np.linalg.norm(v)


def main():
    folder, onnx_path, ts_path = sys.argv[1:4]
    ours = {}
    with open(os.path.join(folder, "ours.tsv")) as f:
        for line in f:
            name, *vals = line.rstrip("\n").split("\t")
            ours[name] = unit([float(x) for x in vals])
    names = sorted(ours)
    sess = ort.InferenceSession(onnx_path, providers=["CPUExecutionProvider"])
    inp = sess.get_inputs()[0].name
    ts = torch.jit.load(ts_path, map_location="cpu").eval()
    onnx_e, ts_e = {}, {}
    with torch.no_grad():
        for n in names:
            t = tensor(os.path.join(folder, n + ".png"))
            onnx_e[n] = unit(sess.run(None, {inp: t})[0])
            ts_e[n] = unit(ts(torch.from_numpy(t)).numpy())

    def cos(a, b):
        return float(np.dot(a, b))

    rows = []
    for label, a, b in [
        ("ONNX export vs TorchScript (ONNX Runtime)", onnx_e, ts_e),
        ("ours (tract) vs ONNX Runtime, same export", ours, onnx_e),
        ("ours (tract) vs TorchScript", ours, ts_e),
    ]:
        c = np.array([cos(a[n], b[n]) for n in names])
        pairs = [n[1:] for n in names if n.startswith("p")]
        d = np.array(
            [
                abs(cos(a["p" + k], a["v" + k]) - cos(b["p" + k], b["v" + k]))
                for k in pairs
            ]
        )
        rows.append(
            f"| {label} | {c.min():.6f} | {np.percentile(c, 1):.6f} | {np.median(c):.6f}"
            f" | {d.max():.6f} | {np.median(d):.6f} |"
        )
    head = (
        f"SSCD agreement on {len(names)} pictures (320 x 320, the same tensor to every engine)\n\n"
        "| engines | cosine min | p1 | median | pair-cosine difference max | median |\n"
        "|---|---|---|---|---|---|"
    )
    text = head + "\n" + "\n".join(rows) + "\n"
    print(text)
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a") as f:
            f.write(text)


if __name__ == "__main__":
    main()
