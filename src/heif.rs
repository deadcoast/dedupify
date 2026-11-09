//! HEIF, HEIC and AVIF stills (ISO/IEC 23008-12, MIAF, AVIF): the
//! container is read here; the coded pictures are handed to ffmpeg as an
//! elementary stream by `decode`. [CLAUDE, Phase 10]
//!
//! Why not `ffmpeg -i photo.heic`: ffmpeg's demuxer reads HEIF stills from
//! 7.0 and its command line composes tiled ones from 8.1 (Changelog). A
//! phone photo is tiled — an iPhone stores a 4032×3024 picture as 48 HEVC
//! tiles of 512×512 under a `grid` item — so any ffmpeg before 8.1 returns
//! one tile or nothing. Every ffmpeg since 2013 decodes a raw HEVC stream
//! (and, with libdav1d or libaom, a raw AV1 one), so the container work is
//! done here and the result is the same on every ffmpeg the person has.
//!
//! What is read, from the primary item (`pitm`):
//! - a coded item (`hvc1`, `av01`, `jpeg`), or a `grid` of them (the
//!   tiles are its `dimg` references, row by row), or an `iden` derived
//!   from one;
//! - its decoder configuration (`hvcC` parameter sets, `av1C` OBUs);
//! - its transformative properties in the order they are associated:
//!   `clap` (crop), `irot` (n × 90° anticlockwise), `imir` (mode 0 flips
//!   top to bottom, mode 1 left to right — measured on the AVIF test set's
//!   `kimono.mirror-*` files, RESEARCH §2.9);
//! - (not `colr`: the coded stream's own colour signalling is used; see
//!   `decode::ffmpeg_frames` for the measurement behind that);
//! - the `Exif` item that describes it (`cdsc`), for the capture moment.
//!
//! Every length and offset is checked against the file; a malformed file
//! is an error, never a panic.

use std::ops::Range;

/// The coding of the primary picture's tiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    Hevc,
    Av1,
    Jpeg,
}

impl Codec {
    pub fn name(self) -> &'static str {
        match self {
            Codec::Hevc => "HEIC",
            Codec::Av1 => "AVIF",
            Codec::Jpeg => "HEIF (JPEG)",
        }
    }
}

/// A transformative property, applied in association order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transform {
    /// Clean aperture: the crop rectangle in pixels of the image it
    /// applies to (left, top, width, height).
    Crop {
        left: u32,
        top: u32,
        width: u32,
        height: u32,
    },
    /// `irot`: quarter turns anticlockwise (1–3).
    Rotate(u8),
    /// `imir`: 0 = top-bottom flip, 1 = left-right flip.
    Mirror(u8),
}

/// The primary picture, ready to decode.
#[derive(Debug, Clone)]
pub struct Heif {
    pub codec: Codec,
    /// One entry for a plain item; rows × columns for a grid, row-major.
    pub tiles: Vec<Vec<u8>>,
    /// Per-tile decoder configuration (HEVC parameter sets as Annex B, AV1
    /// configuration OBUs); usually identical across tiles.
    pub configs: Vec<Vec<u8>>,
    /// `(rows, columns, output width, output height)` for a grid.
    pub grid: Option<(u32, u32, u32, u32)>,
    /// Size of the reconstructed image before transforms (`ispe`, or the
    /// grid's output size).
    pub width: u32,
    pub height: u32,
    pub transforms: Vec<Transform>,
    /// Byte range of the TIFF structure of the describing `Exif` item.
    pub exif: Option<Range<usize>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeifError(pub String);

impl std::fmt::Display for HeifError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

fn err<T>(s: &str) -> Result<T, HeifError> {
    Err(HeifError(s.to_string()))
}

/// The brands that mark a HEIF-family still: the ones this reader can
/// handle when the primary item is one of the coded types above.
pub fn is_heif(b: &[u8]) -> bool {
    if b.len() < 12 || &b[4..8] != b"ftyp" {
        return false;
    }
    const BRANDS: [&[u8; 4]; 10] = [
        b"heic", b"heix", b"heim", b"heis", b"hevc", b"hevx", b"mif1", b"msf1", b"avif", b"avis",
    ];
    let size = (be32(b, 0).unwrap_or(0) as usize).min(b.len());
    let mut i = 8;
    while i + 4 <= size {
        if i != 12 && BRANDS.iter().any(|x| &b[i..i + 4] == *x) {
            return true;
        }
        i += 4;
    }
    false
}

// ---------------------------------------------------------------------------
// reading
// ---------------------------------------------------------------------------

fn be16(b: &[u8], i: usize) -> Option<u16> {
    b.get(i..i + 2).map(|s| u16::from_be_bytes([s[0], s[1]]))
}
fn be32(b: &[u8], i: usize) -> Option<u32> {
    b.get(i..i + 4)
        .map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
}
fn be_n(b: &[u8], i: usize, n: usize) -> Option<u64> {
    if n == 0 {
        return Some(0);
    }
    let s = b.get(i..i + n)?;
    Some(s.iter().fold(0u64, |a, &x| (a << 8) | x as u64))
}

/// One box: type, payload range.
#[derive(Debug, Clone)]
struct BoxRef {
    typ: [u8; 4],
    body: Range<usize>,
}

/// The boxes laid end to end in `r`.
fn boxes(b: &[u8], r: Range<usize>) -> Result<Vec<BoxRef>, HeifError> {
    let mut out = Vec::new();
    let mut i = r.start;
    while i + 8 <= r.end {
        let size32 = be32(b, i).ok_or_else(|| HeifError("short box".into()))? as u64;
        let typ: [u8; 4] = b[i + 4..i + 8].try_into().unwrap_or(*b"    ");
        let (size, hdr) = match size32 {
            0 => ((r.end - i) as u64, 8usize),
            1 => (
                be_n(b, i + 8, 8).ok_or_else(|| HeifError("short box".into()))?,
                16,
            ),
            s => (s, 8),
        };
        if size < hdr as u64 || i as u64 + size > r.end as u64 {
            return err("box runs past its container");
        }
        let end = i + size as usize;
        out.push(BoxRef {
            typ,
            body: i + hdr..end,
        });
        i = end;
        if out.len() > 100_000 {
            return err("too many boxes");
        }
    }
    Ok(out)
}

fn find<'a>(v: &'a [BoxRef], t: &[u8; 4]) -> Option<&'a BoxRef> {
    v.iter().find(|x| &x.typ == t)
}

/// Where an item's bytes are: construction method, base offset, and the
/// extents as (offset, length).
type Location = (u8, u64, Vec<(u64, u64)>);

#[derive(Debug, Clone, Default)]
struct Item {
    typ: [u8; 4],
    loc: Option<Location>,
    props: Vec<usize>,
}

struct Meta {
    items: std::collections::HashMap<u32, Item>,
    /// (type, from, to…)
    refs: Vec<([u8; 4], u32, Vec<u32>)>,
    /// ipco children, 1-based in ipma.
    props: Vec<BoxRef>,
    idat: Option<Range<usize>>,
    primary: u32,
}

fn read_meta(b: &[u8]) -> Result<Meta, HeifError> {
    let top = boxes(b, 0..b.len())?;
    let meta = find(&top, b"meta").ok_or_else(|| HeifError("no meta box".into()))?;
    // meta is a FullBox: 4 bytes of version and flags
    if meta.body.len() < 4 {
        return err("short meta");
    }
    let mb = boxes(b, meta.body.start + 4..meta.body.end)?;
    let hdlr = find(&mb, b"hdlr").ok_or_else(|| HeifError("no hdlr".into()))?;
    if b.get(hdlr.body.start + 8..hdlr.body.start + 12) != Some(b"pict") {
        return err("not a picture handler");
    }

    let pitm = find(&mb, b"pitm").ok_or_else(|| HeifError("no primary item".into()))?;
    let v = *b.get(pitm.body.start).unwrap_or(&0);
    let primary = if v == 0 {
        be16(b, pitm.body.start + 4).map(u32::from)
    } else {
        be32(b, pitm.body.start + 4)
    }
    .ok_or_else(|| HeifError("short pitm".into()))?;

    let mut items: std::collections::HashMap<u32, Item> = std::collections::HashMap::new();

    // iinf → infe: id and type
    if let Some(iinf) = find(&mb, b"iinf") {
        let v = *b.get(iinf.body.start).unwrap_or(&0);
        let first = iinf.body.start + if v == 0 { 6 } else { 8 };
        for infe in boxes(b, first.min(iinf.body.end)..iinf.body.end)? {
            if &infe.typ != b"infe" {
                continue;
            }
            let s = infe.body.start;
            let ver = *b.get(s).unwrap_or(&0);
            let (id, typ_at) = match ver {
                2 => (be16(b, s + 4).map(u32::from), s + 8),
                3 => (be32(b, s + 4), s + 10),
                _ => continue, // HEIF requires version 2 or 3
            };
            let Some(id) = id else { continue };
            let Some(t) = b.get(typ_at..typ_at + 4) else {
                continue;
            };
            items.entry(id).or_default().typ = [t[0], t[1], t[2], t[3]];
        }
    }

    // iloc: where each item's bytes are
    if let Some(iloc) = find(&mb, b"iloc") {
        let s = iloc.body.start;
        let v = *b.get(s).ok_or_else(|| HeifError("short iloc".into()))?;
        let sizes = *b.get(s + 4).ok_or_else(|| HeifError("short iloc".into()))?;
        let sizes2 = *b.get(s + 5).ok_or_else(|| HeifError("short iloc".into()))?;
        let (off_sz, len_sz) = ((sizes >> 4) as usize, (sizes & 15) as usize);
        let (base_sz, idx_sz) = (
            (sizes2 >> 4) as usize,
            if v >= 1 { (sizes2 & 15) as usize } else { 0 },
        );
        for n in [off_sz, len_sz, base_sz, idx_sz] {
            if ![0, 4, 8].contains(&n) {
                return err("iloc field size");
            }
        }
        let mut i = s + 6;
        let count = if v < 2 {
            let c = be16(b, i).map(u32::from);
            i += 2;
            c
        } else {
            let c = be32(b, i);
            i += 4;
            c
        }
        .ok_or_else(|| HeifError("short iloc".into()))?;
        for _ in 0..count {
            let id = if v < 2 {
                let x = be16(b, i).map(u32::from);
                i += 2;
                x
            } else {
                let x = be32(b, i);
                i += 4;
                x
            }
            .ok_or_else(|| HeifError("short iloc".into()))?;
            let method = if v >= 1 {
                let x = be16(b, i).ok_or_else(|| HeifError("short iloc".into()))? & 15;
                i += 2;
                x as u8
            } else {
                0
            };
            i += 2; // data_reference_index
            let base = be_n(b, i, base_sz).ok_or_else(|| HeifError("short iloc".into()))?;
            i += base_sz;
            let n = be16(b, i).ok_or_else(|| HeifError("short iloc".into()))?;
            i += 2;
            let mut ext = Vec::with_capacity(n as usize);
            for _ in 0..n {
                i += idx_sz;
                let off = be_n(b, i, off_sz).ok_or_else(|| HeifError("short iloc".into()))?;
                i += off_sz;
                let len = be_n(b, i, len_sz).ok_or_else(|| HeifError("short iloc".into()))?;
                i += len_sz;
                ext.push((off, len));
            }
            items.entry(id).or_default().loc = Some((method, base, ext));
        }
    }

    // iref
    let mut refs = Vec::new();
    if let Some(iref) = find(&mb, b"iref") {
        let v = *b.get(iref.body.start).unwrap_or(&0);
        let w = if v == 0 { 2 } else { 4 };
        for r in boxes(b, iref.body.start + 4..iref.body.end)? {
            let mut i = r.body.start;
            let from = be_n(b, i, w).ok_or_else(|| HeifError("short iref".into()))? as u32;
            i += w;
            let n = be16(b, i).ok_or_else(|| HeifError("short iref".into()))?;
            i += 2;
            let mut to = Vec::with_capacity(n as usize);
            for _ in 0..n {
                to.push(be_n(b, i, w).ok_or_else(|| HeifError("short iref".into()))? as u32);
                i += w;
            }
            if i > r.body.end {
                return err("iref runs past its box");
            }
            refs.push((r.typ, from, to));
        }
    }

    // iprp: ipco (properties), ipma (associations)
    let mut props = Vec::new();
    if let Some(iprp) = find(&mb, b"iprp") {
        let pb = boxes(b, iprp.body.clone())?;
        if let Some(ipco) = find(&pb, b"ipco") {
            props = boxes(b, ipco.body.clone())?;
        }
        for ipma in pb.iter().filter(|x| &x.typ == b"ipma") {
            let s = ipma.body.start;
            let v = *b.get(s).unwrap_or(&0);
            let flags = be32(b, s).unwrap_or(0) & 0xFF_FFFF;
            let mut i = s + 4;
            let n = be32(b, i).ok_or_else(|| HeifError("short ipma".into()))?;
            i += 4;
            for _ in 0..n {
                let id = if v < 1 {
                    let x = be16(b, i).map(u32::from);
                    i += 2;
                    x
                } else {
                    let x = be32(b, i);
                    i += 4;
                    x
                }
                .ok_or_else(|| HeifError("short ipma".into()))?;
                let k = *b.get(i).ok_or_else(|| HeifError("short ipma".into()))?;
                i += 1;
                let mut list = Vec::with_capacity(k as usize);
                for _ in 0..k {
                    let idx = if flags & 1 == 1 {
                        let x = be16(b, i).ok_or_else(|| HeifError("short ipma".into()))?;
                        i += 2;
                        (x & 0x7FFF) as usize
                    } else {
                        let x = *b.get(i).ok_or_else(|| HeifError("short ipma".into()))?;
                        i += 1;
                        (x & 0x7F) as usize
                    };
                    if idx > 0 {
                        list.push(idx);
                    }
                }
                if i > ipma.body.end {
                    return err("ipma runs past its box");
                }
                items.entry(id).or_default().props.extend(list);
            }
        }
    }

    let idat = find(&mb, b"idat").map(|x| x.body.clone());
    Ok(Meta {
        items,
        refs,
        props,
        idat,
        primary,
    })
}

impl Meta {
    fn item(&self, id: u32) -> Result<&Item, HeifError> {
        self.items
            .get(&id)
            .ok_or_else(|| HeifError(format!("item {id} missing")))
    }

    /// The bytes of an item, its extents concatenated.
    fn data(&self, b: &[u8], id: u32) -> Result<Vec<u8>, HeifError> {
        let (method, base, ext) = self
            .item(id)?
            .loc
            .clone()
            .ok_or_else(|| HeifError(format!("item {id} has no location")))?;
        let (origin, limit) = match method {
            0 => (0u64, b.len() as u64),
            1 => {
                let r = self
                    .idat
                    .clone()
                    .ok_or_else(|| HeifError("idat missing".into()))?;
                (r.start as u64, r.end as u64)
            }
            _ => return err("item construction method"),
        };
        let mut out = Vec::new();
        for (off, len) in ext {
            let start = origin
                .checked_add(base)
                .and_then(|x| x.checked_add(off))
                .ok_or_else(|| HeifError("extent overflow".into()))?;
            // a zero length means "to the end of the file" (single extent)
            let end = if len == 0 {
                limit
            } else {
                start
                    .checked_add(len)
                    .ok_or_else(|| HeifError("extent overflow".into()))?
            };
            if end > limit || start > end {
                return err("extent outside the file");
            }
            out.extend_from_slice(&b[start as usize..end as usize]);
            if out.len() > 512 << 20 {
                return err("item too large");
            }
        }
        Ok(out)
    }

    fn prop<'a>(&'a self, id: u32, t: &[u8; 4]) -> Option<&'a BoxRef> {
        let it = self.items.get(&id)?;
        it.props
            .iter()
            .filter_map(|&i| self.props.get(i - 1))
            .find(|p| &p.typ == t)
    }

    fn refs_from(&self, t: &[u8; 4], from: u32) -> Option<&Vec<u32>> {
        self.refs
            .iter()
            .find(|(ty, f, _)| ty == t && *f == from)
            .map(|(_, _, to)| to)
    }
}

/// `ispe` of an item.
fn ispe(b: &[u8], m: &Meta, id: u32) -> Option<(u32, u32)> {
    let p = m.prop(id, b"ispe")?;
    Some((be32(b, p.body.start + 4)?, be32(b, p.body.start + 8)?))
}

/// HEVC parameter sets of an `hvcC`, as Annex B, and the NAL length size.
fn hvcc(b: &[u8], p: &BoxRef) -> Result<(Vec<u8>, usize), HeifError> {
    let s = p.body.start;
    let len_size = (*b
        .get(s + 21)
        .ok_or_else(|| HeifError("short hvcC".into()))?
        & 3) as usize
        + 1;
    let arrays = *b
        .get(s + 22)
        .ok_or_else(|| HeifError("short hvcC".into()))?;
    let mut i = s + 23;
    let mut out = Vec::new();
    for _ in 0..arrays {
        i += 1; // completeness, NAL type
        let n = be16(b, i).ok_or_else(|| HeifError("short hvcC".into()))?;
        i += 2;
        for _ in 0..n {
            let l = be16(b, i).ok_or_else(|| HeifError("short hvcC".into()))? as usize;
            i += 2;
            let nal = b
                .get(i..i + l)
                .filter(|_| i + l <= p.body.end)
                .ok_or_else(|| HeifError("short hvcC".into()))?;
            out.extend_from_slice(&[0, 0, 0, 1]);
            out.extend_from_slice(nal);
            i += l;
        }
    }
    Ok((out, len_size))
}

/// Length-prefixed NAL units → Annex B.
fn annex_b(data: &[u8], len_size: usize) -> Result<Vec<u8>, HeifError> {
    let mut out = Vec::with_capacity(data.len() + 64);
    let mut i = 0;
    while i < data.len() {
        let l = be_n(data, i, len_size).ok_or_else(|| HeifError("short NAL".into()))? as usize;
        i += len_size;
        let nal = data
            .get(i..i + l)
            .ok_or_else(|| HeifError("NAL runs past its item".into()))?;
        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(nal);
        i += l;
    }
    Ok(out)
}

/// Read the primary picture of a HEIF-family file held in `b`.
pub fn read(b: &[u8]) -> Result<Heif, HeifError> {
    let m = read_meta(b)?;
    let mut id = m.primary;
    let mut item = m.item(id)?.clone();
    let mut transforms = Vec::new();

    // an identity-derived item: the transforms are its own, on its source
    let mut iden_props: Vec<usize> = Vec::new();
    if &item.typ == b"iden" {
        let src = m
            .refs_from(b"dimg", id)
            .and_then(|v| v.first().copied())
            .ok_or_else(|| HeifError("iden without a source".into()))?;
        iden_props = item.props.clone();
        id = src;
        item = m.item(id)?.clone();
    }

    let (codec, tile_ids, grid) = match &item.typ {
        b"hvc1" => (Codec::Hevc, vec![id], None),
        b"av01" => (Codec::Av1, vec![id], None),
        b"jpeg" => (Codec::Jpeg, vec![id], None),
        b"grid" => {
            let tiles = m
                .refs_from(b"dimg", id)
                .cloned()
                .ok_or_else(|| HeifError("grid without tiles".into()))?;
            let g = m.data(b, id)?;
            let flags = *g.get(1).ok_or_else(|| HeifError("short grid".into()))?;
            let rows = *g.get(2).ok_or_else(|| HeifError("short grid".into()))? as u32 + 1;
            let cols = *g.get(3).ok_or_else(|| HeifError("short grid".into()))? as u32 + 1;
            let (w, h) = if flags & 1 == 1 {
                (be32(&g, 4), be32(&g, 8))
            } else {
                (be16(&g, 4).map(u32::from), be16(&g, 6).map(u32::from))
            };
            let (w, h) = (
                w.ok_or_else(|| HeifError("short grid".into()))?,
                h.ok_or_else(|| HeifError("short grid".into()))?,
            );
            if (rows * cols) as usize != tiles.len() {
                return err("grid tile count");
            }
            let first = m.item(*tiles.first().ok_or_else(|| HeifError("no tiles".into()))?)?;
            let codec = match &first.typ {
                b"hvc1" => Codec::Hevc,
                b"av01" => Codec::Av1,
                b"jpeg" => Codec::Jpeg,
                _ => return err("grid of an unsupported coding"),
            };
            (codec, tiles, Some((rows, cols, w, h)))
        }
        t => {
            return Err(HeifError(format!(
                "primary item of type {}",
                String::from_utf8_lossy(t)
            )))
        }
    };
    if tile_ids.len() > 4096 {
        return err("too many tiles");
    }

    let (width, height) = match grid {
        Some((_, _, w, h)) => (w, h),
        None => ispe(b, &m, id).ok_or_else(|| HeifError("no ispe".into()))?,
    };
    if width == 0 || height == 0 || width > 65_535 || height > 65_535 {
        return err("image size out of range");
    }

    // tiles and their configurations
    let mut tiles = Vec::with_capacity(tile_ids.len());
    let mut configs = Vec::with_capacity(tile_ids.len());
    for &t in &tile_ids {
        let it = m.item(t)?;
        let data = m.data(b, t)?;
        match codec {
            Codec::Hevc => {
                if &it.typ != b"hvc1" {
                    return err("mixed tile codings");
                }
                let p = m
                    .prop(t, b"hvcC")
                    .ok_or_else(|| HeifError("tile without hvcC".into()))?;
                let (ps, len_size) = hvcc(b, p)?;
                configs.push(ps);
                tiles.push(annex_b(&data, len_size)?);
            }
            Codec::Av1 => {
                if &it.typ != b"av01" {
                    return err("mixed tile codings");
                }
                let cfg = m
                    .prop(t, b"av1C")
                    .map(|p| b[(p.body.start + 4).min(p.body.end)..p.body.end].to_vec())
                    .unwrap_or_default();
                configs.push(cfg);
                tiles.push(data);
            }
            Codec::Jpeg => {
                if &it.typ != b"jpeg" {
                    return err("mixed tile codings");
                }
                configs.push(Vec::new());
                tiles.push(data);
            }
        }
    }

    // transforms, in association order: the coded item's (or the grid's),
    // then an identity item's
    let mut cur = (width, height);
    let mut props: Vec<usize> = item.props.clone();
    props.extend(iden_props);
    for &pi in &props {
        let Some(p) = m.props.get(pi - 1) else {
            continue;
        };
        match &p.typ {
            b"irot" => {
                let a = b.get(p.body.start).map(|x| x & 3).unwrap_or(0);
                if a != 0 {
                    transforms.push(Transform::Rotate(a));
                    if a % 2 == 1 {
                        cur = (cur.1, cur.0);
                    }
                }
            }
            b"imir" => {
                let a = b.get(p.body.start).map(|x| x & 1).unwrap_or(0);
                transforms.push(Transform::Mirror(a));
            }
            b"clap" => {
                if let Some(c) = clean_aperture(b, p, cur) {
                    cur = (c.2, c.3);
                    transforms.push(Transform::Crop {
                        left: c.0,
                        top: c.1,
                        width: c.2,
                        height: c.3,
                    });
                }
            }
            _ => {}
        }
    }

    // the Exif item that describes the primary (or its source)
    let exif = m
        .refs
        .iter()
        .filter(|(t, _, to)| t == b"cdsc" && (to.contains(&m.primary) || to.contains(&id)))
        .map(|(_, from, _)| *from)
        .find(|f| m.items.get(f).map(|x| &x.typ == b"Exif").unwrap_or(false))
        .and_then(|f| exif_range(b, &m, f));

    Ok(Heif {
        codec,
        tiles,
        configs,
        grid,
        width,
        height,
        transforms,
        exif,
    })
}

/// The TIFF structure inside an `Exif` item: a 4-byte offset to the TIFF
/// header, then the data (often after an `Exif\0\0` prefix). Only items
/// stored as one extent in the file are returned as a range.
fn exif_range(b: &[u8], m: &Meta, id: u32) -> Option<Range<usize>> {
    let (method, base, ext) = m.items.get(&id)?.loc.clone()?;
    if method != 0 || ext.len() != 1 {
        return None;
    }
    let start = (base + ext[0].0) as usize;
    let end = start.checked_add(ext[0].1 as usize)?;
    if end > b.len() || end < start + 8 {
        return None;
    }
    let skip = be32(b, start)? as usize;
    let tiff = start + 4 + skip;
    if tiff + 8 > end {
        return None;
    }
    let head = &b[tiff..tiff + 4];
    (head == b"II*\0" || head == b"MM\0*").then_some(tiff..end)
}

/// ISO/IEC 14496-12 §12.1.4: the clean aperture is centred at
/// (horizOff + (w − 1)/2, vertOff + (h − 1)/2); left = centre −
/// (cleanW − 1)/2. Returns (left, top, width, height), whole pixels,
/// or `None` when it does not fit the image.
fn clean_aperture(b: &[u8], p: &BoxRef, (w, h): (u32, u32)) -> Option<(u32, u32, u32, u32)> {
    let s = p.body.start;
    let f = |i: usize| be32(b, s + 4 * i);
    let (wn, wd, hn, hd) = (f(0)?, f(1)?, f(2)?, f(3)?);
    let (xn, xd, yn, yd) = (f(4)? as i32, f(5)?, f(6)? as i32, f(7)?);
    if wd == 0 || hd == 0 || xd == 0 || yd == 0 {
        return None;
    }
    let cw = wn as f64 / wd as f64;
    let ch = hn as f64 / hd as f64;
    let cx = xn as f64 / xd as f64 + (w as f64 - 1.0) / 2.0;
    let cy = yn as f64 / yd as f64 + (h as f64 - 1.0) / 2.0;
    let left = (cx - (cw - 1.0) / 2.0).round();
    let top = (cy - (ch - 1.0) / 2.0).round();
    let (cw, ch) = (cw.round(), ch.round());
    if left < 0.0 || top < 0.0 || cw < 1.0 || ch < 1.0 {
        return None;
    }
    if left + cw > w as f64 || top + ch > h as f64 {
        return None;
    }
    Some((left as u32, top as u32, cw as u32, ch as u32))
}

impl Heif {
    /// The elementary stream ffmpeg decodes: every tile in order, each
    /// preceded by its configuration (Annex B HEVC, or AV1 low-overhead
    /// OBUs, one temporal unit per tile).
    pub fn stream(&self) -> Vec<u8> {
        let total: usize = self.tiles.iter().map(Vec::len).sum::<usize>()
            + self.configs.iter().map(Vec::len).sum::<usize>();
        let mut out = Vec::with_capacity(total + self.tiles.len() * 2);
        for (cfg, t) in self.configs.iter().zip(&self.tiles) {
            match self.codec {
                Codec::Hevc => {
                    out.extend_from_slice(cfg);
                    out.extend_from_slice(t);
                }
                Codec::Av1 => {
                    // temporal delimiter, then the item (which carries its
                    // sequence header; the av1C copy is prepended only when
                    // the item does not start with one)
                    out.extend_from_slice(&[0x12, 0x00]);
                    let starts_with_seq = t.first().map(|x| (x >> 3) & 15 == 1).unwrap_or(false);
                    if !starts_with_seq {
                        out.extend_from_slice(cfg);
                    }
                    out.extend_from_slice(t);
                }
                Codec::Jpeg => {}
            }
        }
        out
    }

    /// Size after the transforms.
    pub fn display_size(&self) -> (u32, u32) {
        let mut s = (self.width, self.height);
        for t in &self.transforms {
            match t {
                Transform::Rotate(a) if a % 2 == 1 => s = (s.1, s.0),
                Transform::Crop { width, height, .. } => s = (*width, *height),
                _ => {}
            }
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A box: size, type, payload.
    fn bx(t: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut v = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        v.extend_from_slice(t);
        v.extend_from_slice(body);
        v
    }
    fn full(t: &[u8; 4], ver: u8, flags: u32, body: &[u8]) -> Vec<u8> {
        let mut b = vec![ver];
        b.extend_from_slice(&flags.to_be_bytes()[1..]);
        b.extend_from_slice(body);
        bx(t, &b)
    }

    /// A two-by-one grid of AV1 "tiles" (opaque bytes), rotated once and
    /// mirrored, with an Exif item: enough structure to read back.
    fn sample() -> Vec<u8> {
        let ftyp = bx(b"ftyp", b"avifmif1avifmiaf");
        let hdlr = full(b"hdlr", 0, 0, b"\0\0\0\0pict\0\0\0\0\0\0\0\0\0\0\0\0\0");
        let pitm = full(b"pitm", 0, 0, &3u16.to_be_bytes());
        let infe = |id: u16, t: &[u8; 4]| {
            let mut b = id.to_be_bytes().to_vec();
            b.extend_from_slice(&[0, 0]);
            b.extend_from_slice(t);
            b.push(0);
            full(b"infe", 2, 0, &b)
        };
        let mut iinf_body = 4u16.to_be_bytes().to_vec();
        iinf_body.extend(infe(1, b"av01"));
        iinf_body.extend(infe(2, b"av01"));
        iinf_body.extend(infe(3, b"grid"));
        iinf_body.extend(infe(4, b"Exif"));
        let iinf = full(b"iinf", 0, 0, &iinf_body);
        // iref: grid → tiles; exif → grid
        let mut dimg = 3u16.to_be_bytes().to_vec();
        dimg.extend_from_slice(&2u16.to_be_bytes());
        dimg.extend_from_slice(&1u16.to_be_bytes());
        dimg.extend_from_slice(&2u16.to_be_bytes());
        let mut cdsc = 4u16.to_be_bytes().to_vec();
        cdsc.extend_from_slice(&1u16.to_be_bytes());
        cdsc.extend_from_slice(&3u16.to_be_bytes());
        let mut iref_body = bx(b"dimg", &dimg);
        iref_body.extend(bx(b"cdsc", &cdsc));
        let iref = full(b"iref", 0, 0, &iref_body);
        // properties: 1 ispe 8×4 (tile), 2 av1C, 3 irot 1, 4 imir 1
        let mut ispe_b = 8u32.to_be_bytes().to_vec();
        ispe_b.extend_from_slice(&4u32.to_be_bytes());
        let mut ipco_body = full(b"ispe", 0, 0, &ispe_b);
        ipco_body.extend(bx(b"av1C", &[0x81, 0, 0, 0]));
        ipco_body.extend(bx(b"irot", &[1]));
        ipco_body.extend(bx(b"imir", &[1]));
        let ipco = bx(b"ipco", &ipco_body);
        let mut ipma_body = 3u32.to_be_bytes().to_vec();
        for (id, props) in [(1u16, vec![1u8, 2]), (2, vec![1, 2]), (3, vec![3, 4])] {
            ipma_body.extend_from_slice(&id.to_be_bytes());
            ipma_body.push(props.len() as u8);
            ipma_body.extend(props.iter().map(|p| p | 0x80));
        }
        let ipma = full(b"ipma", 0, 0, &ipma_body);
        let mut iprp_body = ipco;
        iprp_body.extend(ipma);
        let iprp = bx(b"iprp", &iprp_body);
        // data: tile1 "AAAA", tile2 "BBBB", grid (rows 1, cols 2, 16×4),
        // exif (offset 0, then a TIFF header)
        let grid: [u8; 8] = [0, 0, 0, 1, 0, 16, 0, 4];
        let exif: Vec<u8> = [&0u32.to_be_bytes()[..], b"II*\0\x08\0\0\0\0\0"].concat();
        let datas: Vec<Vec<u8>> = vec![b"AAAA".to_vec(), b"BBBB".to_vec(), grid.to_vec(), exif];
        // iloc v1 with 4-byte offsets and lengths; offsets patched below
        let iloc_len = 4 + 2 + 2 + datas.len() * (2 + 2 + 2 + 2 + 4 + 4) + 8;
        let pre_len = ftyp.len()
            + 12
            + hdlr.len()
            + pitm.len()
            + iinf.len()
            + iref.len()
            + iprp.len()
            + iloc_len;
        let mdat_start = pre_len + 8;
        let mut iloc_body = vec![0x44, 0x00];
        iloc_body.extend_from_slice(&(datas.len() as u16).to_be_bytes());
        let mut off = mdat_start;
        for (i, d) in datas.iter().enumerate() {
            iloc_body.extend_from_slice(&((i + 1) as u16).to_be_bytes());
            iloc_body.extend_from_slice(&[0, 0]); // construction method 0
            iloc_body.extend_from_slice(&[0, 0]); // data reference
            iloc_body.extend_from_slice(&1u16.to_be_bytes());
            iloc_body.extend_from_slice(&(off as u32).to_be_bytes());
            iloc_body.extend_from_slice(&(d.len() as u32).to_be_bytes());
            off += d.len();
        }
        let iloc = full(b"iloc", 1, 0, &iloc_body);
        let mut meta_body = hdlr;
        meta_body.extend(pitm);
        meta_body.extend(iloc);
        meta_body.extend(iinf);
        meta_body.extend(iref);
        meta_body.extend(iprp);
        let meta = full(b"meta", 0, 0, &meta_body);
        let mut out = ftyp;
        out.extend(meta);
        assert_eq!(out.len() + 8, mdat_start, "layout arithmetic");
        out.extend(bx(b"mdat", &datas.concat()));
        out
    }

    #[test]
    fn reads_a_grid_its_transforms_and_its_exif() {
        let b = sample();
        assert!(is_heif(&b));
        let h = read(&b).unwrap();
        assert_eq!(h.codec, Codec::Av1);
        assert_eq!(h.grid, Some((1, 2, 16, 4)));
        assert_eq!(h.tiles, vec![b"AAAA".to_vec(), b"BBBB".to_vec()]);
        assert_eq!((h.width, h.height), (16, 4));
        assert_eq!(
            h.transforms,
            vec![Transform::Rotate(1), Transform::Mirror(1)]
        );
        assert_eq!(h.display_size(), (4, 16));
        let e = h.exif.clone().unwrap();
        assert_eq!(&b[e.start..e.start + 4], b"II*\0");
        // a temporal delimiter before each tile; av1C's OBUs (none here)
        let s = h.stream();
        assert_eq!(&s[..2], &[0x12, 0]);
        assert_eq!(s.len(), 2 * (2 + 4));
    }

    #[test]
    fn truncated_or_garbled_files_are_errors_not_panics() {
        let b = sample();
        for cut in [0, 10, 40, 100, b.len() / 2, b.len() - 3] {
            let _ = read(&b[..cut]);
        }
        let mut g = b.clone();
        for i in (0..g.len()).step_by(7) {
            g[i] ^= 0x5A;
            let _ = read(&g);
        }
        assert!(read(b"\0\0\0\x0cftypheic").is_err());
    }

    #[test]
    fn clean_aperture_matches_the_test_set_crop() {
        // kimono.crop.avif: 722×1024, clap 385/1 × 330/1, offsets 207/2
        // and −616/2 → the expected crop starts at (272, 39) (measured
        // against kimono.crop.png, RESEARCH §2.9)
        let mut body = Vec::new();
        for v in [385u32, 1, 330, 1, 207, 2, (-616i32) as u32, 2] {
            body.extend_from_slice(&v.to_be_bytes());
        }
        let b = bx(b"clap", &body);
        let p = BoxRef {
            typ: *b"clap",
            body: 8..b.len(),
        };
        assert_eq!(
            clean_aperture(&b, &p, (722, 1024)),
            Some((272, 39, 385, 330))
        );
    }

    #[test]
    fn annex_b_rewrites_length_prefixes() {
        let d = [0, 0, 0, 2, 0xAA, 0xBB, 0, 0, 0, 1, 0xCC];
        assert_eq!(
            annex_b(&d, 4).unwrap(),
            vec![0, 0, 0, 1, 0xAA, 0xBB, 0, 0, 0, 1, 0xCC]
        );
        assert!(annex_b(&[0, 0, 0, 9, 1], 4).is_err());
    }
}
