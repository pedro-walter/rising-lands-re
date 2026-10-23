mod disc;

use rl_assets::{cmp::SpriteArchive, eta, lzs, pie, Palette};
use std::{env, error::Error, fs, fs::File, io::BufWriter, path::Path, process::ExitCode};

const USAGE: &str = "\
usage:
  rl-tool disc <image.bin> <out-dir>     extract game files and CD audio from a BIN/CUE image
  rl-tool dump <game-dir> <out-dir>      convert screens and sprites to PNG
                                         (<game-dir> is the INSTALHD folder)
  rl-tool map <game-dir> <file.eta> <out.png> [x y w h]
                                         render a scenario's terrain (cells, default 0 0 80 70)";

type Res<T = ()> = Result<T, Box<dyn Error>>;

fn main() -> ExitCode {
    run(&env::args().skip(1).collect::<Vec<_>>())
}

fn run(a: &[String]) -> ExitCode {
    let r = match a.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["disc", bin, out] => cmd_disc(bin.as_ref(), out.as_ref()),
        ["dump", game, out] => cmd_dump(game.as_ref(), out.as_ref()),
        ["map", game, eta, out] => cmd_map(game.as_ref(), eta.as_ref(), out.as_ref(), [0, 0, 80, 70]),
        ["map", game, eta, out, x, y, w, h] => {
            let r = [x, y, w, h].map(|v| v.parse::<usize>().unwrap_or(0));
            cmd_map(game.as_ref(), eta.as_ref(), out.as_ref(), r)
        }
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match r {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn cmd_disc(bin: &Path, out: &Path) -> Res {
    let disc = disc::Disc::open(bin)?;
    let n = disc.extract(out, &["DIRECTX"])?;
    println!("extracted {n} files to {}", out.display());
    // CD audio tracks come from the CUE sheet next to the image.
    let cue = bin.with_extension("CUE");
    let cue = if cue.exists() { cue } else { bin.with_extension("cue") };
    if cue.exists() {
        let t = disc.extract_audio(&cue, out)?;
        println!("extracted {t} audio tracks");
    }
    Ok(())
}

fn write_png(path: &Path, w: u32, h: u32, rgba: &[u8]) -> Res {
    let mut enc = png::Encoder::new(BufWriter::new(File::create(path)?), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()?.write_image_data(rgba)?;
    Ok(())
}

fn cmd_dump(game: &Path, out: &Path) -> Res {
    fs::create_dir_all(out.join("screens"))?;
    // The in-game palette is the one embedded in the loading screen.
    let charge = lzs::parse(&fs::read(game.join("CHARGE.LZS"))?)?;
    let game_palette: &Palette = &charge.palette;

    for e in fs::read_dir(game)? {
        let p = e?.path();
        let ext = p.extension().and_then(|x| x.to_str()).unwrap_or("").to_ascii_lowercase();
        let screen = match ext.as_str() {
            "pie" => {
                // The palette embedded in PIE files is not the one the game displays
                // them with; they are indexed against the global game palette.
                let mut s = pie::parse(&fs::read(&p)?)?;
                s.palette = game_palette.clone();
                s
            }
            "lzs" => lzs::parse(&fs::read(&p)?)?,
            _ => continue,
        };
        let stem = p.file_stem().unwrap().to_string_lossy().to_lowercase();
        write_png(&out.join("screens").join(format!("{stem}.png")), screen.width as u32, screen.height as u32, &screen.to_rgba())?;
        println!("screen {stem}");
    }

    for e in fs::read_dir(game.join("GRAPHICS"))? {
        let p = e?.path();
        let stem = p.file_stem().unwrap().to_string_lossy().to_lowercase();
        let dir = out.join("sprites").join(&stem);
        fs::create_dir_all(&dir)?;
        let arc = SpriteArchive::parse(&fs::read(&p)?)?;
        for (i, s) in arc.sprites.iter().enumerate() {
            let bm = s.decode()?;
            let mut rgba = Vec::with_capacity(bm.pixels.len() * 4);
            for (&px, &op) in bm.pixels.iter().zip(&bm.opaque) {
                // Index 3 is magenta (255,0,255), used by the art as a colour key.
                rgba.extend_from_slice(&if op { game_palette.rgba(px) } else { [0; 4] });
            }
            write_png(&dir.join(format!("{i:05}.png")), bm.width as u32, bm.height as u32, &rgba)?;
        }
        println!("sprites {stem}: {}", arc.sprites.len());
    }
    Ok(())
}

fn cmd_map(game: &Path, eta_path: &Path, out: &Path, [x0, y0, w, h]: [usize; 4]) -> Res {
    const T: usize = 32; // BLOCS tiles are 32x32
    let palette = lzs::parse(&fs::read(game.join("CHARGE.LZS"))?)?.palette;
    let blocs = SpriteArchive::parse(&fs::read(game.join("GRAPHICS").join("BLOCS.CMP"))?)?;
    let map = eta::Map::parse(&fs::read(eta_path)?)?;
    let (w, h) = (w.min(eta::MAP_SIZE - x0), h.min(eta::MAP_SIZE - y0));
    let mut img = vec![0u8; w * T * h * T * 4];
    for cy in 0..h {
        for cx in 0..w {
            let Some(s) = blocs.sprites.get(map.tile(x0 + cx, y0 + cy) as usize) else { continue };
            let bm = s.decode()?;
            for py in 0..bm.height as usize {
                for px in 0..bm.width as usize {
                    let i = py * bm.width as usize + px;
                    if bm.opaque[i] {
                        let o = ((cy * T + py) * w * T + cx * T + px) * 4;
                        img[o..o + 4].copy_from_slice(&palette.rgba(bm.pixels[i]));
                    }
                }
            }
        }
    }
    write_png(out, (w * T) as u32, (h * T) as u32, &img)?;
    println!("{} ({}) -> {}", eta_path.display(), map.version, out.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> std::path::PathBuf {
        let d = env::temp_dir().join(format!("rl-tool-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    /// CHARGE.LZS for a 2x1 image whose palette maps index i to (i, i, i); pixels are indices 1 and 2.
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

    /// A 32x32 tile (index 7 everywhere) in a one-sprite archive.
    fn blocs() -> Vec<u8> {
        let mut d = vec![1, 0, 32, 0, 32, 0];
        let mut data = Vec::new();
        for _ in 0..8 {
            data.push(0x80);
            data.extend([7u8; 128]);
        }
        d.extend_from_slice(&(data.len() as u32).to_le_bytes());
        d.extend(data);
        d
    }

    fn game_dir(name: &str) -> std::path::PathBuf {
        let d = tmp(name);
        fs::write(d.join("CHARGE.LZS"), charge()).unwrap();
        fs::create_dir_all(d.join("GRAPHICS")).unwrap();
        fs::write(d.join("GRAPHICS/BLOCS.CMP"), blocs()).unwrap();
        d
    }

    fn eta() -> Vec<u8> {
        let mut d = vec![0u8; 0x50030 + 8];
        d[..4].copy_from_slice(b"V0.3");
        d[0x5001c..0x5001e].copy_from_slice(&64u16.to_le_bytes());
        d[0x5001e..0x50020].copy_from_slice(&64u16.to_le_bytes());
        d
    }

    fn read_png(path: &Path) -> (u32, u32, Vec<u8>) {
        let dec = png::Decoder::new(File::open(path).unwrap());
        let mut r = dec.read_info().unwrap();
        let mut buf = vec![0; r.output_buffer_size()];
        let info = r.next_frame(&mut buf).unwrap();
        (info.width, info.height, buf[..info.buffer_size()].to_vec())
    }

    #[test]
    fn png_writer_round_trips() {
        let d = tmp("png");
        write_png(&d.join("a.png"), 2, 1, &[1, 2, 3, 255, 4, 5, 6, 0]).unwrap();
        assert_eq!(read_png(&d.join("a.png")), (2, 1, vec![1, 2, 3, 255, 4, 5, 6, 0]));
        assert!(write_png(&d.join("nodir/a.png"), 1, 1, &[0; 4]).is_err());
    }

    #[test]
    fn dump_converts_screens_and_sprites() {
        let g = game_dir("dump");
        // A PIE screen is shown with the game palette, not its own.
        let mut pie = b"Pie".to_vec();
        pie.extend_from_slice(&[1, 0, 1, 0, 0, 1]);
        pie.extend(std::iter::repeat(200).take(768));
        pie.extend_from_slice(&[0, 9]);
        fs::write(g.join("TITLE.PIE"), pie).unwrap();
        fs::write(g.join("readme.txt"), b"ignored").unwrap();
        let out = g.join("out");
        cmd_dump(&g, &out).unwrap();
        assert_eq!(read_png(&out.join("screens/charge.png")), (2, 1, vec![1, 1, 1, 255, 2, 2, 2, 255]));
        assert_eq!(read_png(&out.join("screens/title.png")).2, vec![9, 9, 9, 255]);
        let (w, h, px) = read_png(&out.join("sprites/blocs/00000.png"));
        assert_eq!((w, h, &px[..4]), (32, 32, &[7, 7, 7, 255][..]));
        assert!(!out.join("screens/readme.png").exists());
        // No CHARGE.LZS: a clear error.
        assert!(cmd_dump(&tmp("empty"), &out).is_err());
    }

    #[test]
    fn map_renders_tiles_and_clips_the_window() {
        let g = game_dir("map");
        fs::write(g.join("m.eta"), eta()).unwrap();
        let out = g.join("m.png");
        cmd_map(&g, &g.join("m.eta"), &out, [0, 0, 2, 1]).unwrap();
        let (w, h, px) = read_png(&out);
        assert_eq!((w, h), (64, 32));
        assert_eq!(&px[..4], &[7, 7, 7, 255]);
        // A window past the edge is clipped to the 256x256 grid.
        cmd_map(&g, &g.join("m.eta"), &out, [255, 255, 10, 10]).unwrap();
        assert_eq!(read_png(&out).0, 32);
        assert!(cmd_map(&g, &g.join("missing.eta"), &out, [0, 0, 1, 1]).is_err());
    }

    #[test]
    fn command_line_dispatch() {
        let g = game_dir("cli");
        fs::write(g.join("m.eta"), eta()).unwrap();
        let out = g.join("cli.png");
        let (g_s, e_s, o_s) = (g.to_str().unwrap(), g.join("m.eta"), out.to_str().unwrap().to_string());
        assert_eq!(run(&args(&[])), ExitCode::from(2));
        assert_eq!(run(&args(&["nope"])), ExitCode::from(2));
        assert_eq!(run(&args(&["map", g_s, e_s.to_str().unwrap(), &o_s])), ExitCode::SUCCESS);
        assert_eq!(run(&args(&["map", g_s, e_s.to_str().unwrap(), &o_s, "0", "0", "1", "1"])), ExitCode::SUCCESS);
        assert_eq!(read_png(&out).0, 32);
        assert_eq!(run(&args(&["dump", g_s, g.join("o").to_str().unwrap()])), ExitCode::SUCCESS);
        assert_eq!(run(&args(&["dump", "/nonexistent", "/tmp/x"])), ExitCode::FAILURE);
    }

    #[test]
    fn disc_command_extracts_files_and_audio_next_to_the_cue() {
        // An image with no ISO header fails cleanly.
        let d = tmp("disccmd");
        fs::write(d.join("x.bin"), vec![0u8; 20 * 2352]).unwrap();
        assert!(cmd_disc(&d.join("x.bin"), &d.join("out")).is_err());
        assert_eq!(run(&args(&["disc", d.join("x.bin").to_str().unwrap(), d.join("out").to_str().unwrap()])), ExitCode::FAILURE);
    }
}
