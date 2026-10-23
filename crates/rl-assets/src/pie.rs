//! `.PIE` screens: `"Pie"`, width, height, colour count (u16 each, LE),
//! a 256x RGB palette, then a PackBits-compressed pixel stream.
//!
//! The embedded palette is *not* what the game displays these with: pixels are
//! indexed against the global game palette (see [`crate::lzs`], `CHARGE.LZS`).

use crate::{u16le, Error, Palette, Screen};

pub fn parse(d: &[u8]) -> Result<Screen, Error> {
    if d.get(..3) != Some(b"Pie") {
        return Err(Error::BadMagic("PIE"));
    }
    let (w, h) = (u16le(d, 3)?, u16le(d, 5)?);
    let palette = Palette::from_bytes(d.get(9..).ok_or(Error::Truncated("PIE"))?)?;
    let total = w as usize * h as usize;
    let mut out = Vec::with_capacity(total);
    let mut p = 9 + 768;
    while out.len() < total {
        let c = *d.get(p).ok_or(Error::Truncated("PIE data"))?;
        p += 1;
        if c < 0x80 {
            let n = c as usize + 1;
            out.extend_from_slice(d.get(p..p + n).ok_or(Error::Truncated("PIE data"))?);
            p += n;
        } else {
            let v = *d.get(p).ok_or(Error::Truncated("PIE data"))?;
            p += 1;
            out.resize(out.len() + (257 - c as usize), v);
        }
    }
    out.truncate(total);
    Ok(Screen { width: w, height: h, palette, pixels: out })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(w: u16, h: u16, stream: &[u8]) -> Vec<u8> {
        let mut f = b"Pie".to_vec();
        f.extend_from_slice(&w.to_le_bytes());
        f.extend_from_slice(&h.to_le_bytes());
        f.extend_from_slice(&256u16.to_le_bytes());
        f.extend((0..768).map(|i| (i % 251) as u8));
        f.extend_from_slice(stream);
        f
    }

    #[test]
    fn packbits_literals_and_repeats() {
        // Literal run of 2, then a repeat of 9 (0xff = 257 - 255 = 2 copies).
        let s = parse(&file(2, 2, &[1, 5, 6, 0xff, 9])).unwrap();
        assert_eq!((s.width, s.height), (2, 2));
        assert_eq!(s.pixels, [5, 6, 9, 9]);
        assert_eq!(s.palette.0[1], [3, 4, 5]);
    }

    #[test]
    fn long_repeat_is_truncated_to_the_image() {
        assert_eq!(parse(&file(3, 1, &[0x80, 4])).unwrap().pixels, [4, 4, 4]);
    }

    #[test]
    fn errors() {
        assert!(matches!(parse(b"Pix"), Err(Error::BadMagic("PIE"))));
        assert!(parse(&file(1, 1, &[])[..100]).is_err());
        assert!(matches!(parse(&file(4, 1, &[3, 1, 2])), Err(Error::Truncated(_))));
        assert!(matches!(parse(&file(4, 1, &[0xff])), Err(Error::Truncated(_))));
    }
}
