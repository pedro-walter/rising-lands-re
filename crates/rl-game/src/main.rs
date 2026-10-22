//! Scrolling map viewer: the first playable-engine milestone.
//!
//! Game logic advances in fixed `TICK_HZ` steps, decoupled from rendering, so
//! the speed no longer depends on the machine (the original ran uncapped).

use rl_assets::{cmp::SpriteArchive, eta, lzs};
use sdl2::{event::Event, keyboard::Scancode, pixels::PixelFormatEnum};
use std::{env, error::Error, fs, path::Path, time::{Duration, Instant}};

const SCREEN_W: usize = 800;
const SCREEN_H: usize = 600;
const TILE: usize = 32;
const TICK_HZ: u32 = 30;
/// Camera speed in pixels per tick.
const SCROLL: i32 = 16;

/// Building footprint in cells (w, h), indexed by building kind (`RISING.INI` order).
const FOOTPRINT: [(i32, i32); 21] = [
    (3, 4), (5, 6), (6, 5), (5, 5), (3, 4), (6, 6), (6, 5), (4, 6), (6, 6), (1, 1), (2, 3),
    (2, 3), (1, 1), (4, 5), (4, 6), (4, 5), (4, 5), (2, 2), (4, 5), (3, 4), (4, 5),
];

/// Index in `BUILDING.CMP` of the finished-building sprite for a kind.
fn building_sprite(kind: u16) -> Option<usize> {
    match kind {
        0 => Some(47),
        1 => Some(1),
        2 => Some(3),
        _ => None,
    }
}

struct Img {
    w: usize,
    h: usize,
    rgba: Vec<u8>,
}

/// A `.CMP` archive with lazily decoded RGBA sprites.
struct Sheet {
    archive: SpriteArchive,
    cache: Vec<Option<Img>>,
    palette: rl_assets::Palette,
}

impl Sheet {
    fn load(game: &Path, name: &str, palette: &rl_assets::Palette) -> Result<Sheet, Box<dyn Error>> {
        let archive = SpriteArchive::parse(&fs::read(game.join("GRAPHICS").join(name))?)?;
        Ok(Sheet { cache: (0..archive.sprites.len()).map(|_| None).collect(), archive, palette: palette.clone() })
    }

    fn get(&mut self, i: usize) -> Option<&Img> {
        if i >= self.cache.len() {
            return None;
        }
        if self.cache[i].is_none() {
            let bm = self.archive.sprites[i].decode().ok()?;
            let (w, h) = (bm.width as usize, bm.height as usize);
            let mut rgba = vec![0u8; w * h * 4];
            for (p, (&px, &op)) in bm.pixels.iter().zip(&bm.opaque).enumerate() {
                if op {
                    rgba[p * 4..][..4].copy_from_slice(&self.palette.rgba(px));
                }
            }
            self.cache[i] = Some(Img { w, h, rgba });
        }
        self.cache[i].as_ref()
    }
}

fn blit(fb: &mut [u8], img: &Img, x: i32, y: i32) {
    for row in 0..img.h as i32 {
        let sy = y + row;
        if !(0..SCREEN_H as i32).contains(&sy) {
            continue;
        }
        for col in 0..img.w as i32 {
            let sx = x + col;
            let s = (row as usize * img.w + col as usize) * 4;
            if (0..SCREEN_W as i32).contains(&sx) && img.rgba[s + 3] != 0 {
                fb[(sy as usize * SCREEN_W + sx as usize) * 4..][..4].copy_from_slice(&img.rgba[s..s + 4]);
            }
        }
    }
}

/// 3x5 digit glyphs, one row per nibble, used for placeholder unit markers.
const DIGITS: [[u8; 5]; 10] = [
    [7, 5, 5, 5, 7], [2, 6, 2, 2, 7], [7, 1, 7, 4, 7], [7, 1, 7, 1, 7], [5, 5, 7, 1, 1],
    [7, 4, 7, 1, 7], [7, 4, 7, 5, 7], [7, 1, 1, 1, 1], [7, 5, 7, 5, 7], [7, 5, 7, 1, 7],
];
const CLAN_COLORS: [[u8; 4]; 4] = [[60, 120, 255, 255], [255, 70, 70, 255], [250, 210, 40, 255], [70, 220, 90, 255]];

fn put(fb: &mut [u8], x: i32, y: i32, c: [u8; 4]) {
    if (0..SCREEN_W as i32).contains(&x) && (0..SCREEN_H as i32).contains(&y) {
        fb[(y as usize * SCREEN_W + x as usize) * 4..][..4].copy_from_slice(&c);
    }
}

/// Placeholder until unit sprites are mapped: a clan-coloured box with the unit kind number.
fn draw_marker(fb: &mut [u8], x: i32, y: i32, clan: u8, kind: u8) {
    let c = CLAN_COLORS[clan as usize & 3];
    for dy in 0..TILE as i32 {
        for dx in 0..TILE as i32 {
            let edge = dx == 0 || dy == 0 || dx == TILE as i32 - 1 || dy == TILE as i32 - 1;
            put(fb, x + dx, y + dy, if edge { c } else { [c[0] / 3, c[1] / 3, c[2] / 3, 255] });
        }
    }
    for (n, digit) in format!("{kind}").bytes().enumerate() {
        for (row, bits) in DIGITS[(digit - b'0') as usize].iter().enumerate() {
            for col in 0..3 {
                if bits >> (2 - col) & 1 != 0 {
                    for (sx, sy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        put(fb, x + 4 + (n as i32 * 4 + col) * 2 + sx, y + 4 + row as i32 * 2 + sy + 6, [255, 255, 255, 255]);
                    }
                }
            }
        }
    }
}

fn render(fb: &mut [u8], map: &eta::Map, tiles: &mut Sheet, bldg: &mut Sheet, cam_x: i32, cam_y: i32) {
    fb.fill(0);
    let (tx0, ty0) = (cam_x as usize / TILE, cam_y as usize / TILE);
    for ty in ty0..(ty0 + SCREEN_H / TILE + 2).min(map.height) {
        for tx in tx0..(tx0 + SCREEN_W / TILE + 2).min(map.width) {
            let Some(img) = tiles.get(map.tile(tx, ty) as usize) else { continue };
            blit(fb, img, (tx * TILE) as i32 - cam_x, (ty * TILE) as i32 - cam_y);
        }
    }
    for b in &map.buildings {
        let (fw, fh) = FOOTPRINT.get(b.kind as usize).copied().unwrap_or((3, 3));
        // Sprites sit on the bottom-left of the footprint and may be taller than it.
        if let Some(img) = building_sprite(b.kind).and_then(|i| bldg.get(i)) {
            blit(fb, img, b.x as i32 * TILE as i32 - cam_x, (b.y as i32 + fh) * TILE as i32 - img.h as i32 - cam_y);
            continue;
        }
        let _ = fw;
        let (x, y) = (b.x as i32 * TILE as i32 - cam_x, b.y as i32 * TILE as i32 - cam_y);
        let c = CLAN_COLORS[b.clan as usize & 3];
        for dy in 0..(3 * TILE) as i32 {
            for dx in 0..(3 * TILE) as i32 {
                let edge = dx < 2 || dy < 2 || dx >= (3 * TILE) as i32 - 2 || dy >= (3 * TILE) as i32 - 2;
                if edge {
                    put(fb, x + dx, y + dy, c);
                }
            }
        }
        draw_marker(fb, x + TILE as i32, y + TILE as i32, b.clan, b.kind as u8);
    }
    for u in &map.units {
        draw_marker(fb, u.x as i32 * TILE as i32 - cam_x, u.y as i32 * TILE as i32 - cam_y, u.clan, u.kind);
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = env::args().skip(1);
    let (Some(game), Some(map_name)) = (args.next(), args.next()) else {
        eprintln!("usage: rl-game <INSTALHD dir> <path to MONDExx.ETA> [--shot out.png [cell_x cell_y]]");
        std::process::exit(2);
    };
    let game = Path::new(&game);
    let palette = lzs::parse(&fs::read(game.join("CHARGE.LZS"))?)?.palette;
    let map = eta::Map::parse(&fs::read(&map_name)?)?;
    let mut tiles = Sheet::load(game, "BLOCS.CMP", &palette)?;
    let mut bldg = Sheet::load(game, "BUILDING.CMP", &palette)?;

    let (max_x, max_y) = (((map.width * TILE).saturating_sub(SCREEN_W)) as i32, ((map.height * TILE).saturating_sub(SCREEN_H)) as i32);
    let (mut cam_x, mut cam_y) = ((map.starts[0].0 as usize * TILE) as i32, (map.starts[0].1 as usize * TILE) as i32);
    // `--shot out.png [cell_x cell_y]` renders one frame without opening a window.
    if args.next().as_deref() == Some("--shot") {
        let out = args.next().ok_or("--shot needs a file")?;
        if let (Some(x), Some(y)) = (args.next(), args.next()) {
            cam_x = x.parse::<i32>()? * TILE as i32;
            cam_y = y.parse::<i32>()? * TILE as i32;
        }
        let mut fb = vec![0u8; SCREEN_W * SCREEN_H * 4];
        render(&mut fb, &map, &mut tiles, &mut bldg, cam_x.clamp(0, max_x), cam_y.clamp(0, max_y));
        let mut enc = png::Encoder::new(std::io::BufWriter::new(fs::File::create(out)?), SCREEN_W as u32, SCREEN_H as u32);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header()?.write_image_data(&fb)?;
        return Ok(());
    }

    let sdl = sdl2::init()?;
    let window = sdl.video()?.window("Rising Lands", SCREEN_W as u32, SCREEN_H as u32).resizable().position_centered().build()?;
    let mut canvas = window.into_canvas().present_vsync().build()?;
    canvas.set_logical_size(SCREEN_W as u32, SCREEN_H as u32)?;
    let creator = canvas.texture_creator();
    let mut tex = creator.create_texture_streaming(PixelFormatEnum::ABGR8888, SCREEN_W as u32, SCREEN_H as u32)?;
    let mut events = sdl.event_pump()?;

    cam_x = cam_x.clamp(0, max_x);
    cam_y = cam_y.clamp(0, max_y);
    let mut fb = vec![0u8; SCREEN_W * SCREEN_H * 4];

    let step = Duration::from_secs_f64(1.0 / TICK_HZ as f64);
    let mut last = Instant::now();
    let mut acc = Duration::ZERO;
    'run: loop {
        for e in events.poll_iter() {
            if let Event::Quit { .. } = e {
                break 'run;
            }
        }
        let now = Instant::now();
        // Clamp so a long stall (window drag, breakpoint) doesn't cause a burst of ticks.
        acc += (now - last).min(Duration::from_millis(250));
        last = now;
        while acc >= step {
            acc -= step;
            let k = events.keyboard_state();
            let m = events.mouse_state();
            let (mx, my) = (m.x(), m.y());
            let edge = |on: bool| on as i32;
            let dx = edge(k.is_scancode_pressed(Scancode::Right)) - edge(k.is_scancode_pressed(Scancode::Left));
            let dy = edge(k.is_scancode_pressed(Scancode::Down)) - edge(k.is_scancode_pressed(Scancode::Up));
            let _ = (mx, my);
            cam_x = (cam_x + dx * SCROLL).clamp(0, max_x);
            cam_y = (cam_y + dy * SCROLL).clamp(0, max_y);
        }

        render(&mut fb, &map, &mut tiles, &mut bldg, cam_x, cam_y);
        tex.update(None, &fb, SCREEN_W * 4)?;
        canvas.clear();
        canvas.copy(&tex, None, None)?;
        canvas.present();
        // vsync normally paces us; this guards drivers that ignore it.
        std::thread::sleep(Duration::from_millis(1));
    }
    Ok(())
}
