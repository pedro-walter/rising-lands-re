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
