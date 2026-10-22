//! Minimal reader for a raw `MODE2/2352` BIN image holding an ISO 9660 filesystem.

use std::{fs, io, path::Path};

const RAW: usize = 2352;
const USER_OFFSET: usize = 24; // sync(12) + header(4) + subheader(8) for Mode 2 Form 1

pub struct Disc {
    raw: Vec<u8>,
}

struct Entry {
    name: String,
    lba: usize,
    len: usize,
    dir: bool,
}

impl Disc {
    pub fn open(bin: &Path) -> io::Result<Self> {
        Ok(Disc { raw: fs::read(bin)? })
    }

    fn sector(&self, lba: usize) -> io::Result<&[u8]> {
        let at = lba * RAW + USER_OFFSET;
        self.raw
            .get(at..at + 2048)
            .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "sector beyond image"))
    }

    fn read(&self, lba: usize, len: usize) -> io::Result<Vec<u8>> {
        let mut out = Vec::with_capacity(len);
        let mut s = lba;
        while out.len() < len {
            let sec = self.sector(s)?;
            out.extend_from_slice(&sec[..(len - out.len()).min(2048)]);
            s += 1;
        }
        Ok(out)
    }

    fn list(&self, lba: usize, len: usize) -> io::Result<Vec<Entry>> {
        let d = self.read(lba, len)?;
        let (mut p, mut v) = (0, Vec::new());
        while p < d.len() {
            let n = d[p] as usize;
            if n == 0 {
                p = (p / 2048 + 1) * 2048; // records never straddle sectors
                continue;
            }
            let rec = &d[p..p + n];
            let le32 = |o: usize| u32::from_le_bytes(rec[o..o + 4].try_into().unwrap()) as usize;
            let id = &rec[33..33 + rec[32] as usize];
            if id != [0] && id != [1] {
                let name = String::from_utf8_lossy(id);
                let name = name.split(';').next().unwrap().trim_end_matches('.').to_string();
                v.push(Entry { name, lba: le32(2), len: le32(10), dir: rec[25] & 2 != 0 });
            }
            p += n;
        }
        Ok(v)
    }

    /// Extract every file under the root, skipping top-level directories named in `skip`.
    pub fn extract(&self, out: &Path, skip: &[&str]) -> io::Result<usize> {
        let pvd = self.sector(16)?;
        if &pvd[1..6] != b"CD001" {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "not an ISO 9660 disc"));
        }
        let root = &pvd[156..190];
        let (lba, len) = (
            u32::from_le_bytes(root[2..6].try_into().unwrap()) as usize,
            u32::from_le_bytes(root[10..14].try_into().unwrap()) as usize,
        );
        self.extract_dir(lba, len, out, skip, true)
    }

    fn extract_dir(&self, lba: usize, len: usize, out: &Path, skip: &[&str], top: bool) -> io::Result<usize> {
        fs::create_dir_all(out)?;
        let mut n = 0;
        for e in self.list(lba, len)? {
            if e.dir {
                if top && skip.iter().any(|s| s.eq_ignore_ascii_case(&e.name)) {
                    continue;
                }
                n += self.extract_dir(e.lba, e.len, &out.join(&e.name), skip, false)?;
            } else {
                fs::write(out.join(&e.name), self.read(e.lba, e.len)?)?;
                n += 1;
            }
        }
        Ok(n)
    }
}
