//! `.ETA` scenario maps (version `V0.3`), as read by the game at `FUN_0046c2cf`.
//!
//! The loader skips the 4-byte `V0.3` tag, then reads sequentially:
//!
//! | offset     | size    | meaning                                                         |
//! |------------|---------|-----------------------------------------------------------------|
//! | `0x00004`  | 64 KiB  | layer 0 (cell space starts at `0x00008`, see `LAYER0`): sparse object bytes                                    |
//! | `0x10004`  | 128 KiB | `u16` terrain tile index into `BLOCS.CMP`, row-major, 256 wide  |
//! | `0x30004`  | 64 KiB  | layer 2: ASCII terrain-class bytes (the loader rewrites `'6'` to `'B'`) |
//! | `0x40004`  | 64 KiB  | layer 3: sparse object bytes                                    |
//! | `0x50004`  | 24      | starting resources: 3 groups x 4 clans (`u16`), see [`Map::resources`] |
//! | `0x5001C`  | 4       | playable width, height (cells)                                  |
//! | `0x50020`  | 16      | per-clan start (camera top-left) x, y; only the local clan's is used |
//! | then       | ...     | `u16 count, u16 size`, then `count` x (`u16 slot`, `size` bytes): buildings (204 bytes) |
//! | then       | ...     | `u16 count, u16 size`, then `count` x (`u16 id`, `size` bytes): units. The record is 784 bytes in the file; bytes `0x8C..0x28C` are skipped by the loader. |
//!
//! Building record: `+2` u16 top-left cell index (`y * 256 + x`), `+4` clan, `+5` state,
//! `+8` u16 kind (order of the building sections in `RISING.INI`: 0 Phare, 1 Caserne,
//! 2 Sanctuaire, ...), `+10` build time, `+14` housing.
//!
//! Unit record (file offsets): `+0` clan, `+1` kind (index in `RISING.INI` unit order),
//! `+8`/`+9` cell x/y. Several units can share a cell when they are waiting inside a building.
//! The tables after the units (goals, events, ...) are not parsed yet.

use crate::{u16le, Error};

pub const MAP_SIZE: usize = 256;
/// File offset of layer 0 in cell space. The loader reads from byte 4, but terrain only lines up
/// with entity coordinates (e.g. a Sanctuary and its 6x5 stone pad) when 2 more cells are skipped.
const LAYER0: usize = 8;
const CELLS: usize = MAP_SIZE * MAP_SIZE;

#[derive(Clone, Debug)]
pub struct Map {
    pub version: String,
    /// Playable size in cells (64..=256); everything outside is filler terrain.
    pub width: usize,
    pub height: usize,
    pub layer0: Vec<u8>,
    /// Index into `BLOCS.CMP`, `MAP_SIZE * MAP_SIZE` entries.
    pub tiles: Vec<u16>,
    pub layer2: Vec<u8>,
    pub layer3: Vec<u8>,
    /// Starting stock per clan, indexed `[group][clan]`; groups are in file order
    /// (the loader stores them as resource types 1, 2, 0).
    pub resources: [[u16; 4]; 3],
    pub starts: [(u16, u16); 4],
    pub buildings: Vec<Building>,
    pub units: Vec<Unit>,
}

#[derive(Clone, Debug)]
pub struct Building {
    pub slot: u16,
    pub clan: u8,
    pub kind: u16,
    pub x: u8,
    pub y: u8,
    pub raw: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct Unit {
    pub id: u16,
    pub clan: u8,
    pub kind: u8,
    pub x: u8,
    pub y: u8,
    /// The 784-byte file record.
    pub raw: Vec<u8>,
}

impl Map {
    pub fn parse(d: &[u8]) -> Result<Self, Error> {
        if d.len() < 0x50020 || d.len() < LAYER0 + 0x50000 {
            return Err(Error::Truncated("ETA"));
        }
        if d.get(..2) != Some(b"V0") {
            return Err(Error::BadMagic("ETA"));
        }
        let version = String::from_utf8_lossy(&d[..4]).into_owned();
        let tiles = d[0x10000..0x30000]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect::<Vec<_>>();
        debug_assert_eq!(tiles.len(), CELLS);
        let (width, height) = (u16le(d, 0x5001c)? as usize, u16le(d, 0x5001e)? as usize);
        if !(1..=MAP_SIZE).contains(&width) || !(1..=MAP_SIZE).contains(&height) {
            return Err(Error::Corrupt("ETA map size"));
        }
        let mut resources = [[0u16; 4]; 3];
        for (g, row) in resources.iter_mut().enumerate() {
            for (c, v) in row.iter_mut().enumerate() {
                *v = u16le(d, 0x50004 + (g * 4 + c) * 2)?;
            }
        }
        let mut starts = [(0u16, 0u16); 4];
        for (c, st) in starts.iter_mut().enumerate() {
            *st = (u16le(d, 0x50020 + c * 4)?, u16le(d, 0x50022 + c * 4)?);
        }
        // Clan records, then units; each table is `count, size, count x (id, size bytes)`.
        let mut at = 0x50030;
        let (count, size) = (u16le(d, at)? as usize, u16le(d, at + 2)? as usize);
        at += 4;
        let mut buildings = Vec::with_capacity(count);
        for _ in 0..count {
            let raw = d.get(at + 2..at + 2 + size).ok_or(Error::Truncated("ETA buildings"))?;
            if size < 10 {
                return Err(Error::Corrupt("ETA building size"));
            }
            let cell = u16::from_le_bytes([raw[2], raw[3]]);
            buildings.push(Building {
                slot: u16le(d, at)?,
                clan: raw[4],
                kind: u16::from_le_bytes([raw[8], raw[9]]),
                x: cell as u8,
                y: (cell >> 8) as u8,
                raw: raw.to_vec(),
            });
            at += 2 + size;
        }
        let (count, size) = (u16le(d, at)? as usize, u16le(d, at + 2)? as usize);
        at += 4;
        let mut units = Vec::with_capacity(count);
        for _ in 0..count {
            let raw = d.get(at + 2..at + 2 + size).ok_or(Error::Truncated("ETA units"))?;
            if size < 10 {
                return Err(Error::Corrupt("ETA unit size"));
            }
            units.push(Unit { id: u16le(d, at)?, clan: raw[0], kind: raw[1], x: raw[8], y: raw[9], raw: raw.to_vec() });
            at += 2 + size;
        }
        Ok(Map {
            resources,
            starts,
            buildings,
            units,
            version,
            width,
            height,
            layer0: d[LAYER0..LAYER0 + 0x10000].to_vec(),
            tiles,
            layer2: d[LAYER0 + 0x30000..LAYER0 + 0x40000].to_vec(),
            layer3: d[LAYER0 + 0x40000..LAYER0 + 0x50000].to_vec(),
        })
    }

    pub fn tile(&self, x: usize, y: usize) -> u16 {
        self.tiles[y * MAP_SIZE + x]
    }
}
