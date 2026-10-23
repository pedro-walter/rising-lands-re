//! Tiny on-screen text for host-side messages (typing a host address, network status).
//! A 5x7 bitmap font drawn straight into the RGBA frame; the game itself has no such UI.

const GLYPHS: &[(char, [u8; 7])] = &[
    (' ', [0, 0, 0, 0, 0, 0, 0]),
    ('.', [0, 0, 0, 0, 0, 0b01100, 0b01100]),
    (':', [0, 0b01100, 0b01100, 0, 0b01100, 0b01100, 0]),
    ('-', [0, 0, 0, 0b11111, 0, 0, 0]),
    ('_', [0, 0, 0, 0, 0, 0, 0b11111]),
    ('/', [0b00001, 0b00010, 0b00010, 0b00100, 0b01000, 0b01000, 0b10000]),
    ('(', [0b00010, 0b00100, 0b01000, 0b01000, 0b01000, 0b00100, 0b00010]),
    (')', [0b01000, 0b00100, 0b00010, 0b00010, 0b00010, 0b00100, 0b01000]),
    ('0', [0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110]),
    ('1', [0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110]),
    ('2', [0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111]),
    ('3', [0b11110, 0b00001, 0b00001, 0b01110, 0b00001, 0b00001, 0b11110]),
    ('4', [0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010]),
    ('5', [0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110]),
    ('6', [0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110]),
    ('7', [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000]),
    ('8', [0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110]),
    ('9', [0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100]),
    ('A', [0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001]),
    ('B', [0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110]),
    ('C', [0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110]),
    ('D', [0b11110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11110]),
    ('E', [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111]),
    ('F', [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000]),
    ('G', [0b01110, 0b10001, 0b10000, 0b10111, 0b10001, 0b10001, 0b01111]),
    ('H', [0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001]),
    ('I', [0b01110, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110]),
    ('J', [0b00111, 0b00010, 0b00010, 0b00010, 0b00010, 0b10010, 0b01100]),
    ('K', [0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001]),
    ('L', [0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111]),
    ('M', [0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001]),
    ('N', [0b10001, 0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001]),
    ('O', [0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110]),
    ('P', [0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000]),
    ('Q', [0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101]),
    ('R', [0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001]),
    ('S', [0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110]),
    ('T', [0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100]),
    ('U', [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110]),
    ('V', [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100]),
    ('W', [0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b10101, 0b01010]),
    ('X', [0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001]),
    ('Y', [0b10001, 0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100]),
    ('Z', [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111]),
];

fn glyph(c: char) -> &'static [u8; 7] {
    let c = c.to_ascii_uppercase();
    GLYPHS.iter().find(|(g, _)| *g == c).map_or(&GLYPHS[0].1, |(_, rows)| rows)
}

/// Width in pixels of `text` at `scale`.
pub fn text_width(text: &str, scale: usize) -> usize {
    text.chars().count() * 6 * scale
}

pub fn fill_rect(rgba: &mut [u8], stride_px: usize, x: usize, y: usize, w: usize, h: usize, color: [u8; 4]) {
    for yy in y..y + h {
        for xx in x..x + w {
            let o = (yy * stride_px + xx) * 4;
            if let Some(px) = rgba.get_mut(o..o + 4) {
                px.copy_from_slice(&color);
            }
        }
    }
}

pub fn draw_text(rgba: &mut [u8], stride_px: usize, x: usize, y: usize, text: &str, scale: usize, color: [u8; 4]) {
    for (i, c) in text.chars().enumerate() {
        for (row, bits) in glyph(c).iter().enumerate() {
            for col in 0..5 {
                if bits >> (4 - col) & 1 != 0 {
                    fill_rect(rgba, stride_px, x + (i * 6 + col) * scale, y + row * scale, scale, scale, color);
                }
            }
        }
    }
}

/// The game's own pointer (14x14, 0 = transparent), captured from its rendering, tip at the top-left.
const CURSOR: [u32; 14 * 14] = [
    0xff101010, 0xff101010, 0xff101010, 0xff101010, 0xff101010, 0xff101010, 0xff101010, 0xff101010, 0xff101010, 0xff101010, 0xff101010, 0xff101010, 0, 0,
    0xff101010, 0xfffcfcfc, 0xffe0e0e0, 0xffcccccc, 0xffcccccc, 0xffcccccc, 0xffcccccc, 0xffcccccc, 0xffcccccc, 0xffcccccc, 0xffcccccc, 0xff101010, 0, 0,
    0xff101010, 0xfffcfcfc, 0xffcccccc, 0xffcccccc, 0xffcccccc, 0xffcccccc, 0xffcccccc, 0xffcccccc, 0xffcccccc, 0xff989898, 0xff101010, 0xff101010, 0xff101010, 0xff101010,
    0xff101010, 0xfffcfcfc, 0xffcccccc, 0xffcccccc, 0xffcccccc, 0xffcccccc, 0xffcccccc, 0xffcccccc, 0xff989898, 0xff101010, 0xff101010, 0xff101010, 0xff101010, 0xff101010,
    0xff101010, 0xfffcfcfc, 0xffcccccc, 0xffcccccc, 0xffcccccc, 0xffcccccc, 0xffcccccc, 0xff989898, 0xff101010, 0xff101010, 0xff101010, 0xff101010, 0xff101010, 0,
    0xff101010, 0xfffcfcfc, 0xffcccccc, 0xffcccccc, 0xffcccccc, 0xffcccccc, 0xffb0b0b0, 0xff989898, 0xff545454, 0xff101010, 0xff101010, 0xff101010, 0, 0,
    0xff101010, 0xfffcfcfc, 0xffcccccc, 0xffcccccc, 0xffcccccc, 0xffcccccc, 0xffcccccc, 0xffb0b0b0, 0xff989898, 0xff686868, 0xff101010, 0, 0, 0,
    0xff101010, 0xfffcfcfc, 0xffcccccc, 0xffcccccc, 0xffe0e0e0, 0xffcccccc, 0xffcccccc, 0xffcccccc, 0xffb0b0b0, 0xff989898, 0xff686868, 0xff101010, 0, 0,
    0xff101010, 0xfffcfcfc, 0xffcccccc, 0xffcccccc, 0xff101010, 0xfffcfcfc, 0xffcccccc, 0xffcccccc, 0xffcccccc, 0xffb0b0b0, 0xff989898, 0xff101010, 0xff101010, 0,
    0xff101010, 0xfffcfcfc, 0xffcccccc, 0xff101010, 0xff101010, 0xff101010, 0xfffcfcfc, 0xffcccccc, 0xffcccccc, 0xff989898, 0xff101010, 0xff101010, 0xff101010, 0xff101010,
    0xff101010, 0xffe0e0e0, 0xff101010, 0xff101010, 0xff101010, 0xff101010, 0xff101010, 0xfffcfcfc, 0xffb0b0b0, 0xff101010, 0xff101010, 0xff101010, 0xff101010, 0xff101010,
    0xff101010, 0xff101010, 0xff101010, 0xff101010, 0xff101010, 0xff101010, 0, 0xff101010, 0xff101010, 0xff101010, 0xff101010, 0xff101010, 0xff101010, 0,
    0, 0, 0xff101010, 0xff101010, 0xff101010, 0, 0, 0, 0xff101010, 0xff101010, 0xff101010, 0xff101010, 0, 0,
    0, 0, 0xff101010, 0xff101010, 0, 0, 0, 0, 0, 0xff101010, 0xff101010, 0, 0, 0,
];

/// Draws the pointer with its tip at (x, y).
pub fn draw_arrow(rgba: &mut [u8], w: usize, h: usize, x: i32, y: i32) {
    for (k, &c) in CURSOR.iter().enumerate() {
        let (px, py) = (x + (k % 14) as i32, y + (k / 14) as i32);
        if c == 0 || px < 0 || py < 0 || px as usize >= w || py as usize >= h {
            continue;
        }
        let o = (py as usize * w + px as usize) * 4;
        rgba[o..o + 4].copy_from_slice(&[(c >> 16) as u8, (c >> 8) as u8, c as u8, 255]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: [u8; 4] = [255, 0, 0, 255];

    fn px(buf: &[u8], stride: usize, x: usize, y: usize) -> [u8; 4] {
        let o = (y * stride + x) * 4;
        buf[o..o + 4].try_into().unwrap()
    }

    #[test]
    fn every_glyph_is_unique_and_unknown_characters_are_blank() {
        for (i, (c, rows)) in GLYPHS.iter().enumerate() {
            assert!(rows.iter().all(|r| r >> 5 == 0), "{c:?} uses more than 5 columns");
            for (d, other) in &GLYPHS[i + 1..] {
                assert_ne!(c, d);
                assert_ne!(rows, other, "{c:?} and {d:?} look the same");
            }
        }
        assert_eq!(glyph('a'), glyph('A'));
        assert_eq!(glyph('~'), &[0; 7]);
    }

    #[test]
    fn text_width_counts_six_pixels_per_character() {
        assert_eq!(text_width("", 3), 0);
        assert_eq!(text_width("ABC", 1), 18);
        assert_eq!(text_width("ab", 2), 24);
    }

    #[test]
    fn fill_rect_clips_at_the_buffer_end() {
        let mut buf = vec![0u8; 4 * 4 * 4];
        fill_rect(&mut buf, 4, 2, 2, 10, 10, RED);
        assert_eq!(px(&buf, 4, 3, 3), RED);
        assert_eq!(px(&buf, 4, 1, 1), [0; 4]);
        assert_eq!(buf.len(), 64);
    }

    #[test]
    fn draw_text_lights_the_expected_pixels() {
        let mut buf = vec![0u8; 10 * 8 * 4];
        draw_text(&mut buf, 10, 1, 0, "I", 1, RED);
        // 'I' has a full top bar (row 0 = 0b01110) and a centre stem (row 3 = 0b00100).
        assert_eq!(px(&buf, 10, 2, 0), RED);
        assert_eq!(px(&buf, 10, 4, 0), RED);
        assert_eq!(px(&buf, 10, 1, 0), [0; 4]);
        assert_eq!(px(&buf, 10, 3, 3), RED);
        let mut big = vec![0u8; 20 * 20 * 4];
        draw_text(&mut big, 20, 0, 0, "I", 2, RED);
        assert_eq!(px(&big, 20, 3, 1), RED, "scale 2 doubles each dot");
    }

    #[test]
    fn arrow_is_clipped_to_the_frame() {
        let mut buf = vec![0u8; 20 * 20 * 4];
        draw_arrow(&mut buf, 20, 20, 2, 3);
        assert_eq!(px(&buf, 20, 2, 3), [0x10, 0x10, 0x10, 255], "tip pixel");
        assert_eq!(px(&buf, 20, 3, 4), [0xfc, 0xfc, 0xfc, 255]);
        assert_eq!(px(&buf, 20, 15, 3), [0; 4], "transparent corner untouched");
        // Entirely or partly off-screen positions must not panic.
        draw_arrow(&mut buf, 20, 20, -7, -7);
        draw_arrow(&mut buf, 20, 20, 15, 15);
        draw_arrow(&mut buf, 20, 20, 100, 100);
    }
}
