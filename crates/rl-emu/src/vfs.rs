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
    /// Writable per-user directory layered over `C:`: saves and settings go here.
    pub overlay: PathBuf,
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
    pub fn new(install: PathBuf, disc: PathBuf, overlay: PathBuf) -> Vfs {
        Vfs { overlay, install, disc, cwd: "C:\\RISING".into(), files: HashMap::new(), next_handle: 0x100 }
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

    fn walk(root: &Path, comps: &[String]) -> Option<PathBuf> {
        let mut cur = root.to_path_buf();
        for c in comps {
            cur = lookup(&cur, c)?;
        }
        Some(cur)
    }

    /// Resolves a Windows path to an existing host path (the per-user overlay wins over `C:`).
    pub fn resolve(&self, path: &str) -> Option<PathBuf> {
        let (drive, comps) = self.normalize(path);
        match drive {
            'C' => {
                // `C:\RISING` is the install dir itself.
                let skip = usize::from(comps.first().is_some_and(|c| c.eq_ignore_ascii_case("RISING")));
                Self::walk(&self.overlay, &comps[skip..]).or_else(|| Self::walk(&self.install, &comps[skip..]))
            }
            'D' => Self::walk(&self.disc, &comps),
            _ => None,
        }
    }

    /// Host path in the overlay for a `C:` path, creating parent directories.
    fn overlay_path(&self, path: &str) -> Option<PathBuf> {
        let (drive, comps) = self.normalize(path);
        if drive != 'C' {
            return None;
        }
        let skip = usize::from(comps.first().is_some_and(|c| c.eq_ignore_ascii_case("RISING")));
        let comps = &comps[skip..];
        let (last, dir) = comps.split_last()?;
        let mut cur = self.overlay.clone();
        for c in dir {
            cur = lookup(&cur, c).unwrap_or_else(|| cur.join(c));
        }
        fs::create_dir_all(&cur).ok()?;
        Some(lookup(&cur, last).unwrap_or_else(|| cur.join(last)))
    }

    pub fn open(&mut self, path: &str, write: bool, create: bool) -> Option<u32> {
        let f = if write || create {
            let dst = self.overlay_path(path)?;
            if !dst.exists() && !create {
                // Copy-on-write: start from the installed file.
                fs::copy(self.resolve(path)?, &dst).ok()?;
            }
            fs::OpenOptions::new().read(true).write(true).create(create).truncate(create).open(dst).ok()?
        } else {
            File::open(self.patched(self.resolve(path)?)).ok()?
        };
        let h = self.next_handle;
        self.next_handle += 4;
        self.files.insert(h, f);
        Some(h)
    }

    /// Text corrections applied to installed files when they are read: (file name, wrong, right),
    /// all CP1252 bytes. The corrected copy lives in the overlay; the install is never touched.
    const TEXT_FIXES: &'static [(&'static str, &'static [u8], &'static [u8])] = &[
        // Mission 1 asks for a storage with 50 food and 50 stone, not for protecting a cave.
        (
            "monde01.txt",
            b"- Proteja uma caverna.\r\n- Estoque 50 sacos de comida e\r\n50 sacos de pedras na caverna.",
            b"- Construa um armaz\xe9m.\r\n- Estoque 50 sacos de comida e\r\n50 sacos de pedras no armaz\xe9m.",
        ),
        // A technology theft that rolls well but finds nothing to steal showed no message (see
        // `patch_silent_tech_theft` in machine.rs); message 503 is that case.
        (
            "events.ini",
            b"502 = A tentativa de roubo fracassou!!!\r\n",
            b"502 = A tentativa de roubo fracassou!!!\r\n503 = O roubo n\xe3o rendeu nada: n\xe3o h\xe1 tecnologia a roubar!\r\n",
        ),
    ];

    /// The installed file at `host` as the game should read it (with `TEXT_FIXES` applied).
    pub fn read_path(&self, host: PathBuf) -> PathBuf {
        self.patched(host)
    }

    fn patched(&self, src: PathBuf) -> PathBuf {
        let name = src.file_name().and_then(|n| n.to_str()).unwrap_or("").to_ascii_lowercase();
        let fixes: Vec<_> = Self::TEXT_FIXES.iter().filter(|f| f.0 == name).collect();
        if fixes.is_empty() {
            return src;
        }
        let Ok(mut data) = fs::read(&src) else { return src };
        let mut changed = false;
        for (_, from, to) in fixes {
            if let Some(i) = data.windows(from.len()).position(|w| w == *from) {
                data.splice(i..i + from.len(), to.iter().copied());
                changed = true;
            }
        }
        let dir = self.overlay.join(".textfix");
        let dst = dir.join(&name);
        if changed && fs::create_dir_all(&dir).is_ok() && fs::write(&dst, data).is_ok() {
            dst
        } else {
            src
        }
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
        let mut entries: Vec<PathBuf> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let (drive, comps) = self.normalize(dir);
        let mut dirs = Vec::new();
        if drive == 'C' {
            let skip = usize::from(comps.first().is_some_and(|c| c.eq_ignore_ascii_case("RISING")));
            dirs.extend(Self::walk(&self.overlay, &comps[skip..]));
            dirs.extend(Self::walk(&self.install, &comps[skip..]));
        } else {
            dirs.extend(self.resolve(dir));
        }
        for d in dirs {
            if let Ok(rd) = fs::read_dir(d) {
                for e in rd.flatten() {
                    let name = e.file_name().to_string_lossy().to_ascii_lowercase();
                    if pattern_match(pat.as_bytes(), name.as_bytes()) && seen.insert(name) {
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

#[cfg(test)]
mod tests {
    use super::*;

    struct Dirs {
        root: PathBuf,
        vfs: Vfs,
    }

    impl Drop for Dirs {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    /// install: `Maps/Level1.TXT`, `Rising.INI`; disc: `Data/Movie.AVI`; empty overlay.
    fn setup(name: &str) -> Dirs {
        let root = std::env::temp_dir().join(format!("rl-vfs-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for d in ["install/Maps", "disc/Data", "user"] {
            fs::create_dir_all(root.join(d)).unwrap();
        }
        fs::write(root.join("install/Maps/Level1.TXT"), b"one").unwrap();
        fs::write(root.join("install/Rising.INI"), b"ini").unwrap();
        fs::write(root.join("disc/Data/Movie.AVI"), b"avi").unwrap();
        let vfs = Vfs::new(root.join("install"), root.join("disc"), root.join("user"));
        Dirs { root, vfs }
    }

    #[test]
    fn wildcard_patterns() {
        assert!(pattern_match(b"*.txt", b"level1.txt"));
        assert!(pattern_match(b"LEVEL?.TXT", b"level1.txt"));
        assert!(pattern_match(b"*", b""));
        assert!(pattern_match(b"a*b*c", b"axxbyyc"));
        assert!(!pattern_match(b"*.txt", b"level1.ini"));
        assert!(!pattern_match(b"a?", b"a"));
        assert!(!pattern_match(b"abc", b"abcd"));
    }

    #[test]
    fn normalizes_paths_against_the_cwd() {
        let d = setup("norm");
        let n = |p| d.vfs.normalize(p);
        assert_eq!(n("c:\\Rising\\Maps"), ('C', vec!["Rising".into(), "Maps".into()]));
        assert_eq!(n("Maps/x.txt"), ('C', vec!["RISING".into(), "Maps".into(), "x.txt".into()]));
        assert_eq!(n("D:\\a\\.\\b\\..\\c"), ('D', vec!["a".into(), "c".into()]));
        assert_eq!(n("\\x"), ('C', vec!["x".into()]));
        assert_eq!(n("..\\x"), ('C', vec!["x".into()]));
    }

    #[test]
    fn resolves_case_insensitively_across_drives() {
        let d = setup("resolve");
        let v = &d.vfs;
        assert_eq!(v.resolve("C:\\RISING\\maps\\LEVEL1.txt"), Some(d.root.join("install/Maps/Level1.TXT")));
        assert_eq!(v.resolve("rising.ini"), Some(d.root.join("install/Rising.INI")));
        assert_eq!(v.resolve("D:\\data\\movie.avi"), Some(d.root.join("disc/Data/Movie.AVI")));
        assert_eq!(v.resolve("C:\\RISING"), Some(d.root.join("user")), "the overlay root wins when it exists");
        assert_eq!(v.resolve("C:\\nope.txt"), None);
        assert_eq!(v.resolve("E:\\x"), None);
    }

    #[test]
    fn reads_and_closes_handles() {
        let mut d = setup("read");
        let h = d.vfs.open("Rising.ini", false, false).unwrap();
        let mut s = String::new();
        std::io::Read::read_to_string(d.vfs.file(h).unwrap(), &mut s).unwrap();
        assert_eq!(s, "ini");
        assert!(d.vfs.close(h));
        assert!(!d.vfs.close(h));
        assert!(d.vfs.file(h).is_none());
        assert!(d.vfs.open("missing.ini", false, false).is_none());
        let (a, b) = (d.vfs.open("Rising.ini", false, false).unwrap(), d.vfs.open("Rising.ini", false, false).unwrap());
        assert_ne!(a, b);
    }

    #[test]
    fn writes_go_to_the_overlay_copy_on_write() {
        let mut d = setup("cow");
        let h = d.vfs.open("C:\\RISING\\Maps\\level1.txt", true, false).unwrap();
        std::io::Write::write_all(d.vfs.file(h).unwrap(), b"TWO").unwrap();
        d.vfs.close(h);
        assert_eq!(fs::read(d.root.join("install/Maps/Level1.TXT")).unwrap(), b"one", "install untouched");
        assert_eq!(fs::read(d.root.join("user/Maps/level1.txt")).unwrap(), b"TWO", "the overlay copy keeps the name the guest asked for");
        // The overlay now wins on reads.
        let h = d.vfs.open("Maps\\Level1.TXT", false, false).unwrap();
        let mut s = String::new();
        std::io::Read::read_to_string(d.vfs.file(h).unwrap(), &mut s).unwrap();
        assert_eq!(s, "TWO");
        // The disc is read-only: no overlay path for it, and no writable open.
        assert!(d.vfs.open("D:\\Data\\Movie.AVI", true, false).is_none());
        assert!(d.vfs.open("nothing.sav", true, false).is_none(), "cannot copy a file that does not exist");
    }

    #[test]
    fn create_truncates_and_makes_directories() {
        let mut d = setup("create");
        let h = d.vfs.open("C:\\RISING\\Saves\\a.sav", true, true).unwrap();
        std::io::Write::write_all(d.vfs.file(h).unwrap(), b"first").unwrap();
        d.vfs.close(h);
        let h = d.vfs.open("saves\\A.SAV", true, true).unwrap();
        std::io::Write::write_all(d.vfs.file(h).unwrap(), b"2").unwrap();
        d.vfs.close(h);
        assert_eq!(fs::read(d.root.join("user/Saves/a.sav")).unwrap(), b"2");
        assert!(d.vfs.open("D:\\new.bin", true, true).is_none());
    }

    #[test]
    fn find_merges_overlay_and_install_without_duplicates() {
        let mut d = setup("find");
        fs::write(d.root.join("user/extra.ini"), b"x").unwrap();
        fs::write(d.root.join("user/RISING.INI"), b"override").unwrap();
        let names = |f: Find| f.entries.iter().map(|p| p.file_name().unwrap().to_string_lossy().to_ascii_lowercase()).collect::<Vec<_>>();
        assert_eq!(names(d.vfs.find("C:\\RISING\\*.ini")), ["rising.ini", "extra.ini"]); // sorted by host path: RISING.INI < extra.ini
        assert_eq!(names(d.vfs.find("Maps\\level?.txt")), ["level1.txt"]);
        assert_eq!(names(d.vfs.find("D:\\Data\\*.*")), ["movie.avi"]);
        assert_eq!(names(d.vfs.find("D:\\Data\\*")), ["movie.avi"]);
        assert!(d.vfs.find("C:\\RISING\\*.zzz").entries.is_empty());
        assert_eq!(d.vfs.find("*.ini").next, 0);
        d.vfs.cwd = "C:\\RISING\\Maps".into();
        assert_eq!(names(d.vfs.find("*.txt")), ["level1.txt"]);
    }

    #[test]
    fn text_fixes_are_applied_on_read_without_touching_the_install() {
        let mut d = setup("fix");
        let original = b"Missao\r\n- Proteja uma caverna.\r\n- Estoque 50 sacos de comida e\r\n50 sacos de pedras na caverna.\r\nfim";
        fs::write(d.root.join("install/MONDE01.TXT"), original).unwrap();
        let h = d.vfs.open("monde01.txt", false, false).unwrap();
        let mut s = Vec::new();
        std::io::Read::read_to_end(d.vfs.file(h).unwrap(), &mut s).unwrap();
        let s = String::from_utf8_lossy(&s).into_owned();
        assert!(s.contains("Construa um armaz"), "{s}");
        assert!(!s.contains("caverna"));
        assert_eq!(fs::read(d.root.join("install/MONDE01.TXT")).unwrap(), original);
        // A file with no matching text is read straight from the install.
        fs::write(d.root.join("install/monde02.txt"), b"nothing to fix").unwrap();
        assert_eq!(d.vfs.patched(d.root.join("install/monde02.txt")), d.root.join("install/monde02.txt"));
        fs::write(d.root.join("install/other.txt"), b"x").unwrap();
        assert_eq!(d.vfs.patched(d.root.join("install/other.txt")), d.root.join("install/other.txt"));
    }
}
