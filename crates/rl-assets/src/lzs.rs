//! `.LZS` screens: `"LZSS"`, width, height, `"CMAP"` + 256x RGB palette,
//! then `"FORM"` + little-endian u32 length + Okumura-style LZSS
//! (4096-byte window, initial write position 4078, LSB-first flags).

use crate::{u16le, Error, Palette, Screen};

pub fn parse(d: &[u8]) -> Result<Screen, Error> {
    if d.get(..4) != Some(b"LZSS") {
        return Err(Error::BadMagic("LZS"));
    }
    let (w, h) = (u16le(d, 4)?, u16le(d, 6)?);
    if d.get(8..12) != Some(b"CMAP") {
        return Err(Error::BadMagic("LZS CMAP"));
    }
    let palette = Palette::from_bytes(d.get(12..).ok_or(Error::Truncated("LZS"))?)?;
    if d.get(780..784) != Some(b"FORM") {
        return Err(Error::BadMagic("LZS FORM"));
    }
    let pixels = lzss(d.get(788..).ok_or(Error::Truncated("LZS"))?, w as usize * h as usize)?;
    Ok(Screen { width: w, height: h, palette, pixels })
}

fn lzss(src: &[u8], total: usize) -> Result<Vec<u8>, Error> {
    let mut win = [0u8; 4096];
    let mut r = 4096 - 18;
    let mut out = Vec::with_capacity(total);
    let mut p = 0;
    while out.len() < total {
        let flags = *src.get(p).ok_or(Error::Truncated("LZS data"))?;
        p += 1;
        for bit in 0..8 {
            if out.len() >= total {
                break;
            }
            if flags >> bit & 1 != 0 {
                let c = *src.get(p).ok_or(Error::Truncated("LZS data"))?;
                p += 1;
                out.push(c);
                win[r] = c;
                r = (r + 1) & 4095;
            } else {
                let (i, j) = match src.get(p..p + 2) {
                    Some(s) => (s[0] as usize, s[1] as usize),
                    None => return Err(Error::Truncated("LZS data")),
                };
                p += 2;
                let off = i | ((j & 0xf0) << 4);
                for k in 0..(j & 0x0f) + 3 {
                    let c = win[(off + k) & 4095];
                    out.push(c);
                    win[r] = c;
                    r = (r + 1) & 4095;
                }
            }
        }
    }
    out.truncate(total);
    Ok(out)
}
