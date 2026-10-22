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
