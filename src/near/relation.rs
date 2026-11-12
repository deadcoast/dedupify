//! How a member relates to its keeper: the evidence, the verdict, the
//! words on its line. (Split from `near.rs`, Phase 14, SC-09.)

use super::*;

/// How a member relates to its keeper: the evidence, and what it allows.
///
/// Acting (moving or binning) needs two independent families to agree:
/// the two 64-bit hashes **and** either PDQ (frequency domain, 256 bits)
/// or the geometric fit (spatial correlation); or, without the hashes,
/// the learned descriptor **and** the fit. A member whose capture time
/// differs from the keeper's is a different photograph whatever the
/// pictures say: `kin`, listed, never acted on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Relation {
    pub variant: Variant,
    pub d: u32,
    pub p: u32,
    /// PDQ distance to the keeper on the same orientation (0–256).
    pub q: u32,
    /// PDQ quality of the lower-quality side; below `PDQ_QUALITY_MIN` the
    /// PDQ distance carries no weight.
    pub q_quality: u8,
    /// SSCD cosine to the keeper when the deep tier is active.
    pub deep: Option<f32>,
    /// Geometric fit to the keeper, computed when PDQ did not confirm or
    /// the member came in through the deep tier.
    pub fit: Option<Fit>,
    /// Both files state a capture time and they differ.
    pub kin: bool,
    /// The member is a camera RAW: listed, never acted on, not even by
    /// hand (Phase 10). [CLAUDE]
    pub raw: bool,
    /// The keeper is a camera RAW: the member's size is not compared with
    /// it (a RAW's own pixel count is not read, only its preview's).
    pub keeper_raw: bool,
    /// A RAW and the camera's own JPEG of it (same folder, same name):
    /// kept together; the JPEG can be chosen by hand.
    pub pair: bool,
    /// Another frame of the same moment: both files state one capture
    /// time and it agrees, but their names are numbered apart
    /// (`IMG_0012`, `IMG_0013`) — a burst from a camera that records no
    /// fraction of a second. Counted as `kin`. [CLAUDE, Phase 11]
    pub frame: bool,
    /// Flat bars one side has and the other has not.
    pub boxed: Boxed,
}

/// Which of a near pair has flat bars around its picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Boxed {
    /// Neither, or both.
    #[default]
    No,
    /// The member's picture sits between bars left and right.
    Pillar,
    /// Between bars above and below.
    Letter,
    /// Inside a frame on all four sides.
    Border,
    /// The keeper has bars the member has not.
    Removed,
}

impl Boxed {
    fn of(keeper: Option<[f32; 4]>, member: Option<[f32; 4]>) -> Boxed {
        match (keeper, member) {
            (None, Some([l, t, r, b])) => {
                let (x, y) = (l > 0.0 || r < 1.0, t > 0.0 || b < 1.0);
                match (x, y) {
                    (true, false) => Boxed::Pillar,
                    (false, true) => Boxed::Letter,
                    _ => Boxed::Border,
                }
            }
            (Some(_), None) => Boxed::Removed,
            _ => Boxed::No,
        }
    }
}

impl Relation {
    /// The two 64-bit hashes agree.
    pub fn hash_pair(&self) -> bool {
        self.d <= DHASH_MAX && self.p <= PHASH_MAX
    }

    /// PDQ within threshold on a hash PDQ itself rates reliable.
    pub fn pdq_ok(&self) -> bool {
        self.q <= PDQ_MAX && self.q_quality >= PDQ_QUALITY_MIN
    }

    pub fn fit_ok(&self) -> bool {
        self.fit.is_some_and(|f| f.confirmed())
    }

    /// Admitted by the learned descriptor: at `DEEP_MIN`, or in the band
    /// under it when the fit confirms the pair as well.
    pub fn deep_in(&self) -> bool {
        self.deep.is_some_and(|c| c >= crate::deep::DEEP_MIN) || (self.deep_band() && self.fit_ok())
    }

    /// The descriptor's cosine is in `[DEEP_FIT_MIN, DEEP_MIN)`: such a
    /// pair is grouped only when the fit confirms it (Phase 11).
    pub fn deep_band(&self) -> bool {
        self.deep
            .is_some_and(|c| (crate::deep::DEEP_FIT_MIN..crate::deep::DEEP_MIN).contains(&c))
    }

    /// Grouped by the deep tier alone: the two 64-bit hashes disagree.
    pub fn deep_only(&self) -> bool {
        !self.hash_pair() && self.deep_in()
    }

    /// What `milk` and `purge` require before touching the file.
    pub fn confirmed(&self) -> bool {
        if self.kin || self.raw || self.pair {
            return false;
        }
        if self.hash_pair() {
            self.pdq_ok() || self.fit_ok()
        } else {
            self.deep_in() && self.fit_ok()
        }
    }

    /// Shown but never acted on by the program's rule, whatever else is
    /// measured: a different photograph (`kin`), a RAW, or the learned
    /// descriptor's word alone.
    pub fn listed_only(&self) -> bool {
        self.kin || self.raw || (self.deep_only() && !self.fit_ok())
    }

    /// The verb on the member's line.
    pub fn verb(&self) -> &'static str {
        if self.kin {
            "kin"
        } else if self.raw {
            "raw"
        } else if self.pair {
            "pair"
        } else if self.deep_only() && !self.fit_ok() {
            "deep"
        } else {
            "near"
        }
    }

    /// What the member is, in words, for its line: the relation read off
    /// the evidence rather than the evidence itself. `keeper`/`member`
    /// supply the pixel sizes and the capture time. [CLAUDE]
    ///
    /// `mirrored`, `rotated 90°`, `resized 50%`, `cropped 80%`,
    /// `letterboxed`, `re-encoded`, `same picture`, `another shot …`,
    /// `looks alike`, `similar, unconfirmed`; two words at most, joined
    /// with a comma.
    pub fn words(&self, keeper: &Hashed, member: &Hashed) -> String {
        self.words_for(
            (keeper.sig.width, keeper.sig.height),
            (member.sig.width, member.sig.height),
            keeper.capture.as_ref().map(|c| c.datetime.as_str()),
            member.capture.as_ref().map(|c| c.datetime.as_str()),
        )
    }

    /// `words` on bare inputs: pixel sizes and `YYYY-MM-DD HH:MM:SS`
    /// capture times.
    pub fn words_for(
        &self,
        keeper: (u32, u32),
        member: (u32, u32),
        keeper_taken: Option<&str>,
        member_taken: Option<&str>,
    ) -> String {
        if self.frame {
            return "another shot, same second".to_string();
        }
        if self.kin {
            // the member's own moment: the time when the day is the
            // keeper's, the day otherwise
            return match (member_taken, keeper_taken) {
                (Some(m), Some(k)) if m.get(..10) == k.get(..10) => {
                    format!("another shot, {}", m.get(11..).unwrap_or(""))
                }
                (Some(m), _) => format!("another shot, {}", m.get(..10).unwrap_or("")),
                (None, _) => "another shot".to_string(),
            };
        }
        if self.pair {
            return "RAW + JPEG".to_string();
        }
        if self.raw {
            return "the RAW".to_string();
        }
        if self.deep_only() && !self.fit_ok() {
            return "looks alike".to_string();
        }
        if !self.confirmed() {
            return "similar, unconfirmed".to_string();
        }
        let mut parts: Vec<String> = Vec::new();
        // orientation: a 64-bit finding, so only when the hashes agree
        let orientation = if self.hash_pair() {
            match self.variant {
                Variant::Identity => "",
                Variant::Rotate90 => "rotated 90°",
                Variant::Rotate180 => "rotated 180°",
                Variant::Rotate270 => "rotated 270°",
                Variant::FlipH => "mirrored",
                Variant::FlipV => "flipped",
                Variant::Transpose | Variant::AntiTranspose => "rotated and mirrored",
            }
        } else {
            ""
        };
        if !orientation.is_empty() {
            parts.push(orientation.to_string());
        }
        // geometry: what the fit found, when it was the evidence
        let mut geometric = false;
        let fit = self.fit.filter(|f| f.confirmed());
        if fit.is_some_and(|f| f.mirrored) && orientation.is_empty() {
            parts.push("mirrored".to_string());
        }
        // flat bars one side has: what was added or taken away around the
        // picture, which the fit's own placement then need not repeat
        let boxed = match self.boxed {
            Boxed::No => None,
            Boxed::Pillar => Some("pillarboxed"),
            Boxed::Letter => Some("letterboxed"),
            Boxed::Border => Some("with a border"),
            Boxed::Removed => Some("bars removed"),
        };
        if let Some(w) = boxed {
            parts.push(w.to_string());
            geometric = true;
        }
        if let Some(f) = fit {
            if f.angle != 0.0 {
                parts.push(format!("rotated {:+.0}°", f.angle));
                geometric = true;
            } else if f.scale < 0.995 && boxed.is_none() {
                if f.inner_is_second {
                    parts.push(format!("cropped {:.0}%", f.scale * 100.0));
                } else {
                    // the member holds the keeper's picture inside a
                    // frame: bars when the shape changed, a border otherwise
                    let ka = keeper.0 as f32 / keeper.1.max(1) as f32;
                    let ma = member.0 as f32 / member.1.max(1) as f32;
                    parts.push(if (ka - ma).abs() / ka > 0.05 {
                        "letterboxed".to_string()
                    } else {
                        "with a border".to_string()
                    });
                }
                geometric = true;
            }
        }
        // size: the member's pixels against the keeper's, on the member's
        // own orientation
        let (mw, mh) = match self.variant {
            Variant::Rotate90
            | Variant::Rotate270
            | Variant::Transpose
            | Variant::AntiTranspose => (member.1, member.0),
            _ => (member.0, member.1),
        };
        let (kw, kh) = (keeper.0.max(1), keeper.1.max(1));
        let rw = mw as f32 / kw as f32;
        let rh = mh as f32 / kh as f32;
        let same_size = (rw - 1.0).abs() <= 0.03 && (rh - 1.0).abs() <= 0.03;
        // against a RAW the comparison would be with its preview's size
        if !geometric && !same_size && !self.keeper_raw {
            if (rw - rh).abs() <= 0.03 {
                parts.push(format!("resized {:.0}%", rw * 100.0));
            } else {
                parts.push(format!("{}×{}", mw, mh));
            }
        }
        if parts.is_empty() && self.keeper_raw {
            parts.push("from the RAW".to_string());
        }
        if parts.is_empty() {
            parts.push(if !self.hash_pair() {
                // in through the descriptor and the fit: the picture is
                // there, the pixels were worked on
                "edited".to_string()
            } else if self.d == 0 && self.p == 0 && self.q == 0 {
                "same picture".to_string()
            } else if self.q <= 8 {
                "re-encoded".to_string()
            } else {
                "near copy".to_string()
            });
        }
        parts.truncate(2);
        parts.join(", ")
    }

    /// The evidence behind the words, for the dim trailing column:
    /// `d 0  p 0  q 4`, then `deep 0.80` and `fit 1.00` when they exist.
    pub fn numbers(&self) -> String {
        let mut s = format!("d {:>2}  p {:>2}  q {:>3}", self.d, self.p, self.q);
        if let Some(c) = self.deep {
            s.push_str(&format!("  deep {c:.2}"));
        }
        if let Some(f) = self.fit {
            s.push_str(&format!("  fit {:.2}", f.score().max(0.0)));
        }
        s
    }

    /// Why `confirmed()` is false, for the `kept~` line.
    pub fn unconfirmed_reason(&self) -> String {
        if self.frame {
            return "another frame of the same second".to_string();
        }
        if self.kin {
            return "taken at a different moment".to_string();
        }
        if self.pair {
            return "the camera's JPEG of a RAW, kept with it".to_string();
        }
        if self.raw {
            return "a RAW, never moved".to_string();
        }
        let fit_txt = match self.fit {
            Some(f) => format!("fit {:.2} under {FIT_MIN:.2}", f.score().max(0.0)),
            None => "no fit".to_string(),
        };
        if self.deep_only() {
            return format!("deep {:.2} only; {fit_txt}", self.deep.unwrap_or(0.0));
        }
        if self.q_quality < PDQ_QUALITY_MIN {
            format!(
                "PDQ quality {} below {}; {fit_txt}",
                self.q_quality, PDQ_QUALITY_MIN
            )
        } else if self.q > PDQ_MAX {
            format!("PDQ q {} over {}; {fit_txt}", self.q, PDQ_MAX)
        } else {
            format!("d {} p {} over threshold to keeper", self.d, self.p)
        }
    }
}

/// Best relation of `m` to the keeper `k`, searching both directions.
pub fn relation(k: &Signature, m: &Signature) -> Relation {
    relation_deep(k, m, None, false)
}

/// The relation of two hashed files: distances, deep cosine, and the
/// capture-time verdict. The fit is added later, only where it is needed.
pub fn relation_of(k: &Hashed, m: &Hashed) -> Relation {
    let cos = match (k.deep.as_ref(), m.deep.as_ref()) {
        (Some(a), Some(b)) => Some(crate::deep::cosine(a, b)),
        _ => None,
    };
    let (kp, mp) = (&k.rep.cand.path, &m.rep.cand.path);
    let frame = capture::different_moments(k.capture.as_ref(), m.capture.as_ref()) == Some(false)
        && numbered_apart(kp, mp);
    let kin =
        frame || capture::different_moments(k.capture.as_ref(), m.capture.as_ref()) == Some(true);
    let mut r = relation_deep(&k.sig, &m.sig, cos, kin);
    r.frame = frame;
    r.raw = m.raw;
    r.keeper_raw = k.raw;
    r.pair = !kin && k.raw != m.raw && twins(kp, mp);
    r.boxed = Boxed::of(k.bars, m.bars);
    r
}

/// Two names a camera numbered apart: the same once every run of digits
/// is set aside, with at least one of those numbers different —
/// `IMG_0012` / `IMG_0013`, `DSC00012` / `DSC00013`,
/// `20230101_120000_001` / `…_002`, `payload_N000` / `payload_N001`.
/// A copy's name changes shape instead (`IMG_0012 (1)`, `IMG_0012 2`,
/// `IMG_0012-2`, `IMG_0012 - Copy`), and a format conversion keeps the
/// number (`IMG_0012.png`). Letters compare without case; numbers by
/// value. [CLAUDE, Phase 11]
pub fn numbered_apart(a: &std::path::Path, b: &std::path::Path) -> bool {
    fn runs(p: &std::path::Path) -> Option<(Vec<String>, Vec<u128>)> {
        let stem = p.file_stem()?.to_string_lossy().to_lowercase();
        let (mut words, mut nums) = (vec![String::new()], Vec::new());
        let mut digits = String::new();
        for c in stem.chars() {
            if c.is_ascii_digit() {
                digits.push(c);
                continue;
            }
            if !digits.is_empty() {
                nums.push(digits.parse().ok()?);
                digits.clear();
                words.push(String::new());
            }
            words.last_mut()?.push(c);
        }
        if !digits.is_empty() {
            nums.push(digits.parse().ok()?);
            words.push(String::new());
        }
        Some((words, nums))
    }
    match (runs(a), runs(b)) {
        (Some((wa, na)), Some((wb, nb))) => wa == wb && na.len() == nb.len() && na != nb,
        _ => false,
    }
}

/// A camera writes a RAW and its JPEG side by side under one name
/// (`IMG_0041.CR2`, `IMG_0041.JPG`): same folder, same stem, the case of
/// either ignored.
pub fn twins(a: &std::path::Path, b: &std::path::Path) -> bool {
    let stem = |p: &std::path::Path| p.file_stem().map(|s| s.to_string_lossy().to_lowercase());
    a.parent() == b.parent() && stem(a).is_some() && stem(a) == stem(b)
}

pub fn relation_deep(k: &Signature, m: &Signature, deep: Option<f32>, kin: bool) -> Relation {
    let mut best: Option<(Variant, u32, u32, u32)> = None;
    for (i, v) in perceptual::VARIANT_ORDER.iter().enumerate() {
        let d1 = perceptual::hamming(k.dhash[0], m.dhash[i]);
        let p1 = perceptual::hamming(k.phash[0], m.phash[i]);
        let q1 = pdq_distance(&k.pdq[0], &m.pdq[i]);
        let d2 = perceptual::hamming(m.dhash[0], k.dhash[i]);
        let p2 = perceptual::hamming(m.phash[0], k.phash[i]);
        let q2 = pdq_distance(&m.pdq[0], &k.pdq[i]);
        for (d, p, q) in [(d1, p1, q1), (d2, p2, q2)] {
            if best.is_none_or(|(_, bd, bp, _)| d + p < bd + bp) {
                best = Some((*v, d, p, q));
            }
        }
    }
    let (variant, d, p, q) = best.expect("invariant: VARIANT_ORDER is not empty");
    Relation {
        variant,
        d,
        p,
        q,
        q_quality: k.pdq_quality.min(m.pdq_quality),
        deep,
        fit: None,
        kin,
        raw: false,
        keeper_raw: false,
        pair: false,
        frame: false,
        boxed: Boxed::No,
    }
}

#[cfg(test)]
mod words_tests {
    use super::*;

    fn rel(d: u32, p: u32, q: u32) -> Relation {
        Relation {
            variant: Variant::Identity,
            d,
            p,
            q,
            q_quality: 100,
            deep: None,
            fit: None,
            kin: false,
            raw: false,
            keeper_raw: false,
            pair: false,
            frame: false,
            boxed: Boxed::No,
        }
    }

    #[test]
    fn plain_words_for_plain_cases() {
        let k = (1000, 750);
        assert_eq!(rel(0, 0, 0).words_for(k, k, None, None), "same picture");
        assert_eq!(rel(1, 0, 6).words_for(k, k, None, None), "re-encoded");
        assert_eq!(rel(4, 2, 22).words_for(k, k, None, None), "near copy");
        assert_eq!(
            rel(0, 0, 4).words_for(k, (500, 375), None, None),
            "resized 50%"
        );
        assert_eq!(
            rel(0, 0, 4).words_for(k, (1500, 1125), None, None),
            "resized 150%"
        );
        // an odd shape with no fit consulted: the size itself
        assert_eq!(rel(0, 0, 4).words_for(k, (800, 750), None, None), "800×750");
        let mut r = rel(0, 0, 6);
        r.variant = Variant::Rotate90;
        assert_eq!(r.words_for(k, (750, 1000), None, None), "rotated 90°");
        r.variant = Variant::FlipH;
        assert_eq!(r.words_for(k, k, None, None), "mirrored");
        r.variant = Variant::FlipH;
        assert_eq!(
            r.words_for(k, (500, 375), None, None),
            "mirrored, resized 50%"
        );
    }

    #[test]
    fn numbered_apart_tells_frames_from_copies() {
        use std::path::Path;
        let ap = |a: &str, b: &str| numbered_apart(Path::new(a), Path::new(b));
        // frames a camera numbered
        for (a, b) in [
            ("d/IMG_0012.JPG", "d/IMG_0013.JPG"),
            ("d/DSC00012.JPG", "e/dsc00013.jpg"),
            ("d/20230101_120000_001.jpg", "d/20230101_120000_002.jpg"),
            ("burst/payload_N000.jpg", "burst/payload_N003.jpg"),
            ("d/P1000123.JPG", "d/P1000124.JPG"),
        ] {
            assert!(ap(a, b), "{a} {b}");
        }
        // copies, conversions, and names with no number
        for (a, b) in [
            ("d/IMG_0012.JPG", "d/IMG_0012 (1).JPG"),
            ("d/IMG_0012.JPG", "d/IMG_0012 2.JPG"),
            ("d/IMG_0012.JPG", "d/IMG_0012-2.JPG"),
            ("d/IMG_0012.JPG", "d/IMG_0012 - Copy.JPG"),
            ("d/IMG_0012.JPG", "exports/IMG_0012.png"),
            ("d/IMG_0012.JPG", "d/img_0012.jpg"),
            ("d/IMG_0012.JPG", "d/IMG_E0012.JPG"),
            ("d/beach.jpg", "d/beach_small.jpg"),
            ("d/beach.jpg", "e/beach.jpg"),
            ("d/0012.jpg", "d/012.jpg"),
        ] {
            assert!(!ap(a, b), "{a} {b}");
        }
    }

    #[test]
    fn fit_words_and_the_frames() {
        let k = (1000, 750);
        let mut r = rel(10, 6, 32);
        r.fit = Some(Fit {
            ncc: 1.0,
            scale: 0.95,
            dx: 0,
            dy: 0,
            angle: 0.0,
            inner_is_second: true,
            ..Fit::none()
        });
        assert_eq!(r.words_for(k, (950, 712), None, None), "cropped 95%");
        r.fit = Some(Fit {
            ncc: 0.97,
            scale: 0.8,
            dx: 0,
            dy: 0,
            angle: 0.0,
            inner_is_second: false,
            ..Fit::none()
        });
        assert_eq!(r.words_for(k, (1000, 1000), None, None), "letterboxed");
        assert_eq!(r.words_for(k, (1100, 825), None, None), "with a border");
        r.fit = Some(Fit {
            ncc: 0.95,
            scale: 1.0,
            dx: 0,
            dy: 0,
            angle: 4.0,
            inner_is_second: true,
            ..Fit::none()
        });
        assert_eq!(r.words_for(k, k, None, None), "rotated +4°");
        // the fit fell short: listed, not acted on
        r.fit = Some(Fit {
            ncc: 0.8,
            scale: 1.0,
            dx: 0,
            dy: 0,
            angle: 0.0,
            inner_is_second: true,
            ..Fit::none()
        });
        assert_eq!(r.words_for(k, k, None, None), "similar, unconfirmed");
    }

    #[test]
    fn moments_and_the_descriptor() {
        let k = (1000, 750);
        let mut r = rel(1, 2, 4);
        r.kin = true;
        assert_eq!(
            r.words_for(
                k,
                k,
                Some("2021-03-04 05:06:07"),
                Some("2021-03-04 05:06:08")
            ),
            "another shot, 05:06:08"
        );
        assert_eq!(
            r.words_for(
                k,
                k,
                Some("2021-03-04 05:06:07"),
                Some("2021-03-05 09:00:00")
            ),
            "another shot, 2021-03-05"
        );
        let mut d = rel(30, 28, 120);
        d.deep = Some(0.8);
        assert_eq!(d.words_for(k, k, None, None), "looks alike");
        d.fit = Some(Fit {
            ncc: 0.96,
            scale: 1.0,
            dx: 0,
            dy: 0,
            angle: 0.0,
            inner_is_second: true,
            ..Fit::none()
        });
        assert_eq!(d.words_for(k, k, None, None), "edited");
    }
}
