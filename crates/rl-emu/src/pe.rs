//! Minimal PE32 loader: maps sections and builds the import list.

use std::error::Error;

pub struct Import {
    pub dll: String,
    pub name: String,
    /// Guest address of the IAT slot to fill in.
    pub slot: u32,
}

pub struct Image {
    pub base: u32,
    pub size: u32,
    pub entry: u32,
    /// Everything to copy into guest memory: (address, bytes).
    pub chunks: Vec<(u32, Vec<u8>)>,
    pub imports: Vec<Import>,
}

fn u16at(d: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([d[o], d[o + 1]])
}
fn u32at(d: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
}

fn cstr(d: &[u8], o: usize) -> String {
    let end = d[o..].iter().position(|&b| b == 0).map_or(d.len(), |p| o + p);
    String::from_utf8_lossy(&d[o..end]).into_owned()
}

pub fn load(d: &[u8]) -> Result<Image, Box<dyn Error>> {
    if d.get(..2) != Some(b"MZ") {
        return Err("not an MZ executable".into());
    }
    let pe = u32at(d, 0x3c) as usize;
    if d.get(pe..pe + 4) != Some(b"PE\0\0") {
        return Err("missing PE signature".into());
    }
    let nsec = u16at(d, pe + 6) as usize;
    let opt_size = u16at(d, pe + 20) as usize;
    let opt = pe + 24;
    if u16at(d, opt) != 0x10b {
        return Err("not a PE32 image".into());
    }
    let entry_rva = u32at(d, opt + 16);
    let base = u32at(d, opt + 28);
    let size = u32at(d, opt + 56);
    let hdr_size = u32at(d, opt + 60) as usize;
    let import_rva = u32at(d, opt + 96 + 8) as usize;

    let mut chunks = vec![(base, d[..hdr_size.min(d.len())].to_vec())];
    let mut secs = Vec::new();
    for i in 0..nsec {
        let s = opt + opt_size + i * 40;
        let vsize = u32at(d, s + 8) as usize;
        let va = u32at(d, s + 12) as usize;
        let rsize = u32at(d, s + 16) as usize;
        let raw = u32at(d, s + 20) as usize;
        // Uninitialised sections (raw pointer 0) stay zero-filled.
        let n = if raw == 0 { 0 } else { rsize.min(d.len().saturating_sub(raw)) };
        chunks.push((base + va as u32, d[raw..raw + n].to_vec()));
        secs.push((va, vsize.max(rsize), raw, rsize));
    }
    // RVA -> file offset, for reading the import tables out of the raw file.
    let off = |rva: usize| -> Option<usize> {
        secs.iter().find(|&&(va, vs, _, _)| rva >= va && rva < va + vs).map(|&(va, _, raw, _)| raw + rva - va)
    };

    let mut imports = Vec::new();
    let mut desc = off(import_rva).ok_or("no import table")?;
    loop {
        let (oft, name, ft) = (u32at(d, desc) as usize, u32at(d, desc + 12) as usize, u32at(d, desc + 16) as usize);
        if name == 0 {
            break;
        }
        let dll = cstr(d, off(name).ok_or("bad import dll name")?);
        let mut t = off(if oft != 0 { oft } else { ft }).ok_or("bad thunk")?;
        let mut idx = 0;
        loop {
            let v = u32at(d, t);
            if v == 0 {
                break;
            }
            let name = if v & 0x8000_0000 != 0 {
                format!("#{}", v & 0xffff)
            } else {
                cstr(d, off(v as usize + 2).ok_or("bad import name")?)
            };
            imports.push(Import { dll: dll.clone(), name, slot: base + ft as u32 + idx * 4 });
            t += 4;
            idx += 1;
        }
        desc += 20;
    }
    Ok(Image { base, size, entry: base + entry_rva, chunks, imports })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put32(d: &mut [u8], at: usize, v: u32) {
        d[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }

    /// PE32 at base 0x400000 with a `.text` (RVA 0x1000, raw 0x200) and an `.idata` (RVA 0x2000, raw 0x400)
    /// importing `Foo` and ordinal 5 from `K.DLL`, plus an uninitialised `.bss` (RVA 0x3000).
    fn image() -> Vec<u8> {
        let mut d = vec![0u8; 0x600];
        d[..2].copy_from_slice(b"MZ");
        put32(&mut d, 0x3c, 0x40);
        d[0x40..0x44].copy_from_slice(b"PE\0\0");
        d[0x46..0x48].copy_from_slice(&3u16.to_le_bytes()); // sections
        d[0x54..0x56].copy_from_slice(&0xe0u16.to_le_bytes()); // optional header size
        let opt = 0x58;
        d[opt..opt + 2].copy_from_slice(&0x10bu16.to_le_bytes());
        put32(&mut d, opt + 16, 0x1010); // entry
        put32(&mut d, opt + 28, 0x40_0000); // base
        put32(&mut d, opt + 56, 0x4000); // image size
        put32(&mut d, opt + 60, 0x200); // headers
        put32(&mut d, opt + 104, 0x2000); // import directory RVA
        let sec = |d: &mut Vec<u8>, i: usize, vsize: u32, va: u32, rsize: u32, raw: u32| {
            let s = opt + 0xe0 + i * 40;
            put32(d, s + 8, vsize);
            put32(d, s + 12, va);
            put32(d, s + 16, rsize);
            put32(d, s + 20, raw);
        };
        sec(&mut d, 0, 0x100, 0x1000, 0x200, 0x200);
        sec(&mut d, 1, 0x200, 0x2000, 0x200, 0x400);
        sec(&mut d, 2, 0x1000, 0x3000, 0, 0);
        d[0x200] = 0xc3;
        // Import descriptor at RVA 0x2000 (raw 0x400): OFT 0x2040, name 0x2080, FT 0x2090; terminator after it.
        put32(&mut d, 0x400, 0x2040);
        put32(&mut d, 0x400 + 12, 0x2080);
        put32(&mut d, 0x400 + 16, 0x2090);
        put32(&mut d, 0x440, 0x2060); // OFT[0]: by name
        put32(&mut d, 0x444, 0x8000_0005); // OFT[1]: ordinal 5
        d[0x462..0x466].copy_from_slice(b"Foo\0"); // hint u16 + name
        d[0x480..0x486].copy_from_slice(b"K.DLL\0");
        d
    }

    #[test]
    fn maps_sections_and_lists_imports() {
        let img = load(&image()).unwrap();
        assert_eq!((img.base, img.size, img.entry), (0x40_0000, 0x4000, 0x40_1010));
        assert_eq!(img.chunks.len(), 4);
        assert_eq!(img.chunks[0].1.len(), 0x200, "headers");
        assert_eq!(img.chunks[1].0, 0x40_1000);
        assert_eq!(img.chunks[1].1[0], 0xc3);
        assert!(img.chunks[3].1.is_empty(), "bss has no file bytes");
        let names: Vec<_> = img.imports.iter().map(|i| (i.dll.as_str(), i.name.as_str(), i.slot)).collect();
        assert_eq!(names, [("K.DLL", "Foo", 0x40_2090), ("K.DLL", "#5", 0x40_2094)]);
    }

    #[test]
    fn rejects_malformed_images() {
        let msg = |d: &[u8]| load(d).err().unwrap().to_string();
        assert_eq!(msg(b"ZM"), "not an MZ executable");
        let mut d = image();
        d[0x40] = b'X';
        assert_eq!(msg(&d), "missing PE signature");
        let mut d = image();
        d[0x58] = 0x0b;
        d[0x59] = 0x02; // PE32+
        assert_eq!(msg(&d), "not a PE32 image");
        let mut d = image();
        put32(&mut d, 0x58 + 104, 0x9000);
        assert_eq!(msg(&d), "no import table");
        let mut d = image();
        put32(&mut d, 0x400 + 12, 0x9000);
        assert_eq!(msg(&d), "bad import dll name");
        let mut d = image();
        put32(&mut d, 0x440, 0x9000);
        assert_eq!(msg(&d), "bad import name");
        let mut d = image();
        put32(&mut d, 0x400, 0x9000);
        assert_eq!(msg(&d), "bad thunk");
    }

    #[test]
    fn falls_back_to_the_first_thunk_when_the_lookup_table_is_empty() {
        let mut d = image();
        put32(&mut d, 0x400, 0); // no OFT: the IAT itself holds the names
        put32(&mut d, 0x490, 0x2060);
        let img = load(&d).unwrap();
        assert_eq!(img.imports.len(), 1);
        assert_eq!(img.imports[0].name, "Foo");
    }
}

/// Builds tiny executables for tests that run real guest code.
#[cfg(test)]
pub mod testing {
    /// Guest address of the IAT slot of the `i`-th import passed to [`exe`].
    pub fn slot(i: usize) -> u32 {
        0x40_2100 + 4 * i as u32
    }

    /// A PE32 with `code` at 0x401000 (the entry point) and the named imports (all from `KERNEL32.dll`;
    /// names like `#2` import by ordinal). The image spans 0x400000..0x404000; 0x403000 is free scratch.
    pub fn exe(code: &[u8], imports: &[&str]) -> Vec<u8> {
        assert!(code.len() <= 0x1000 && imports.len() < 20);
        let mut d = vec![0u8; 0x1600];
        let put32 = |d: &mut [u8], at: usize, v: u32| d[at..at + 4].copy_from_slice(&v.to_le_bytes());
        d[..2].copy_from_slice(b"MZ");
        put32(&mut d, 0x3c, 0x40);
        d[0x40..0x44].copy_from_slice(b"PE\0\0");
        d[0x46..0x48].copy_from_slice(&2u16.to_le_bytes());
        d[0x54..0x56].copy_from_slice(&0xe0u16.to_le_bytes());
        let opt = 0x58;
        d[opt..opt + 2].copy_from_slice(&0x10bu16.to_le_bytes());
        put32(&mut d, opt + 16, 0x1000);
        put32(&mut d, opt + 28, 0x40_0000);
        put32(&mut d, opt + 56, 0x4000);
        put32(&mut d, opt + 60, 0x200);
        put32(&mut d, opt + 104, 0x2000);
        for (i, (va, raw, size)) in [(0x1000u32, 0x200u32, 0x1000u32), (0x2000, 0x1200, 0x400)].into_iter().enumerate() {
            let s = opt + 0xe0 + i * 40;
            put32(&mut d, s + 8, size);
            put32(&mut d, s + 12, va);
            put32(&mut d, s + 16, size);
            put32(&mut d, s + 20, raw);
        }
        d[0x200..0x200 + code.len()].copy_from_slice(code);
        // .idata: descriptor at 0x2000, thunks at 0x2100, DLL name at 0x2180, hint/name entries from 0x2200.
        put32(&mut d, 0x1200, 0x2100);
        put32(&mut d, 0x1200 + 12, 0x2180);
        put32(&mut d, 0x1200 + 16, 0x2100);
        d[0x1380..0x138d].copy_from_slice(b"KERNEL32.dll\0");
        for (i, name) in imports.iter().enumerate() {
            let thunk = 0x1300 + i * 4;
            if let Some(ord) = name.strip_prefix('#') {
                put32(&mut d, thunk, 0x8000_0000 | ord.parse::<u32>().unwrap());
            } else {
                let rva = 0x2200 + i * 0x20;
                put32(&mut d, thunk, rva as u32);
                let at = 0x1200 + (rva - 0x2000) + 2;
                d[at..at + name.len()].copy_from_slice(name.as_bytes());
            }
        }
        d
    }
}
