//! `.CMP` sprite archives: `u16` count, then `count` records of
//! `{ u16 width, u16 height, u32 size, size bytes }`.
//!
//! Each sprite is a token stream laid over a `width * height` raster, filled
//! row-major (tokens may wrap across rows):
//!
//! * `b < 0x80`  — skip `b` transparent pixels
//! * `b >= 0x80` — copy the next `256 - b` bytes as palette indices
//!
//! Sprites carry no palette; the game palette is the one stored in `CHARGE.LZS`.

use crate::{u16le, u32le, Error};

#[derive(Clone, Debug)]
pub struct Sprite {
    pub width: u16,
    pub height: u16,
    data: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct SpriteArchive {
    pub sprites: Vec<Sprite>,
}

/// Decoded sprite: palette indices plus an opacity mask.
#[derive(Clone, Debug)]
pub struct Bitmap {
    pub width: u16,
    pub height: u16,
    /// `width * height` palette indices (0 where transparent).
    pub pixels: Vec<u8>,
    /// `width * height` flags, `true` where the sprite has a pixel.
    pub opaque: Vec<bool>,
}

impl SpriteArchive {
    pub fn parse(d: &[u8]) -> Result<Self, Error> {
        let n = u16le(d, 0)? as usize;
        let mut p = 2;
        let mut sprites = Vec::with_capacity(n);
        for _ in 0..n {
            let (width, height) = (u16le(d, p)?, u16le(d, p + 2)?);
            let size = u32le(d, p + 4)? as usize;
            p += 8;
            let data = d.get(p..p + size).ok_or(Error::Truncated("CMP sprite"))?.to_vec();
            p += size;
            sprites.push(Sprite { width, height, data });
        }
        if p != d.len() {
            return Err(Error::Corrupt("CMP: trailing bytes"));
        }
        Ok(SpriteArchive { sprites })
    }
}

impl Sprite {
    pub fn decode(&self) -> Result<Bitmap, Error> {
        let total = self.width as usize * self.height as usize;
        let mut pixels = vec![0u8; total];
        let mut opaque = vec![false; total];
        let (mut p, mut pos) = (0, 0);
        while p < self.data.len() {
            let t = self.data[p];
            p += 1;
            if t < 0x80 {
                pos += t as usize;
            } else {
                let n = 256 - t as usize;
                let src = self.data.get(p..p + n).ok_or(Error::Truncated("CMP run"))?;
                let dst = pixels.get_mut(pos..pos + n).ok_or(Error::Corrupt("CMP overrun"))?;
                dst.copy_from_slice(src);
                opaque[pos..pos + n].fill(true);
                p += n;
                pos += n;
            }
        }
        if pos != total {
            return Err(Error::Corrupt("CMP: pixel count mismatch"));
        }
        Ok(Bitmap { width: self.width, height: self.height, pixels, opaque })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn archive(sprites: &[(u16, u16, &[u8])]) -> Vec<u8> {
        let mut d = (sprites.len() as u16).to_le_bytes().to_vec();
        for (w, h, data) in sprites {
            d.extend_from_slice(&w.to_le_bytes());
            d.extend_from_slice(&h.to_le_bytes());
            d.extend_from_slice(&(data.len() as u32).to_le_bytes());
            d.extend_from_slice(data);
        }
        d
    }

    #[test]
    fn decodes_skips_and_runs() {
        // skip 1, copy 2 (0xfe = 256 - 2), skip 1.
        let a = SpriteArchive::parse(&archive(&[(4, 1, &[1, 0xfe, 7, 8, 1])])).unwrap();
        let b = a.sprites[0].decode().unwrap();
        assert_eq!((b.width, b.height), (4, 1));
        assert_eq!(b.pixels, [0, 7, 8, 0]);
        assert_eq!(b.opaque, [false, true, true, false]);
    }

    #[test]
    fn tokens_wrap_across_rows() {
        let a = SpriteArchive::parse(&archive(&[(2, 2, &[0xfd, 1, 2, 3, 1])])).unwrap();
        assert_eq!(a.sprites[0].decode().unwrap().pixels, [1, 2, 3, 0]);
    }

    #[test]
    fn archive_errors() {
        assert!(SpriteArchive::parse(&[]).is_err());
        assert!(matches!(SpriteArchive::parse(&[1, 0, 1, 0, 1, 0, 9, 0, 0, 0]), Err(Error::Truncated(_))));
        let mut d = archive(&[(1, 1, &[0xff, 5])]);
        d.push(0);
        assert!(matches!(SpriteArchive::parse(&d), Err(Error::Corrupt(_))));
        assert_eq!(SpriteArchive::parse(&archive(&[])).unwrap().sprites.len(), 0);
    }

    #[test]
    fn sprite_errors() {
        let dec = |w, h, data: &[u8]| SpriteArchive::parse(&archive(&[(w, h, data)])).unwrap().sprites[0].decode();
        assert!(matches!(dec(4, 1, &[0xfe, 7]), Err(Error::Truncated(_))));
        assert!(matches!(dec(1, 1, &[0xfe, 7, 8]), Err(Error::Corrupt(_))));
        assert!(matches!(dec(4, 1, &[1, 0xff, 7]), Err(Error::Corrupt(_))));
    }
}
