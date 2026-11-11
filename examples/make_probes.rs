//! Writes the doctor's two probe files, `assets/doctor/probe.heic` and
//! `probe.avif`: a 64×64 picture, red on the left half and blue on the
//! right, as one HEVC item (x265) and one AV1 item (libaom), each in a
//! minimal single-item HEIF. The doctor decodes them through `decode` and
//! checks the two halves, so a PASS means the whole path works.
//!
//! cargo run --release --example make_probes   (needs ffmpeg with libx265
//! and libaom-av1)
use std::path::Path;

fn ffmpeg(args: &[&str]) {
    let st = dedupe::tools::ffmpeg()
        .args(["-hide_banner", "-loglevel", "error", "-y", "-nostdin"])
        .args(args)
        .status()
        .expect("ffmpeg");
    assert!(st.success(), "ffmpeg {args:?}");
}

fn bx(t: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut v = ((body.len() + 8) as u32).to_be_bytes().to_vec();
    v.extend_from_slice(t);
    v.extend_from_slice(body);
    v
}

fn full(t: &[u8; 4], ver: u8, body: &[u8]) -> Vec<u8> {
    let mut b = vec![ver, 0, 0, 0];
    b.extend_from_slice(body);
    bx(t, &b)
}

/// A single-item HEIF: `item_type` with its configuration box and ispe.
fn heif(brand: &[u8; 4], item_type: &[u8; 4], config: Vec<u8>, data: &[u8]) -> Vec<u8> {
    let ftyp = bx(
        b"ftyp",
        &[&brand[..], &[0, 0, 0, 0], b"mif1", &brand[..], b"miaf"].concat(),
    );
    let hdlr = full(b"hdlr", 0, b"\0\0\0\0pict\0\0\0\0\0\0\0\0\0\0\0\0\0");
    let pitm = full(b"pitm", 0, &1u16.to_be_bytes());
    let mut infe = 1u16.to_be_bytes().to_vec();
    infe.extend_from_slice(&[0, 0]);
    infe.extend_from_slice(item_type);
    infe.push(0);
    let iinf = full(
        b"iinf",
        0,
        &[&1u16.to_be_bytes()[..], &full(b"infe", 2, &infe)].concat(),
    );
    let mut ispe = 64u32.to_be_bytes().to_vec();
    ispe.extend_from_slice(&64u32.to_be_bytes());
    let ipco = bx(b"ipco", &[config, full(b"ispe", 0, &ispe)].concat());
    let ipma = full(b"ipma", 0, &[0, 0, 0, 1, 0, 1, 2, 0x81, 0x02]);
    let iprp = bx(b"iprp", &[ipco, ipma].concat());
    let iloc = |off: u32| {
        let mut b = vec![0x44, 0x00, 0, 1, 0, 1, 0, 0, 0, 1];
        b.extend_from_slice(&off.to_be_bytes());
        b.extend_from_slice(&(data.len() as u32).to_be_bytes());
        full(b"iloc", 0, &b)
    };
    let meta = |off: u32| {
        full(
            b"meta",
            0,
            &[
                hdlr.clone(),
                pitm.clone(),
                iloc(off),
                iinf.clone(),
                iprp.clone(),
            ]
            .concat(),
        )
    };
    let off = (ftyp.len() + meta(0).len() + 8) as u32;
    [ftyp, meta(off), bx(b"mdat", data)].concat()
}

fn nals(b: &[u8]) -> Vec<&[u8]> {
    let mut starts = Vec::new();
    let mut i = 0;
    while i + 3 <= b.len() {
        if b[i] == 0 && b[i + 1] == 0 && b[i + 2] == 1 {
            starts.push(i + 3);
            i += 3;
        } else {
            i += 1;
        }
    }
    starts
        .iter()
        .enumerate()
        .map(|(k, &s)| {
            let mut e = starts.get(k + 1).map(|&n| n - 3).unwrap_or(b.len());
            while e > s && b[e - 1] == 0 {
                e -= 1;
            }
            &b[s..e]
        })
        .collect()
}

fn main() {
    let out = Path::new("assets/doctor");
    std::fs::create_dir_all(out).unwrap();
    let tmp = std::env::temp_dir().join("dedupe_probes");
    std::fs::create_dir_all(&tmp).unwrap();
    let png = tmp.join("probe.png");
    let img = image::RgbImage::from_fn(64, 64, |x, _| {
        if x < 32 {
            image::Rgb([220, 40, 40])
        } else {
            image::Rgb([40, 40, 220])
        }
    });
    img.save(&png).unwrap();

    // HEVC: parameter sets into hvcC, the picture's VCL units as the item
    let hevc = tmp.join("probe.hevc");
    ffmpeg(&[
        "-i",
        png.to_str().unwrap(),
        "-c:v",
        "libx265",
        "-x265-params",
        "keyint=1:log-level=none",
        "-pix_fmt",
        "yuv420p",
        "-frames:v",
        "1",
        "-f",
        "hevc",
        hevc.to_str().unwrap(),
    ]);
    let s = std::fs::read(&hevc).unwrap();
    let units = nals(&s);
    let typ = |n: &[u8]| (n[0] >> 1) & 0x3F;
    let get = |t: u8| *units.iter().find(|n| typ(n) == t).unwrap();
    let (vps, sps, pps) = (get(32), get(33), get(34));
    let mut rbsp = Vec::new();
    let mut zeros = 0;
    for &b in &sps[2..] {
        if zeros >= 2 && b == 3 {
            zeros = 0;
            continue;
        }
        zeros = if b == 0 { zeros + 1 } else { 0 };
        rbsp.push(b);
    }
    let mut hvcc = vec![1u8];
    hvcc.extend_from_slice(&rbsp[1..13]);
    hvcc.extend_from_slice(&[0xF0, 0x00, 0xFC, 0xFD, 0xF8, 0xF8, 0x00, 0x00, 0x0F, 3]);
    for (t, n) in [(32u8, vps), (33, sps), (34, pps)] {
        hvcc.push(0x80 | t);
        hvcc.extend_from_slice(&1u16.to_be_bytes());
        hvcc.extend_from_slice(&(n.len() as u16).to_be_bytes());
        hvcc.extend_from_slice(n);
    }
    let mut item = Vec::new();
    for u in units.iter().filter(|n| typ(n) < 32) {
        item.extend_from_slice(&(u.len() as u32).to_be_bytes());
        item.extend_from_slice(u);
    }
    let heic = heif(b"heic", b"hvc1", bx(b"hvcC", &hvcc), &item);
    std::fs::write(out.join("probe.heic"), &heic).unwrap();

    // AV1: the low-overhead OBUs without the temporal delimiter; av1C for
    // profile 0, level 2.0, 8-bit 4:2:0, with the sequence header
    let obu = tmp.join("probe.obu");
    ffmpeg(&[
        "-i",
        png.to_str().unwrap(),
        "-c:v",
        "libaom-av1",
        "-still-picture",
        "1",
        "-pix_fmt",
        "yuv420p",
        "-frames:v",
        "1",
        "-f",
        "obu",
        obu.to_str().unwrap(),
    ]);
    let s = std::fs::read(&obu).unwrap();
    // walk OBUs (each has a size field): drop temporal delimiters (type 2),
    // keep the sequence header (type 1) aside for av1C
    let mut item = Vec::new();
    let mut seq = Vec::new();
    let mut i = 0;
    while i < s.len() {
        let h = s[i];
        let t = (h >> 3) & 15;
        let ext = (h >> 2) & 1;
        let mut j = i + 1 + ext as usize;
        let (mut size, mut shift) = (0usize, 0);
        loop {
            let b = s[j];
            j += 1;
            size |= ((b & 0x7F) as usize) << shift;
            shift += 7;
            if b & 0x80 == 0 {
                break;
            }
        }
        let end = j + size;
        if t == 1 {
            seq = s[i..end].to_vec();
        }
        if t != 2 {
            item.extend_from_slice(&s[i..end]);
        }
        i = end;
    }
    let mut av1c = vec![0x81, 0x00, 0x0C, 0x00];
    av1c.extend_from_slice(&seq);
    let avif = heif(b"avif", b"av01", bx(b"av1C", &av1c), &item);
    std::fs::write(out.join("probe.avif"), &avif).unwrap();
    println!(
        "probe.heic {} bytes, probe.avif {} bytes",
        heic.len(),
        avif.len()
    );
}
