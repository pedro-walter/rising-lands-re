//! The options menu (F10): one volume slider each for the music, the building noises (the
//! builder's hammer), the sound effects and the unit voices. Drawn over the game with the
//! bitmap font in `overlay`; the volumes themselves live in `audio::Volumes`.

use crate::{audio::Volumes, overlay};

/// File in the user directory holding the volumes.
pub const FILE: &str = "options.txt";
pub const LABELS: [&str; 4] = ["MUSICA", "CONSTRUCAO (MARTELO)", "EFEITOS SONOROS", "VOZES"];
const TITLE: &str = "OPCOES DE SOM";
const HELP: &str = "CIMA/BAIXO ESCOLHEM - ESQ/DIR AJUSTAM - ESC FECHA";
const STEP: i32 = 5;

const PANEL_W: usize = 560;
const ROW_H: usize = 48;
const BAR_W: usize = 200;
const BAR_H: usize = 14;
const SCALE: usize = 2;

/// Menu state: the selected row and the row being dragged with the mouse.
#[derive(Default)]
pub struct Options {
    pub sel: usize,
    pub drag: Option<usize>,
}

struct Layout {
    x: usize,
    y: usize,
    h: usize,
    bar_x: usize,
}

fn layout(w: usize, h: usize) -> Layout {
    let ph = 70 + ROW_H * LABELS.len() + 34;
    Layout { x: w.saturating_sub(PANEL_W) / 2, y: h.saturating_sub(ph) / 2, h: ph, bar_x: w.saturating_sub(PANEL_W) / 2 + 270 }
}

fn bar_y(l: &Layout, row: usize) -> usize {
    l.y + 70 + row * ROW_H + 6
}

/// Where a row's bar is in a `w` x `h` frame (for tests).
#[cfg(test)]
pub fn bar_x(w: usize) -> usize {
    layout(w, 600).bar_x
}

#[cfg(test)]
pub fn row_y(w: usize, h: usize, row: usize) -> usize {
    bar_y(&layout(w, h), row)
}

impl Options {
    /// Moves the selection by `d` rows, wrapping.
    pub fn select(&mut self, d: i32) {
        self.sel = (self.sel as i32 + d).rem_euclid(LABELS.len() as i32) as usize;
    }

    /// Changes the selected volume by `d` steps of 5 percent.
    pub fn adjust(&self, v: &mut Volumes, d: i32) {
        let slot = v.slot(self.sel);
        *slot = (*slot as i32 + d * STEP).clamp(0, 100) as u8;
    }

    /// Sets the slider under a mouse press or drag at (x, y) of a `w` x `h` frame. A press starts the
    /// drag on the row it lands on; while dragging, the row stays fixed whatever the height.
    pub fn pointer(&mut self, v: &mut Volumes, w: usize, h: usize, x: i32, y: i32, press: bool) -> bool {
        let l = layout(w, h);
        let row = if press {
            (0..LABELS.len()).find(|&r| {
                let by = bar_y(&l, r) as i32;
                y >= by - 12 && y < by + BAR_H as i32 + 12 && x >= l.bar_x as i32 - 10 && x < (l.bar_x + BAR_W + 60) as i32
            })
        } else {
            self.drag
        };
        let Some(row) = row else { return false };
        self.sel = row;
        self.drag = Some(row);
        let pct = ((x - l.bar_x as i32) * 100 / BAR_W as i32).clamp(0, 100);
        // Snap to the same 5 percent grid as the keys, with the ends exact.
        *v.slot(row) = ((pct + STEP / 2) / STEP * STEP).clamp(0, 100) as u8;
        true
    }

    pub fn draw(&self, rgba: &mut [u8], w: usize, h: usize, v: &mut Volumes) {
        let l = layout(w, h);
        let (bg, gold, text, dim) = ([20, 12, 8, 255], [220, 170, 90, 255], [230, 200, 140, 255], [120, 90, 50, 255]);
        overlay::fill_rect(rgba, w, l.x, l.y, PANEL_W.min(w), l.h, bg);
        overlay::fill_rect(rgba, w, l.x, l.y, PANEL_W.min(w), 2, gold);
        overlay::fill_rect(rgba, w, l.x, l.y + l.h - 2, PANEL_W.min(w), 2, gold);
        overlay::draw_text(rgba, w, l.x + (PANEL_W.saturating_sub(overlay::text_width(TITLE, 3))) / 2, l.y + 18, TITLE, 3, [255, 255, 255, 255]);
        for (i, label) in LABELS.iter().enumerate() {
            let by = bar_y(&l, i);
            let color = if i == self.sel { [255, 255, 255, 255] } else { text };
            overlay::draw_text(rgba, w, l.x + 20, by, label, SCALE, color);
            overlay::fill_rect(rgba, w, l.bar_x, by, BAR_W, BAR_H, dim);
            let pct = *v.slot(i) as usize;
            overlay::fill_rect(rgba, w, l.bar_x, by, BAR_W * pct / 100, BAR_H, if i == self.sel { gold } else { text });
            overlay::draw_text(rgba, w, l.bar_x + BAR_W + 14, by, &pct.to_string(), SCALE, color);
        }
        let hy = l.y + l.h - 26;
        overlay::draw_text(rgba, w, l.x + (PANEL_W.saturating_sub(overlay::text_width(HELP, 1))) / 2, hy, HELP, 1, text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_wraps_and_volumes_step_within_bounds() {
        let mut o = Options::default();
        o.select(-1);
        assert_eq!(o.sel, 3);
        o.select(1);
        assert_eq!(o.sel, 0);
        let mut v = Volumes::default();
        o.adjust(&mut v, 1);
        assert_eq!(v.music, 100);
        for _ in 0..30 {
            o.adjust(&mut v, -1);
        }
        assert_eq!(v.music, 0);
        o.sel = 1;
        o.adjust(&mut v, -1);
        assert_eq!(v.building, 95);
        assert_eq!(v.effects, 100);
    }

    #[test]
    fn mouse_sets_the_slider_it_lands_on_and_drags_stay_on_their_row() {
        let (w, h) = (800, 600);
        let l = layout(w, h);
        let mut o = Options::default();
        let mut v = Volumes::default();
        let y1 = bar_y(&l, 1) as i32 + 5;
        assert!(o.pointer(&mut v, w, h, (l.bar_x + BAR_W / 2) as i32, y1, true));
        assert_eq!((o.sel, v.building), (1, 50));
        // Dragging far below the bar still moves row 1, and past the ends clamps.
        assert!(o.pointer(&mut v, w, h, l.bar_x as i32 - 80, y1 + 200, false));
        assert_eq!(v.building, 0);
        assert!(o.pointer(&mut v, w, h, (l.bar_x + BAR_W + 40) as i32, y1, false));
        assert_eq!(v.building, 100);
        o.drag = None;
        assert!(!o.pointer(&mut v, w, h, 5, 5, true), "outside every bar");
        assert!(!o.pointer(&mut v, w, h, 5, 5, false), "no drag in progress");
        assert_eq!((v.music, v.effects, v.speech), (100, 100, 100));
    }

    #[test]
    fn draws_a_panel_inside_the_frame_with_the_bars_filled_to_the_volume() {
        let (w, h) = (800usize, 600usize);
        let mut buf = vec![0u8; w * h * 4];
        let mut v = Volumes { music: 100, building: 0, effects: 50, speech: 100 };
        Options { sel: 2, drag: None }.draw(&mut buf, w, h, &mut v);
        let l = layout(w, h);
        let px = |x: usize, y: usize| -> [u8; 4] { buf[(y * w + x) * 4..][..4].try_into().unwrap() };
        assert_eq!(px(l.x, l.y), [220, 170, 90, 255], "gold top edge");
        assert_eq!(px(0, 0), [0; 4], "nothing outside the panel");
        let y = bar_y(&l, 2) + 2;
        assert_eq!(px(l.bar_x + 10, y), [220, 170, 90, 255], "selected bar filled");
        assert_eq!(px(l.bar_x + BAR_W - 10, y), [120, 90, 50, 255], "half-filled bar is empty at the right");
        assert_eq!(px(l.bar_x + 10, bar_y(&l, 1) + 2), [120, 90, 50, 255], "muted bar is empty");
        // A frame smaller than the panel must not panic.
        let mut small = vec![0u8; 320 * 200 * 4];
        Options::default().draw(&mut small, 320, 200, &mut v);
    }
}
