# DEEP — the optional learned tier

`dedupe` can run a copy-detection neural network over the images it
scans. It is **off until a model file is present**. There is no flag; the
file is the switch, and `dedupe doctor` reports it.

## What it does

The model is **SSCD** (`sscd_disc_mixup`, Pizzi et al., CVPR 2022): a
ResNet-50 trained on DISC2021 to put edited copies of a picture close
together and unrelated pictures far apart, as a 512-dimensional
L2-normalised embedding compared by cosine. It finds what the hash tiers
miss: heavy crops, small rotations, letterboxes, watermarks.

On the generated corpus (`docs/VERIFICATION.md`, Phase 4) the hash pair
dHash∧pHash caught 70 % of 10 % crops and 2.7 % of 20 % crops; SSCD at the
shipped threshold catches 100 % of both, and 92.5 % of letterboxes against
62 %. Neither catches a 40 % crop (0 % and 2.5 %).

## What it is allowed to do

**List alone, act with the fit.** A member brought into a group by the
deep score alone prints with the verb `deep` instead of `near`, its 64-bit
distances lit in the warning colour and its `deep 0.78` score lit; `milk`
and `purge` leave such members where they are and the totals line says
so. Since Phase 6 every deep-admitted member is also put to the geometric
fit (`docs/RESEARCH.md` §2.7): when the fit confirms it — a letterboxed
copy at `deep 0.78  fit 1.00` — the member is an ordinary `near` and the
commands act on it; when it does not (`forest.png`, `deep 0.76  fit 0.51`)
it stays listed. Two independent families, the learned descriptor and
spatial correlation, have then agreed; the descriptor's word alone never
moves a file. The threshold was first measured on procedural pictures,
where unrelated images score higher than unrelated photographs would;
Phase 11 measured it on real labelled copies (INRIA Copydays, 5.44
million unrelated pairs, none over 0.526; `docs/VERIFICATION.md` § Phase
11). The listing threshold stays 0.70 — photographs of one subject from
another position reach 0.88 — and under it, down to 0.60, the descriptor
brings a member in only when the fit confirms it, and that member acts
(crops to 40 % of the area: 46.5 → 91.7 % acted on). The harnesses are
`cargo test --release --test corpus -- --ignored measure_deep` and
`tests/real_copies.rs`.

## Install

`dedupe --doctor` on a terminal, then enter: when no model is installed
the doctor ends on that offer, fetches the file with curl, and keeps it
only at the published size and SHA-256 (Phase 16). By hand:

Put the ONNX model at (Windows) `%LOCALAPPDATA%\dedupe\models\sscd_disc_mixup.onnx`
or (Linux) `~/.local/share/dedupe/models/sscd_disc_mixup.onnx`, or point
`DEDUPE_MODEL` at any path.

PowerShell:

```powershell
$d = "$env:LOCALAPPDATA\dedupe\models"; New-Item -ItemType Directory -Force $d | Out-Null
Invoke-WebRequest "https://huggingface.co/nutszebra/sscd_onnx/resolve/main/sscd_disc_mixup.onnx" -OutFile "$d\sscd_disc_mixup.onnx"
(Get-FileHash "$d\sscd_disc_mixup.onnx").Hash
```

bash:

```sh
d=~/.local/share/dedupe/models; mkdir -p "$d"
curl -L -o "$d/sscd_disc_mixup.onnx" "https://huggingface.co/nutszebra/sscd_onnx/resolve/main/sscd_disc_mixup.onnx"
sha256sum "$d/sscd_disc_mixup.onnx"
```

Expected SHA-256:
`e65e2a5b9a5e44a0b0f011bddaa6f28621021f6120ca7ce8b526c19f2f6156f8`
(98,156,202 bytes).

## Provenance

The ONNX file is a third-party export (Hugging Face `nutszebra/sscd_onnx`,
revision `0f03b67c…`) of Meta's checkpoint
`https://dl.fbaipublicfiles.com/sscd-copy-detection/sscd_disc_mixup.torchscript.pt`.
The export's metadata records that checkpoint's SHA-256 as
`9f26bd4c848cc19b73d2ae92eea6e04886f61a7b764ceb7a13aeee62e6a6db56`; the
checkpoint was downloaded and hashed during the Phase 4 build and matches
byte for byte. The export's *outputs* agree with the TorchScript
original: on 200 pictures fed as the same tensor to both (Phase 15,
`.github/workflows/sscd.yml`), cosine 1.000000 between the two models'
embeddings and between `dedupe`'s (`tract`) and ONNX Runtime's, every
pair's cosine within 0.000001 (VERIFICATION § Phase 15). SSCD is
MIT-licensed.

## Cost

Inference runs in `tract`, a pure-Rust ONNX engine, so the binary stays a
single file: 25.6 MB with the tier compiled in, 2.9 MB without
(`cargo install --path . --no-default-features`). Embedding one image
took 0.3–0.4 s on the two-core build machine and parallelises across
cores; at that rate a thousand images are five to seven minutes of DECODE
with the model, against nine to sixteen seconds without it (the Phase 2
and 2b timings in `docs/VERIFICATION.md`, same machine).

Preprocessing is the official `skew_320` transform: resize to 320×320
ignoring aspect, RGB in [0, 1], ImageNet mean/std.
