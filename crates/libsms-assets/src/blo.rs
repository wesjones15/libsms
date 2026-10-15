//! J2D screens (`.blo`, `SCRNblo1`): the HUD's layout of panes and pictures.
//!
//! Read as `J2DScreen::makeHiearachyPanes` reads them (JSystem J2DScreen.cpp): an `INF1` block
//! with the screen size, then pane blocks (`PAN1`, `PIC1`, `WIN1`, `TBX1`) nested by
//! `BGN1`/`END1`, up to `EXT1`. Every pane block starts as `J2DPane`'s stream constructor reads
//! it (a field count, visibility, the pane's tag and its rectangle relative to its parent, then
//! optional rotation, anchor and alpha); a picture goes on as `J2DPicture`'s (its `TIMG`
//! reference, then optional mirror, wrap and black/white colours). Only what drawing pictures
//! needs is kept.

use crate::Error;
use crate::reader::Be;

/// One pane.
#[derive(Debug, Clone)]
pub struct Pane {
    /// Block kind: `PAN1`, `PIC1`, `WIN1` or `TBX1`.
    pub kind: [u8; 4],
    /// The pane's tag (`mUserInfoTag`), the multi-character constant the game searches by
    /// (`'w_ba'`, `'\0w_0'`).
    pub tag: u32,
    pub visible: bool,
    /// Rectangle relative to the parent's top-left: x, y, width, height.
    pub rect: [i32; 4],
    /// Top-left on the screen (the parents' offsets summed, as `J2DPane::draw` places it).
    pub at: [i32; 2],
    pub alpha: u8,
    /// A picture's texture file name (`timg/<name>`).
    pub texture: Option<String>,
    /// A picture's colours (`mBlack`/`mWhite`, RGBA): the texture's 0 and 1 map to these.
    pub black: u32,
    pub white: u32,
    /// A picture's corner colours (RGBA), top left, top right, bottom left, bottom right: what its
    /// texture is multiplied by (`J2DPicture::mCornerColor`), all white when it has none.
    pub corners: [u32; 4],
    /// Index of the parent pane.
    pub parent: Option<usize>,
}

/// A whole screen.
#[derive(Debug, Clone)]
pub struct Screen {
    pub width: i32,
    pub height: i32,
    pub panes: Vec<Pane>,
}

/// A pane tag from its name as the game writes it, right-aligned: `tag("w_0")` is `'\0w_0'`.
pub fn tag(name: &str) -> u32 {
    name.bytes().fold(0, |t, b| (t << 8) | b as u32)
}

impl Screen {
    pub fn parse(d: &[u8]) -> Result<Screen, Error> {
        let r = Be(d);
        if r.bytes(0, 8)? != b"SCRNblo1" {
            return Err(Error::Bad("not a SCRNblo1 screen".into()));
        }
        let mut o = 0x20;
        if r.bytes(o, 4)? != b"INF1" {
            return Err(Error::Bad("no INF1 block".into()));
        }
        let width = r.i16(o + 8)? as i32;
        let height = r.i16(o + 10)? as i32;
        o += r.u32(o + 4)? as usize;

        let mut panes: Vec<Pane> = Vec::new();
        // Parents of the current level; a BGN1 makes the last pane read the parent.
        let mut stack: Vec<Option<usize>> = vec![None];
        let mut last: Option<usize> = None;
        while o + 8 <= d.len() {
            let kind: [u8; 4] = r.bytes(o, 4)?.try_into().unwrap();
            let size = r.u32(o + 4)? as usize;
            if size < 8 {
                return Err(Error::Bad(format!("block of {size} bytes at {o:#x}")));
            }
            match &kind {
                b"EXT1" => break,
                b"BGN1" => stack.push(last),
                b"END1" => {
                    stack.pop();
                    if stack.is_empty() {
                        break;
                    }
                }
                _ => {
                    let parent = *stack.last().unwrap();
                    let pane = Self::pane(&r, o + 8, kind, parent, &panes)?;
                    panes.push(pane);
                    last = Some(panes.len() - 1);
                }
            }
            o += size;
        }
        Ok(Screen {
            width,
            height,
            panes,
        })
    }

    /// `J2DPane::J2DPane(J2DPane*, JSURandomInputStream*, true)` and, for a picture, the rest
    /// of `J2DPicture`'s.
    fn pane(
        r: &Be,
        mut p: usize,
        kind: [u8; 4],
        parent: Option<usize>,
        panes: &[Pane],
    ) -> Result<Pane, Error> {
        let fields = r.u8(p)?;
        let visible = r.u8(p + 1)? != 0;
        let tag = r.u32(p + 4)?;
        let rect = [
            r.i16(p + 8)? as i32,
            r.i16(p + 10)? as i32,
            r.i16(p + 12)? as i32,
            r.i16(p + 14)? as i32,
        ];
        p += 16;
        // Optional: rotation (u16), anchor (u8), alpha (u8), alpha inherited (u8).
        let mut left = fields.saturating_sub(6);
        let mut alpha = 255;
        if left > 0 {
            p += 2;
            left -= 1;
        }
        if left > 0 {
            p += 1;
            left -= 1;
        }
        if left > 0 {
            alpha = r.u8(p)?;
            p += 1;
            left -= 1;
        }
        if left > 0 {
            p += 1;
        }
        p = (p + 3) & !3;

        let base = parent.map(|i| panes[i].at).unwrap_or([0, 0]);
        let mut pane = Pane {
            kind,
            tag,
            visible,
            rect,
            at: [base[0] + rect[0], base[1] + rect[1]],
            alpha,
            texture: None,
            black: 0,
            white: 0xFFFF_FFFF,
            corners: [0xFFFF_FFFF; 4],
            parent,
        };
        if &kind == b"PIC1" {
            let mut left = r.u8(p)?;
            p += 1;
            // `JUTResReference`: a type byte, a length and the name.
            let reference = |p: &mut usize| -> Result<String, Error> {
                let n = r.u8(*p + 1)? as usize;
                let name = String::from_utf8_lossy(r.bytes(*p + 2, n)?).into_owned();
                *p += 2 + n;
                Ok(name)
            };
            let timg = reference(&mut p)?;
            reference(&mut p)?; // TLUT
            p += 1; // binding
            left = left.saturating_sub(3);
            for _ in 0..2 {
                // mirror, wrap
                if left > 0 {
                    p += 1;
                    left -= 1;
                }
            }
            if left > 0 {
                pane.black = r.u32(p)?;
                p += 4;
                left -= 1;
            }
            if left > 0 {
                pane.white = r.u32(p)?;
                p += 4;
                left -= 1;
            }
            for k in 0..4 {
                if left > 0 {
                    pane.corners[k] = r.u32(p)?;
                    p += 4;
                    left -= 1;
                }
            }
            pane.texture = (!timg.is_empty()).then_some(timg);
        }
        Ok(pane)
    }

    /// The first pane tagged `tag` (`J2DScreen::search`).
    pub fn find(&self, tag: u32) -> Option<&Pane> {
        self.panes.iter().find(|p| p.tag == tag)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_are_right_aligned() {
        assert_eq!(tag("w_ba"), u32::from_be_bytes(*b"w_ba"));
        assert_eq!(tag("w_0"), u32::from_be_bytes(*b"\0w_0"));
    }

    /// A screen with a pane holding a picture.
    #[test]
    fn nested_picture_is_placed_from_its_parent() {
        let mut d = b"SCRNblo1".to_vec();
        d.resize(0x20, 0);
        d.extend_from_slice(b"INF1");
        d.extend_from_slice(&16u32.to_be_bytes());
        d.extend_from_slice(&600i16.to_be_bytes());
        d.extend_from_slice(&480i16.to_be_bytes());
        d.extend_from_slice(&0u32.to_be_bytes());
        let pane = |kind: &[u8; 4], tag: &[u8; 4], xy: (i16, i16), extra: &[u8]| {
            let mut b = Vec::new();
            b.extend_from_slice(kind);
            b.extend_from_slice(&0u32.to_be_bytes());
            b.extend_from_slice(&[6, 1, 0, 0]);
            b.extend_from_slice(tag);
            for v in [xy.0, xy.1, 10, 10] {
                b.extend_from_slice(&v.to_be_bytes());
            }
            b.extend_from_slice(extra);
            while b.len() % 4 != 0 {
                b.push(0);
            }
            let n = b.len() as u32;
            b[4..8].copy_from_slice(&n.to_be_bytes());
            b
        };
        let block = |kind: &[u8; 4]| {
            let mut b = kind.to_vec();
            b.extend_from_slice(&8u32.to_be_bytes());
            b
        };
        d.extend(pane(b"PAN1", b"\0w_0", (100, 200), &[]));
        d.extend(block(b"BGN1"));
        let pic = [&[3u8, 2, 4][..], b"a.bt", &[0, 0, 15]].concat();
        d.extend(pane(b"PIC1", b"w_ba", (5, 6), &pic));
        d.extend(block(b"END1"));
        d.extend(block(b"EXT1"));
        let s = Screen::parse(&d).unwrap();
        assert_eq!((s.width, s.height), (600, 480));
        let p = s.find(tag("w_ba")).unwrap();
        assert_eq!(p.at, [105, 206]);
        assert_eq!(p.texture.as_deref(), Some("a.bt"));
        assert_eq!(p.parent, Some(0));
    }
}
