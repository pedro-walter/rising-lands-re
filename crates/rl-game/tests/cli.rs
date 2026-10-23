//! Runs the `rl-game` binary in its windowless `--shot` mode against generated assets.

use std::{path::PathBuf, process::Command};

fn solid_cmp(w: u16, h: u16, v: u8) -> Vec<u8> {
    let mut tokens = Vec::new();
    let mut left = w as usize * h as usize;
    while left > 0 {
        let n = left.min(128);
        tokens.push((256 - n) as u8);
        tokens.extend(std::iter::repeat(v).take(n));
        left -= n;
    }
    let mut d = 1u16.to_le_bytes().to_vec();
    d.extend_from_slice(&w.to_le_bytes());
    d.extend_from_slice(&h.to_le_bytes());
    d.extend_from_slice(&(tokens.len() as u32).to_le_bytes());
    d.extend(tokens);
    d
}

/// CHARGE.LZS: 2x1 image, palette entry i = (i, i, i).
fn charge() -> Vec<u8> {
    let mut f = b"LZSS".to_vec();
    f.extend_from_slice(&[2, 0, 1, 0]);
    f.extend_from_slice(b"CMAP");
    f.extend((0..768).map(|i| (i / 3) as u8));
    f.extend_from_slice(b"FORM");
    f.extend_from_slice(&3u32.to_le_bytes());
    f.extend_from_slice(&[0b11, 1, 2]);
    f
}

fn eta() -> Vec<u8> {
    let mut d = vec![0u8; 0x50030 + 8];
    d[..4].copy_from_slice(b"V0.3");
    d[0x5001c..0x5001e].copy_from_slice(&64u16.to_le_bytes());
    d[0x5001e..0x50020].copy_from_slice(&64u16.to_le_bytes());
    d
}

struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Dir {
        let root = std::env::temp_dir().join(format!("rl-game-cli-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("GRAPHICS")).unwrap();
        std::fs::write(root.join("CHARGE.LZS"), charge()).unwrap();
        std::fs::write(root.join("GRAPHICS/BLOCS.CMP"), solid_cmp(32, 32, 7)).unwrap();
        std::fs::write(root.join("GRAPHICS/BUILDING.CMP"), solid_cmp(8, 8, 9)).unwrap();
        std::fs::write(root.join("m.eta"), eta()).unwrap();
        Dir(root)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn game(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rl-game")).args(args).env("SDL_VIDEODRIVER", "dummy").env("SDL_AUDIODRIVER", "dummy").output().unwrap()
}

fn size(png_path: &std::path::Path) -> (u32, u32) {
    let r = png::Decoder::new(std::fs::File::open(png_path).unwrap()).read_info().unwrap();
    (r.info().width, r.info().height)
}

#[test]
fn without_arguments_it_prints_usage() {
    let o = game(&[]);
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("usage: rl-game"));
}

#[test]
fn shot_mode_renders_one_frame_to_a_png() {
    let d = Dir::new("shot");
    let (dir, map, out) = (d.0.to_str().unwrap(), d.0.join("m.eta"), d.0.join("out.png"));
    let o = game(&[dir, map.to_str().unwrap(), "--shot", out.to_str().unwrap()]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(size(&out), (800, 600));
    // Starting cell given: the camera is clamped to the map instead of failing.
    let o = game(&[dir, map.to_str().unwrap(), "--shot", out.to_str().unwrap(), "500", "500"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let o = game(&[dir, map.to_str().unwrap(), "--shot", out.to_str().unwrap(), "x", "1"]);
    assert!(!o.status.success(), "a non-numeric cell is an error");
}

#[test]
fn missing_inputs_are_reported() {
    let d = Dir::new("missing");
    let dir = d.0.to_str().unwrap();
    let o = game(&[dir, d.0.join("nope.eta").to_str().unwrap(), "--shot", d.0.join("o.png").to_str().unwrap()]);
    assert!(!o.status.success());
    let o = game(&[d.0.join("nope").to_str().unwrap(), d.0.join("m.eta").to_str().unwrap()]);
    assert!(!o.status.success());
    let o = game(&[dir, d.0.join("m.eta").to_str().unwrap(), "--shot"]);
    assert!(!o.status.success(), "--shot without a file");
}
