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
    let a: Vec<String> = env::args().skip(1).collect();
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
