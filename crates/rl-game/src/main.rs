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

#[cfg(test)]
mod tests {
    use super::*;

    fn palette() -> rl_assets::Palette {
        let mut p = [[0u8; 3]; 256];
        for (i, e) in p.iter_mut().enumerate() {
            *e = [i as u8, 0, 255 - i as u8];
        }
        rl_assets::Palette(p)
    }

    /// `.CMP` bytes for a sprite of `w x h` that is fully opaque with palette index `v`.
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

    fn sheet(w: u16, h: u16, v: u8) -> Sheet {
        let archive = SpriteArchive::parse(&solid_cmp(w, h, v)).unwrap();
        Sheet { cache: (0..archive.sprites.len()).map(|_| None).collect(), archive, palette: palette() }
    }

    fn px(fb: &[u8], x: usize, y: usize) -> [u8; 4] {
        fb[(y * SCREEN_W + x) * 4..][..4].try_into().unwrap()
    }

    fn map(buildings: Vec<eta::Building>, units: Vec<eta::Unit>) -> eta::Map {
        eta::Map {
            version: "V0.3".into(),
            width: 64,
            height: 64,
            layer0: vec![0; 65536],
            tiles: vec![0; 65536],
            layer2: vec![0; 65536],
            layer3: vec![0; 65536],
            resources: [[0; 4]; 3],
            starts: [(0, 0); 4],
            buildings,
            units,
        }
    }

    fn building(kind: u16, clan: u8, x: u8, y: u8) -> eta::Building {
        eta::Building { slot: 0, clan, kind, x, y, raw: Vec::new() }
    }

    #[test]
    fn sheets_decode_lazily_and_reject_bad_indices() {
        let mut s = sheet(2, 2, 7);
        assert!(s.cache[0].is_none());
        let img = s.get(0).unwrap();
        assert_eq!((img.w, img.h), (2, 2));
        assert_eq!(&img.rgba[..4], &[7, 0, 248, 255]);
        assert!(s.cache[0].is_some());
        assert!(s.get(1).is_none());
    }

    #[test]
    fn sheet_loading_reads_from_the_graphics_folder() {
        let dir = std::env::temp_dir().join(format!("rl-game-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("GRAPHICS")).unwrap();
        std::fs::write(dir.join("GRAPHICS/T.CMP"), solid_cmp(1, 1, 3)).unwrap();
        assert_eq!(Sheet::load(&dir, "T.CMP", &palette()).unwrap().archive.sprites.len(), 1);
        assert!(Sheet::load(&dir, "MISSING.CMP", &palette()).is_err());
        std::fs::write(dir.join("GRAPHICS/BAD.CMP"), b"x").unwrap();
        assert!(Sheet::load(&dir, "BAD.CMP", &palette()).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn blit_clips_at_every_screen_edge_and_skips_transparent_pixels() {
        let mut fb = vec![0u8; SCREEN_W * SCREEN_H * 4];
        let img = Img { w: 2, h: 2, rgba: vec![9, 9, 9, 255, 0, 0, 0, 0, 8, 8, 8, 255, 7, 7, 7, 255] };
        blit(&mut fb, &img, 10, 20);
        assert_eq!((px(&fb, 10, 20), px(&fb, 11, 20), px(&fb, 10, 21), px(&fb, 11, 21)), ([9, 9, 9, 255], [0; 4], [8, 8, 8, 255], [7, 7, 7, 255]));
        blit(&mut fb, &img, -1, -1);
        assert_eq!(px(&fb, 0, 0), [7, 7, 7, 255], "only the bottom-right pixel is on screen");
        blit(&mut fb, &img, SCREEN_W as i32 - 1, SCREEN_H as i32 - 1);
        assert_eq!(px(&fb, SCREEN_W - 1, SCREEN_H - 1), [9, 9, 9, 255]);
        blit(&mut fb, &img, 5000, 5000);
        blit(&mut fb, &img, -5000, -5000);
    }

    #[test]
    fn put_ignores_out_of_range_pixels() {
        let mut fb = vec![0u8; SCREEN_W * SCREEN_H * 4];
        put(&mut fb, -1, 0, [1; 4]);
        put(&mut fb, 0, SCREEN_H as i32, [1; 4]);
        put(&mut fb, SCREEN_W as i32, 0, [1; 4]);
        assert!(fb.iter().all(|&b| b == 0));
        put(&mut fb, 3, 4, [1, 2, 3, 4]);
        assert_eq!(px(&fb, 3, 4), [1, 2, 3, 4]);
    }

    #[test]
    fn markers_show_the_clan_colour_and_the_kind_number() {
        let mut fb = vec![0u8; SCREEN_W * SCREEN_H * 4];
        draw_marker(&mut fb, 100, 100, 1, 12);
        assert_eq!(px(&fb, 100, 100), CLAN_COLORS[1], "border in the clan colour");
        assert_eq!(px(&fb, 131, 131), CLAN_COLORS[1]);
        assert_eq!(px(&fb, 120, 100 + 29), [85, 23, 23, 255], "dimmed fill (only white glyph pixels differ)");
        let white = (100..132).flat_map(|y| (100..132).map(move |x| (x, y))).filter(|&(x, y)| px(&fb, x, y) == [255, 255, 255, 255]).count();
        assert!(white > 20, "two digits drawn: {white}");
        // Clan numbers wrap to the four colours; kinds up to 255 draw three digits without panicking.
        draw_marker(&mut fb, 0, 0, 5, 255);
        assert_eq!(px(&fb, 0, 0), CLAN_COLORS[1]);
        draw_marker(&mut fb, -20, -20, 0, 7);
        draw_marker(&mut fb, SCREEN_W as i32 - 5, SCREEN_H as i32 - 5, 3, 0);
    }

    #[test]
    fn building_sprites_and_footprints() {
        assert_eq!((building_sprite(0), building_sprite(1), building_sprite(2), building_sprite(3)), (Some(47), Some(1), Some(3), None));
        assert_eq!(FOOTPRINT.len(), 21);
    }

    #[test]
    fn render_draws_terrain_buildings_and_unit_markers() {
        let mut tiles = sheet(32, 32, 5);
        let mut bldg = sheet(40, 40, 9);
        // Kind 1 uses sprite 1 which this one-sprite sheet lacks, so it falls back to the outlined placeholder box.
        let m = map(vec![building(1, 2, 4, 3), building(99, 0, 20, 20)], vec![eta::Unit { id: 0, clan: 3, kind: 6, x: 1, y: 1, raw: Vec::new() }]);
        let mut fb = vec![0xaau8; SCREEN_W * SCREEN_H * 4];
        render(&mut fb, &m, &mut tiles, &mut bldg, 0, 0);
        assert_eq!(px(&fb, 790, 590), [5, 0, 250, 255], "terrain fills the view");
        assert_eq!(px(&fb, 4 * TILE, 3 * TILE), CLAN_COLORS[2], "placeholder building outline");
        assert_eq!(px(&fb, TILE, TILE), CLAN_COLORS[3], "unit marker border");
        // Scrolled: the unit at cell (1,1) with the camera at (16, 16) moves up-left by 16 pixels.
        render(&mut fb, &m, &mut tiles, &mut bldg, 16, 16);
        assert_eq!(px(&fb, TILE - 16, TILE - 16), CLAN_COLORS[3]);
        // Past the playable area nothing but black is drawn.
        let mut small = map(vec![], vec![]);
        small.width = 2;
        small.height = 2;
        render(&mut fb, &small, &mut tiles, &mut bldg, 0, 0);
        assert_eq!(px(&fb, 100, 100), [0, 0, 0, 0]);
        assert_eq!(px(&fb, 10, 10), [5, 0, 250, 255]);
    }

    #[test]
    fn sprite_buildings_sit_on_the_bottom_of_their_footprint() {
        let mut tiles = sheet(32, 32, 5);
        // Index 1 is Caserne (sprite 1); build a two-sprite sheet so it exists.
        let mut two = solid_cmp(8, 8, 9);
        let extra = solid_cmp(8, 8, 9);
        two[0] = 2;
        two.extend_from_slice(&extra[2..]);
        let archive = SpriteArchive::parse(&two).unwrap();
        let mut bldg = Sheet { cache: vec![None, None], archive, palette: palette() };
        let m = map(vec![building(1, 0, 10, 10)], vec![]);
        let mut fb = vec![0u8; SCREEN_W * SCREEN_H * 4];
        render(&mut fb, &m, &mut tiles, &mut bldg, 0, 0);
        // Caserne footprint is 5x6 cells: the 8 px sprite ends at the footprint's bottom edge, left-aligned.
        let (x, y) = (10 * TILE, (10 + 6) * TILE - 8);
        assert_eq!(px(&fb, x, y), [9, 0, 246, 255]);
        assert_eq!(px(&fb, x + 8, y), [5, 0, 250, 255]);
    }
}
