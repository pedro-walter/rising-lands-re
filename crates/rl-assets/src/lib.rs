//! Readers for the original *Rising Lands* data formats.
//!
//! Everything here decodes into plain 8-bit indexed pixels plus a [`Palette`];
//! turning those into RGBA or textures is the caller's job.

pub mod cmp;
pub mod eta;
pub mod ini;
pub mod lzs;
pub mod pie;

use std::fmt;

/// 256-entry RGB palette (8 bits per channel).
#[derive(Clone, PartialEq, Eq)]
pub struct Palette(pub [[u8; 3]; 256]);

impl Palette {
    pub(crate) fn from_bytes(b: &[u8]) -> Result<Self, Error> {
        if b.len() < 768 {
            return Err(Error::Truncated("palette"));
        }
        let mut p = [[0u8; 3]; 256];
        for (i, e) in p.iter_mut().enumerate() {
            e.copy_from_slice(&b[i * 3..i * 3 + 3]);
        }
        Ok(Palette(p))
    }

    pub fn rgba(&self, index: u8) -> [u8; 4] {
        let [r, g, b] = self.0[index as usize];
        [r, g, b, 255]
    }
}

impl fmt::Debug for Palette {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Palette(..)")
    }
}

/// A full-screen paletted image (`.PIE` / `.LZS`).
#[derive(Clone, Debug)]
pub struct Screen {
    pub width: u16,
    pub height: u16,
    pub palette: Palette,
    /// `width * height` palette indices, row-major.
    pub pixels: Vec<u8>,
}

impl Screen {
    pub fn to_rgba(&self) -> Vec<u8> {
        self.pixels.iter().flat_map(|&i| self.palette.rgba(i)).collect()
    }
}

#[derive(Debug)]
pub enum Error {
    BadMagic(&'static str),
    Truncated(&'static str),
    Corrupt(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::BadMagic(w) => write!(f, "bad magic in {w}"),
            Error::Truncated(w) => write!(f, "truncated {w}"),
            Error::Corrupt(w) => write!(f, "corrupt {w}"),
        }
    }
}

impl std::error::Error for Error {}

pub(crate) fn u16le(b: &[u8], at: usize) -> Result<u16, Error> {
    b.get(at..at + 2)
        .map(|s| u16::from_le_bytes([s[0], s[1]]))
        .ok_or(Error::Truncated("header"))
}

pub(crate) fn u32le(b: &[u8], at: usize) -> Result<u32, Error> {
    b.get(at..at + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        .ok_or(Error::Truncated("header"))
}
