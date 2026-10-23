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

    fn sector_count(&self) -> usize {
        self.raw.len() / RAW
    }

    fn raw_sector(&self, lba: usize) -> io::Result<&[u8]> {
        self.raw
            .get(lba * RAW..(lba + 1) * RAW)
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

    /// Writes each CD audio track named in a CUE sheet as `trackNN.raw` (44.1 kHz, 16-bit,
    /// stereo, little endian). Track lengths follow the table of contents, i.e. they run up to
    /// the next track's INDEX 01, like the lengths Windows reports for an audio CD.
    pub fn extract_audio(&self, cue: &Path, out: &Path) -> io::Result<usize> {
        let text = fs::read_to_string(cue)?;
        // (track number, is_audio, INDEX 01 sector)
        let mut tracks: Vec<(u32, bool, usize)> = Vec::new();
        for line in text.lines().map(str::trim) {
            let mut w = line.split_whitespace();
            match (w.next(), w.next(), w.next()) {
                (Some("TRACK"), Some(n), Some(kind)) => tracks.push((n.parse().unwrap_or(0), kind == "AUDIO", 0)),
                (Some("INDEX"), Some("01"), Some(msf)) => {
                    let p: Vec<usize> = msf.split(':').filter_map(|v| v.parse().ok()).collect();
                    if let (Some(t), [m, s, f]) = (tracks.last_mut(), p.as_slice()) {
                        t.2 = (m * 60 + s) * 75 + f;
                    }
                }
                _ => {}
            }
        }
        let total = self.sector_count();
        let mut n = 0;
        for (i, &(num, audio, start)) in tracks.iter().enumerate() {
            if !audio {
                continue;
            }
            let end = tracks.get(i + 1).map_or(total, |t| t.2);
            let mut data = Vec::with_capacity((end - start) * 2352);
            for lba in start..end {
                data.extend_from_slice(self.raw_sector(lba)?);
            }
            fs::write(out.join(format!("track{num:02}.raw")), data)?;
            n += 1;
        }
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("rl-disc-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn record(name: &[u8], lba: u32, len: u32, dir: bool) -> Vec<u8> {
        let mut r = vec![0u8; 33 + name.len() + (name.len() + 1) % 2];
        r[0] = r.len() as u8;
        r[2..6].copy_from_slice(&lba.to_le_bytes());
        r[10..14].copy_from_slice(&len.to_le_bytes());
        r[25] = if dir { 2 } else { 0 };
        r[32] = name.len() as u8;
        r[33..33 + name.len()].copy_from_slice(name);
        r
    }

    /// Mode 2 image, 45 sectors: ISO at the start, audio tracks at sectors 40..42 and 42..45.
    /// Root (2 sectors, 18-19): `.`, `..`, README.TXT;1, NOEXT.;1, SUB/, DIRECTX/ (second sector).
    fn image() -> Vec<u8> {
        let mut img = vec![0u8; 45 * RAW];
        let mut put = |lba: usize, data: &[u8]| img[lba * RAW + USER_OFFSET..lba * RAW + USER_OFFSET + data.len()].copy_from_slice(data);
        let mut pvd = vec![0u8; 190];
        pvd[1..6].copy_from_slice(b"CD001");
        pvd[156..190].copy_from_slice(&record(&[0], 18, 4096, true)[..34]);
        put(16, &pvd);
        let mut root1 = record(&[0], 18, 4096, true);
        root1.extend(record(&[1], 18, 4096, true));
        root1.extend(record(b"README.TXT;1", 20, 5, false));
        root1.extend(record(b"NOEXT.;1", 21, 3, false));
        put(18, &root1);
        let mut root2 = record(b"SUB", 22, 2048, true);
        root2.extend(record(b"DIRECTX", 24, 2048, true));
        put(19, &root2);
        put(20, b"hello");
        put(21, b"abc");
        put(22, &record(b"BIG.BIN;1", 25, 3000, false));
        put(24, &record(b"SKIPPED.TXT;1", 23, 1, false));
        put(23, b"x");
        let big: Vec<u8> = (0..3000u32).map(|i| (i % 251) as u8).collect();
        put(25, &big[..2048]);
        put(26, &big[2048..]);
        for lba in 40..45 {
            img[lba * RAW..(lba + 1) * RAW].fill(lba as u8);
        }
        img
    }

    fn disc(name: &str) -> (std::path::PathBuf, Disc) {
        let d = tmp(name);
        fs::write(d.join("game.bin"), image()).unwrap();
        let disc = Disc::open(&d.join("game.bin")).unwrap();
        (d, disc)
    }

    #[test]
    fn extracts_files_recursively_skipping_named_top_level_dirs() {
        let (d, disc) = disc("extract");
        let out = d.join("out");
        assert_eq!(disc.extract(&out, &["directx"]).unwrap(), 3);
        assert_eq!(fs::read(out.join("README.TXT")).unwrap(), b"hello");
        assert_eq!(fs::read(out.join("NOEXT")).unwrap(), b"abc", "version suffix and trailing dot are dropped");
        let big = fs::read(out.join("SUB/BIG.BIN")).unwrap();
        assert_eq!(big.len(), 3000);
        assert_eq!(big[2999], (2999 % 251) as u8, "files spanning sectors are joined");
        assert!(!out.join("DIRECTX").exists());
        // Without the skip list, the nested dir comes out too.
        assert_eq!(disc.extract(&d.join("all"), &[]).unwrap(), 4);
        fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn rejects_images_that_are_not_iso9660_or_are_too_short() {
        let d = tmp("bad");
        fs::write(d.join("a.bin"), vec![0u8; 20 * RAW]).unwrap();
        let e = Disc::open(&d.join("a.bin")).unwrap().extract(&d.join("o"), &[]).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::InvalidData);
        fs::write(d.join("b.bin"), vec![0u8; 100]).unwrap();
        let e = Disc::open(&d.join("b.bin")).unwrap().extract(&d.join("o"), &[]).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::UnexpectedEof);
        assert!(Disc::open(&d.join("missing.bin")).is_err());
        fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn extracts_audio_tracks_up_to_the_next_index() {
        let (d, disc) = disc("audio");
        let cue = d.join("game.cue");
        fs::write(&cue, "FILE \"game.bin\" BINARY\n  TRACK 01 MODE2/2352\n    INDEX 01 00:00:00\n  TRACK 02 AUDIO\n    INDEX 00 00:00:38\n    INDEX 01 00:00:40\n  TRACK 03 AUDIO\n    INDEX 01 00:00:42\n").unwrap();
        assert_eq!(disc.extract_audio(&cue, &d).unwrap(), 2);
        let t2 = fs::read(d.join("track02.raw")).unwrap();
        assert_eq!(t2.len(), 2 * RAW);
        assert_eq!((t2[0], t2[RAW]), (40, 41));
        let t3 = fs::read(d.join("track03.raw")).unwrap();
        assert_eq!(t3.len(), 3 * RAW, "the last track runs to the end of the image");
        assert!(!d.join("track01.raw").exists());
        // A track that starts beyond the image is an error, not a panic.
        fs::write(&cue, "TRACK 01 AUDIO\nINDEX 01 10:00:00\nTRACK 02 AUDIO\nINDEX 01 10:00:10\n").unwrap();
        assert!(disc.extract_audio(&cue, &d).is_err());
        fs::remove_dir_all(d).unwrap();
    }
}
