//! AVI playback for the intro and ending movies: Cinepak video and Microsoft ADPCM audio.

use std::time::Instant;

pub struct Avi {
    pub width: usize,
    pub height: usize,
    /// Frames per second as a ratio.
    pub rate: (u32, u32),
    pub frames: Vec<Vec<u8>>,
    pub audio: Vec<i16>,
    pub channels: u16,
    pub sample_rate: u32,
}

fn le16(d: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([d[o], d[o + 1]])
}
fn le32(d: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
}

impl Avi {
    pub fn parse(d: &[u8]) -> Result<Avi, String> {
        if d.len() < 12 || &d[..4] != b"RIFF" || &d[8..12] != b"AVI " {
            return Err("not an AVI file".into());
        }
        let mut avi = Avi { width: 0, height: 0, rate: (15, 1), frames: Vec::new(), audio: Vec::new(), channels: 1, sample_rate: 22050 };
        let (mut block_align, mut coefs) = (512usize, Vec::<(i32, i32)>::new());
        let (mut video_stream, mut audio_stream, mut streams) = (usize::MAX, usize::MAX, 0usize);
        let mut audio_raw: Vec<u8> = Vec::new();
        // Flatten the chunk tree: LIST bodies are scanned in place.
        let mut stack: Vec<(usize, usize)> = vec![(12, d.len())];
        while let Some((mut p, end)) = stack.pop() {
            while p + 8 <= end.min(d.len()) {
                let id = &d[p..p + 4];
                let size = le32(d, p + 4) as usize;
                let body = p + 8;
                let bend = (body + size).min(d.len());
                if id == b"LIST" {
                    // Process this list's children before the rest of the current level.
                    stack.push((bend + (size & 1), end));
                    stack.push((body + 4, bend));
                    break;
                }
                let b = &d[body..bend];
                match id {
                    b"avih" if b.len() >= 40 => {
                        avi.width = le32(b, 32) as usize;
                        avi.height = le32(b, 36) as usize;
                    }
                    b"strh" if b.len() >= 28 => {
                        let kind = &b[0..4];
                        if kind == b"vids" {
                            video_stream = streams;
                            avi.rate = (le32(b, 24), le32(b, 20).max(1));
                        } else if kind == b"auds" {
                            audio_stream = streams;
                        }
                        streams += 1;
                    }
                    b"strf" if streams > 0 && streams - 1 == audio_stream && b.len() >= 22 => {
                        let tag = le16(b, 0);
                        if tag != 2 {
                            return Err(format!("unsupported audio format {tag}"));
                        }
                        avi.channels = le16(b, 2);
                        avi.sample_rate = le32(b, 4);
                        block_align = le16(b, 12) as usize;
                        let ncoef = le16(b, 20) as usize;
                        coefs = (0..ncoef).filter(|i| 22 + i * 4 + 4 <= b.len()).map(|i| (le16(b, 22 + i * 4) as i16 as i32, le16(b, 24 + i * 4) as i16 as i32)).collect();
                    }
                    _ if id[0].is_ascii_digit() && id[1].is_ascii_digit() => {
                        let n = ((id[0] - b'0') * 10 + (id[1] - b'0')) as usize;
                        match &id[2..] {
                            b"dc" | b"db" if n == video_stream => avi.frames.push(b.to_vec()),
                            b"wb" if n == audio_stream => audio_raw.extend_from_slice(b),
                            _ => {}
                        }
                    }
                    _ => {}
                }
                p = body + size + (size & 1);
            }
        }
        if !coefs.is_empty() && !audio_raw.is_empty() {
            avi.audio = ms_adpcm(&audio_raw, avi.channels as usize, block_align, &coefs);
        }
        Ok(avi)
    }
}

/// Microsoft ADPCM to interleaved 16-bit PCM.
fn ms_adpcm(data: &[u8], ch: usize, block_align: usize, coefs: &[(i32, i32)]) -> Vec<i16> {
    const ADAPT: [i32; 16] = [230, 230, 230, 230, 307, 409, 512, 614, 768, 614, 512, 409, 307, 230, 230, 230];
    let mut out = Vec::new();
    for block in data.chunks(block_align) {
        let header = 7 * ch;
        if block.len() < header {
            break;
        }
        let mut pred_idx = [0usize; 2];
        let (mut delta, mut s1, mut s2) = ([0i32; 2], [0i32; 2], [0i32; 2]);
        let mut p = 0;
        for c in 0..ch {
            pred_idx[c] = (block[p] as usize).min(coefs.len() - 1);
            p += 1;
        }
        for c in 0..ch {
            delta[c] = le16(block, p) as i16 as i32;
            p += 2;
        }
        for c in 0..ch {
            s1[c] = le16(block, p) as i16 as i32;
            p += 2;
        }
        for c in 0..ch {
            s2[c] = le16(block, p) as i16 as i32;
            p += 2;
        }
        for c in 0..ch {
            out.push(s2[c] as i16);
        }
        for c in 0..ch {
            out.push(s1[c] as i16);
        }
        let mut nibble_ch = 0;
        for &b in &block[p..] {
            for nib in [b >> 4, b & 15] {
                let c = nibble_ch;
                nibble_ch = (nibble_ch + 1) % ch;
                let (c1, c2) = coefs[pred_idx[c]];
                let mut pred = (s1[c] * c1 + s2[c] * c2) / 256;
                let signed = if nib >= 8 { nib as i32 - 16 } else { nib as i32 };
                pred += signed * delta[c];
                let pred = pred.clamp(-32768, 32767);
                s2[c] = s1[c];
                s1[c] = pred;
                delta[c] = ((ADAPT[nib as usize] * delta[c]) / 256).max(16);
                out.push(pred as i16);
            }
        }
    }
    out
}

#[derive(Clone, Copy, Default)]
struct Entry {
    y: [u8; 4],
    u: i8,
    v: i8,
}

pub struct Cinepak {
    pub width: usize,
    pub height: usize,
    /// Persistent frame, 3 bytes per pixel (RGB).
    pub rgb: Vec<u8>,
    /// Codebooks per strip index; they persist across frames.
    books: Vec<[Vec<Entry>; 2]>,
    cur: usize,
}

impl Cinepak {
    pub fn new(width: usize, height: usize) -> Cinepak {
        Cinepak { width, height, rgb: vec![0; width * height * 3], books: vec![[vec![Entry::default(); 256], vec![Entry::default(); 256]]; 32], cur: 0 }
    }

    fn put(&mut self, x: usize, y: usize, e: &Entry, i: usize) {
        if x >= self.width || y >= self.height {
            return;
        }
        let yy = e.y[i] as i32;
        let (u, v) = (e.u as i32, e.v as i32);
        let o = (y * self.width + x) * 3;
        self.rgb[o] = (yy + v * 2).clamp(0, 255) as u8;
        self.rgb[o + 1] = (yy - (u >> 1) - v).clamp(0, 255) as u8;
        self.rgb[o + 2] = (yy + u * 2).clamp(0, 255) as u8;
    }

    /// Decodes one frame into `self.rgb`.
    pub fn decode(&mut self, d: &[u8]) -> Result<(), String> {
        if d.len() < 10 {
            return Err("short cinepak frame".into());
        }
        let strips = (u16::from_be_bytes([d[8], d[9]]) as usize).min(32);
        let flags = d[0];
        let (mut p, mut y0) = (10usize, 0usize);
        for si in 0..strips {
            self.cur = si;
            if si > 0 && flags & 1 == 0 {
                self.books[si] = self.books[si - 1].clone();
            }
            if p + 12 > d.len() {
                return Err("truncated strip".into());
            }
            let size = u16::from_be_bytes([d[p + 2], d[p + 3]]) as usize;
            let (ty, ay2) = (u16::from_be_bytes([d[p + 4], d[p + 5]]) as usize, u16::from_be_bytes([d[p + 8], d[p + 9]]) as usize);
            let (y1, y2) = (y0, y0 + ay2.saturating_sub(ty));
            let end = (p + size).min(d.len());
            let mut q = p + 12;
            while q + 4 <= end {
                let id = u16::from_be_bytes([d[q], d[q + 1]]);
                let csize = u16::from_be_bytes([d[q + 2], d[q + 3]]) as usize;
                let body = &d[(q + 4).min(end)..(q + csize).min(end)];
                match id {
                    0x2000 | 0x2100 | 0x2400 | 0x2500 => self.codebook(body, true, id & 0x100 != 0, id >= 0x2400),
                    0x2200 | 0x2300 | 0x2600 | 0x2700 => self.codebook(body, false, id & 0x100 != 0, id >= 0x2400),
                    0x3000 | 0x3100 | 0x3200 => self.vectors(id, body, y1, y2),
                    _ => {}
                }
                q += csize.max(4);
            }
            y0 = y2;
            p = end;
        }
        Ok(())
    }

    fn codebook(&mut self, body: &[u8], v4: bool, partial: bool, mono: bool) {
        let esize = if mono { 4 } else { 6 };
        let book = &mut self.books[self.cur][v4 as usize];
        let mut p = 0;
        let read = |p: &mut usize| -> Option<Entry> {
            let s = body.get(*p..*p + esize)?;
            *p += esize;
            Some(Entry { y: [s[0], s[1], s[2], s[3]], u: if mono { 0 } else { s[4] as i8 }, v: if mono { 0 } else { s[5] as i8 } })
        };
        if !partial {
            let mut i = 0;
            while i < 256 {
                match read(&mut p) {
                    Some(e) => book[i] = e,
                    None => break,
                }
                i += 1;
            }
        } else {
            let mut i = 0;
            while i < 256 {
                let Some(fb) = body.get(p..p + 4) else { break };
                let flags = u32::from_be_bytes([fb[0], fb[1], fb[2], fb[3]]);
                p += 4;
                for bit in (0..32).rev() {
                    if i >= 256 {
                        break;
                    }
                    if flags >> bit & 1 != 0 {
                        match read(&mut p) {
                            Some(e) => book[i] = e,
                            None => return,
                        }
                    }
                    i += 1;
                }
            }
        }
    }

    fn vectors(&mut self, id: u16, body: &[u8], y1: usize, y2: usize) {
        let (mut p, mut flags, mut mask) = (0usize, 0u32, 0u32);
        let take_flags = |p: &mut usize, flags: &mut u32, mask: &mut u32| -> bool {
            match body.get(*p..*p + 4) {
                Some(b) => {
                    *flags = u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
                    *p += 4;
                    *mask = 0x8000_0000;
                    true
                }
                None => false,
            }
        };
        let odd = id & 0x100 != 0; // 0x3100: inter coded (blocks may be skipped)
        let v1_only = id & 0x200 != 0; // 0x3200
        let mut y = y1;
        while y < y2 {
            let mut x = 0;
            while x < self.width {
                if odd {
                    mask >>= 1;
                    if mask == 0 && !take_flags(&mut p, &mut flags, &mut mask) {
                        return;
                    }
                }
                if !odd || flags & mask != 0 {
                    if !v1_only {
                        mask >>= 1;
                        if mask == 0 && !take_flags(&mut p, &mut flags, &mut mask) {
                            return;
                        }
                    }
                    if v1_only || flags & mask == 0 {
                        let Some(&i) = body.get(p) else { return };
                        p += 1;
                        let e = self.books[self.cur][0][i as usize];
                        for by in 0..4 {
                            for bx in 0..4 {
                                self.put(x + bx, y + by, &e, (by / 2) * 2 + bx / 2);
                            }
                        }
                    } else {
                        for q in 0..4 {
                            let Some(&i) = body.get(p) else { return };
                            p += 1;
                            let e = self.books[self.cur][1][i as usize];
                            let (qx, qy) = ((q & 1) * 2, (q >> 1) * 2);
                            for k in 0..4 {
                                self.put(x + qx + (k & 1), y + qy + (k >> 1), &e, k);
                            }
                        }
                    }
                }
                x += 4;
            }
            y += 4;
        }
    }
}

/// A movie being played: the frame to show is `frame_rgb`.
pub struct Player {
    pub avi: Avi,
    pub dec: Cinepak,
    pub start: Instant,
    pub next: usize,
    pub finished: bool,
    pub dirty: bool,
    pub voice: Option<u32>,
}

impl Player {
    pub fn new(avi: Avi) -> Player {
        let dec = Cinepak::new(avi.width, avi.height);
        Player { avi, dec, start: Instant::now(), next: 0, finished: false, dirty: false, voice: None }
    }

    /// Decodes frames that are due; call regularly.
    pub fn tick(&mut self) {
        if self.finished {
            return;
        }
        let secs = self.start.elapsed().as_secs_f64();
        let due = (secs * self.avi.rate.0 as f64 / self.avi.rate.1 as f64) as usize + 1;
        while self.next < due && self.next < self.avi.frames.len() {
            let _ = self.dec.decode(&self.avi.frames[self.next]);
            self.next += 1;
            self.dirty = true;
        }
        if self.next >= self.avi.frames.len() && due > self.avi.frames.len() + 1 {
            self.finished = true;
        }
    }
}
