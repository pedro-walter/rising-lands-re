//! Decodes every asset from a local copy of the game data.
//! Skipped unless `RL_DATA` points at the `INSTALHD` directory.

use rl_assets::{cmp::SpriteArchive, lzs, pie};
use std::{fs, path::PathBuf};

fn data_dir() -> Option<PathBuf> {
    std::env::var_os("RL_DATA").map(PathBuf::from)
}

#[test]
fn all_sprites_decode() {
    let Some(dir) = data_dir() else { return };
    for e in fs::read_dir(dir.join("GRAPHICS")).unwrap() {
        let path = e.unwrap().path();
        let arc = SpriteArchive::parse(&fs::read(&path).unwrap()).unwrap();
        for (i, s) in arc.sprites.iter().enumerate() {
            s.decode().unwrap_or_else(|e| panic!("{path:?} #{i}: {e}"));
        }
    }
}

#[test]
fn all_screens_decode() {
    let Some(dir) = data_dir() else { return };
    for e in fs::read_dir(&dir).unwrap() {
        let path = e.unwrap().path();
        let ext = path.extension().and_then(|x| x.to_str()).unwrap_or("").to_ascii_lowercase();
        let d = fs::read(&path).unwrap_or_default();
        match ext.as_str() {
            "pie" => assert_eq!(pie::parse(&d).unwrap().pixels.len(), 800 * 600),
            "lzs" => assert_eq!(lzs::parse(&d).unwrap().pixels.len(), 800 * 600),
            _ => {}
        }
    }
}

#[test]
fn rules_parse() {
    let Some(dir) = std::env::var_os("RL_DATA") else { return };
    let bytes = std::fs::read(std::path::Path::new(&dir).join("RISING.INI")).unwrap();
    let ini = rl_assets::ini::Ini::parse(&bytes);
    let soldier = ini.section("SOLDAT").expect("SOLDAT section");
    assert_eq!(soldier.int("RAYON"), Some(7));
    assert_eq!(soldier.ints("DEGAT").map(|d| d.len()), Some(27));
    assert!(ini.sections.len() > 80);
}
