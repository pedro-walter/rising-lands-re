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

#[cfg(test)]
mod tests {
    use super::*;

    fn file(w: u16, h: u16, data: &[u8]) -> Vec<u8> {
        let mut f = b"LZSS".to_vec();
        f.extend_from_slice(&w.to_le_bytes());
        f.extend_from_slice(&h.to_le_bytes());
        f.extend_from_slice(b"CMAP");
        f.extend((0..768).map(|i| (i / 3) as u8));
        f.extend_from_slice(b"FORM");
        f.extend_from_slice(&(data.len() as u32).to_le_bytes());
        f.extend_from_slice(data);
        f
    }

    #[test]
    fn literals_and_back_reference() {
        // Three literals, then a 5-byte copy from window position 4078 (overlapping its own output).
        let s = parse(&file(4, 2, &[0b0000_0111, b'a', b'b', b'c', 0xee, 0xf2])).unwrap();
        assert_eq!((s.width, s.height), (4, 2));
        assert_eq!(s.pixels, b"abcabcab");
        assert_eq!(s.palette.rgba(7), [7, 7, 7, 255]);
    }

    #[test]
    fn rejects_bad_headers_and_truncation() {
        assert!(matches!(parse(b"nope"), Err(Error::BadMagic("LZS"))));
        let mut f = file(1, 1, &[1, 0]);
        f[8] = b'X';
        assert!(matches!(parse(&f), Err(Error::BadMagic("LZS CMAP"))));
        let mut f = file(1, 1, &[1, 0]);
        f[780] = b'X';
        assert!(matches!(parse(&f), Err(Error::BadMagic("LZS FORM"))));
        assert!(matches!(parse(&file(4, 1, &[0xff, 1, 2])), Err(Error::Truncated(_))));
        assert!(matches!(parse(&file(4, 1, &[0x00, 1])), Err(Error::Truncated(_))));
        assert!(parse(&file(1, 1, &[1, 0])[..500]).is_err());
    }
}
