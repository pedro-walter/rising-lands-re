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
