//! Host file access for the guest, with Windows-style paths.
//!
//! `C:` is the installed game directory and `D:` is the CD (the extracted disc root).
//! Lookups are case-insensitive, as on Windows.

use std::{
    collections::HashMap,
    fs::{self, File},
    path::{Path, PathBuf},
};

pub struct Vfs {
    pub install: PathBuf,
    pub disc: PathBuf,
    /// Current directory, as an absolute Windows path (`C:\RISING`).
    pub cwd: String,
    files: HashMap<u32, File>,
    next_handle: u32,
}

pub struct Find {
    pub entries: Vec<PathBuf>,
    pub next: usize,
}

fn pattern_match(pat: &[u8], name: &[u8]) -> bool {
    match (pat.first(), name.first()) {
        (None, None) => true,
        (Some(b'*'), _) => pattern_match(&pat[1..], name) || (!name.is_empty() && pattern_match(pat, &name[1..])),
        (Some(b'?'), Some(_)) => pattern_match(&pat[1..], &name[1..]),
        (Some(p), Some(n)) if p.eq_ignore_ascii_case(n) => pattern_match(&pat[1..], &name[1..]),
        _ => false,
    }
}

impl Vfs {
    pub fn new(install: PathBuf, disc: PathBuf) -> Vfs {
        Vfs { install, disc, cwd: "C:\\RISING".into(), files: HashMap::new(), next_handle: 0x100 }
    }

    /// Splits a Windows path into (drive letter, components) after applying the cwd.
    fn normalize(&self, path: &str) -> (char, Vec<String>) {
        let p = path.replace('/', "\\");
        let (drive, rest) = match p.as_bytes() {
            [d, b':', ..] => (d.to_ascii_uppercase() as char, &p[2..]),
            _ => (self.cwd.chars().next().unwrap_or('C'), &p[..]),
        };
        let mut comps: Vec<String> = Vec::new();
        if !rest.starts_with('\\') {
            // Relative: start from the cwd (when it is on the same drive).
            let cwd_rest = &self.cwd[self.cwd.find(':').map_or(0, |i| i + 1)..];
            comps.extend(cwd_rest.split('\\').filter(|c| !c.is_empty()).map(String::from));
        }
        for c in rest.split('\\') {
            match c {
                "" | "." => {}
                ".." => {
                    comps.pop();
                }
                c => comps.push(c.to_string()),
            }
        }
        (drive, comps)
    }

    /// Resolves a Windows path to an existing host path.
    pub fn resolve(&self, path: &str) -> Option<PathBuf> {
        let (drive, comps) = self.normalize(path);
        let mut cur = match drive {
            'C' => self.install.clone(),
            'D' => self.disc.clone(),
            _ => return None,
        };
        // `C:\RISING` is the install dir itself.
        let skip = usize::from(drive == 'C' && comps.first().is_some_and(|c| c.eq_ignore_ascii_case("RISING")));
        for c in &comps[skip..] {
            cur = lookup(&cur, c)?;
        }
        Some(cur)
    }

    /// Resolves a path whose last component may not exist yet (for creating files).
    pub fn resolve_new(&self, path: &str) -> Option<PathBuf> {
        if let Some(p) = self.resolve(path) {
            return Some(p);
        }
        let (drive, comps) = self.normalize(path);
        let (last, dir) = comps.split_last()?;
        let mut cur = match drive {
            'C' => self.install.clone(),
            _ => return None,
        };
        let skip = usize::from(dir.first().is_some_and(|c| c.eq_ignore_ascii_case("RISING")));
        for c in &dir[skip..] {
            cur = lookup(&cur, c)?;
        }
        Some(cur.join(last))
    }

    pub fn open(&mut self, path: &str, write: bool, create: bool) -> Option<u32> {
        let host = if create { self.resolve_new(path)? } else { self.resolve(path)? };
        let f = if write || create {
            fs::OpenOptions::new().read(true).write(true).create(create).truncate(create).open(host).ok()?
        } else {
            File::open(host).ok()?
        };
        let h = self.next_handle;
        self.next_handle += 4;
        self.files.insert(h, f);
        Some(h)
    }

    pub fn file(&mut self, h: u32) -> Option<&mut File> {
        self.files.get_mut(&h)
    }

    pub fn close(&mut self, h: u32) -> bool {
        self.files.remove(&h).is_some()
    }

    pub fn find(&self, pattern: &str) -> Find {
        let p = pattern.replace('/', "\\");
        let (dir, pat) = p.rsplit_once('\\').unwrap_or((".", &p));
        let dir = if dir.is_empty() { "\\" } else { dir };
        let mut entries = Vec::new();
        if let Some(d) = self.resolve(dir) {
            if let Ok(rd) = fs::read_dir(d) {
                for e in rd.flatten() {
                    if pattern_match(pat.as_bytes(), e.file_name().to_string_lossy().as_bytes()) {
                        entries.push(e.path());
                    }
                }
            }
        }
        entries.sort();
        Find { entries, next: 0 }
    }
}

fn lookup(dir: &Path, name: &str) -> Option<PathBuf> {
    let exact = dir.join(name);
    if exact.exists() {
        return Some(exact);
    }
    fs::read_dir(dir).ok()?.flatten().find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(name)).map(|e| e.path())
}
