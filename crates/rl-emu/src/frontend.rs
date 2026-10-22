//! SDL2 window: presents the guest's primary surface and feeds input to the guest.

use crate::{com, machine::*};
use sdl2::{
    event::{Event, WindowEvent},
    keyboard::Keycode,
    mouse::MouseButton,
    pixels::PixelFormatEnum,
    render::{Canvas, Texture, TextureCreator},
    video::{Window, WindowContext},
    EventPump, Sdl,
};

const HWND_MAIN: u32 = 0x1000;

const WM_ACTIVATE: u32 = 0x0006;
const WM_SETFOCUS: u32 = 0x0007;
const WM_KEYDOWN: u32 = 0x0100;
const WM_KEYUP: u32 = 0x0101;
const WM_CHAR: u32 = 0x0102;
const WM_MOUSEMOVE: u32 = 0x0200;
const WM_LBUTTONDOWN: u32 = 0x0201;
const WM_LBUTTONUP: u32 = 0x0202;
const WM_RBUTTONDOWN: u32 = 0x0204;
const WM_RBUTTONUP: u32 = 0x0205;
const WM_MBUTTONDOWN: u32 = 0x0207;
const WM_MBUTTONUP: u32 = 0x0208;
const WM_ACTIVATEAPP: u32 = 0x001c;

pub struct Frontend {
    _sdl: Sdl,
    canvas: Canvas<Window>,
    // Leaked so the texture can live as long as the program, which keeps the types simple.
    tex: Texture<'static>,
    _creator: &'static TextureCreator<WindowContext>,
    events: EventPump,
    shown: u64,
    sent_activate: bool,
}

fn vk_of(k: Keycode) -> Option<u8> {
    let c = k.into_i32();
    Some(match k {
        Keycode::Left => 0x25,
        Keycode::Up => 0x26,
        Keycode::Right => 0x27,
        Keycode::Down => 0x28,
        Keycode::Return | Keycode::KpEnter => 0x0d,
        Keycode::Escape => 0x1b,
        Keycode::Space => 0x20,
        Keycode::Tab => 0x09,
        Keycode::Backspace => 0x08,
        Keycode::LShift | Keycode::RShift => 0x10,
        Keycode::LCtrl | Keycode::RCtrl => 0x11,
        Keycode::LAlt | Keycode::RAlt => 0x12,
        Keycode::Delete => 0x2e,
        Keycode::Insert => 0x2d,
        Keycode::Home => 0x24,
        Keycode::End => 0x23,
        Keycode::PageUp => 0x21,
        Keycode::PageDown => 0x22,
        Keycode::Pause => 0x13,
        Keycode::CapsLock => 0x14,
        _ if (b'a' as i32..=b'z' as i32).contains(&c) => (c - 32) as u8,
        _ if (b'0' as i32..=b'9' as i32).contains(&c) => c as u8,
        _ if (Keycode::F1.into_i32()..=Keycode::F12.into_i32()).contains(&c) => 0x70 + (c - Keycode::F1.into_i32()) as u8,
        _ if (Keycode::Kp0.into_i32()..=Keycode::Kp9.into_i32()).contains(&c) => 0x60 + (c - Keycode::Kp0.into_i32()) as u8,
        _ => return None,
    })
}

pub fn post(uc: &mut Uc, message: u32, wparam: u32, lparam: u32) {
    uc.get_data_mut().msgs.push_back(Msg { hwnd: HWND_MAIN, message, wparam, lparam });
}

impl Frontend {
    pub fn new(scale: u32) -> Result<Frontend, String> {
        let sdl = sdl2::init()?;
        let video = sdl.video()?;
        let window = video
            .window("Rising Lands", 800 * scale, 600 * scale)
            .position_centered()
            .resizable()
            .build()
            .map_err(|e| e.to_string())?;
        let mut canvas = window.into_canvas().present_vsync().build().map_err(|e| e.to_string())?;
        canvas.set_logical_size(800, 600).map_err(|e| e.to_string())?;
        let creator: &'static TextureCreator<WindowContext> = Box::leak(Box::new(canvas.texture_creator()));
        let tex = creator.create_texture_streaming(PixelFormatEnum::ABGR8888, 800, 600).map_err(|e| e.to_string())?;
        video.text_input().start();
        let events = sdl.event_pump()?;
        Ok(Frontend { _sdl: sdl, canvas, tex, _creator: creator, events, shown: 0, sent_activate: false })
    }

    /// Handles window events; returns false when the user asked to quit.
    pub fn pump(&mut self, uc: &mut Uc) -> bool {
        if !self.sent_activate && !uc.get_data().windows.is_empty() {
            self.sent_activate = true;
            activate(uc);
        }
        let events: Vec<Event> = self.events.poll_iter().collect();
        for e in events {
            match e {
                Event::Quit { .. } => return false,
                Event::MouseMotion { x, y, .. } => {
                    uc.get_data_mut().cursor = (x, y);
                    post(uc, WM_MOUSEMOVE, 0, ((y as u32) << 16) | (x as u32 & 0xffff));
                }
                Event::MouseButtonDown { mouse_btn, x, y, .. } | Event::MouseButtonUp { mouse_btn, x, y, .. } => {
                    let down = matches!(e, Event::MouseButtonDown { .. });
                    let (m, vk) = match (mouse_btn, down) {
                        (MouseButton::Left, true) => (WM_LBUTTONDOWN, 1),
                        (MouseButton::Left, false) => (WM_LBUTTONUP, 1),
                        (MouseButton::Right, true) => (WM_RBUTTONDOWN, 2),
                        (MouseButton::Right, false) => (WM_RBUTTONUP, 2),
                        (MouseButton::Middle, true) => (WM_MBUTTONDOWN, 4),
                        (MouseButton::Middle, false) => (WM_MBUTTONUP, 4),
                        _ => continue,
                    };
                    uc.get_data_mut().keys[vk] = down;
                    uc.get_data_mut().cursor = (x, y);
                    post(uc, m, 0, ((y as u32) << 16) | (x as u32 & 0xffff));
                }
                Event::KeyDown { keycode: Some(k), repeat, .. } | Event::KeyUp { keycode: Some(k), repeat, .. } => {
                    let down = matches!(e, Event::KeyDown { .. });
                    if let Some(vk) = vk_of(k) {
                        uc.get_data_mut().keys[vk as usize] = down;
                        if !(repeat && !down) {
                            post(uc, if down { WM_KEYDOWN } else { WM_KEYUP }, vk as u32, if down { 1 } else { 0xc000_0001 });
                        }
                        if down && matches!(k, Keycode::Return | Keycode::Escape | Keycode::Backspace | Keycode::Tab) {
                            post(uc, WM_CHAR, vk as u32, 1);
                        }
                    }
                }
                Event::TextInput { text, .. } => {
                    for c in text.chars() {
                        post(uc, WM_CHAR, c as u32 & 0xff, 1);
                    }
                }
                Event::Window { win_event: WindowEvent::FocusGained, .. } => post(uc, WM_SETFOCUS, 0, 0),
                _ => {}
            }
        }
        true
    }

    /// Uploads and shows the primary surface if the guest presented a new frame.
    pub fn present(&mut self, uc: &mut Uc) {
        let n = uc.get_data().com.frames_presented;
        if n == self.shown {
            return;
        }
        self.shown = n;
        if let Some((w, h, rgba)) = com::snapshot(uc) {
            if (w, h) == (800, 600) {
                let _ = self.tex.update(None, &rgba, 800 * 4);
                self.canvas.clear();
                let _ = self.canvas.copy(&self.tex, None, None);
                self.canvas.present();
            }
        }
    }
}

/// Synthesises input for scripted (headless) runs.
pub fn inject_mouse(uc: &mut Uc, x: i32, y: i32, button: Option<(bool, bool)>) {
    uc.get_data_mut().cursor = (x, y);
    let lp = ((y as u32) << 16) | (x as u32 & 0xffff);
    match button {
        None => post(uc, WM_MOUSEMOVE, 0, lp),
        Some((right, down)) => {
            uc.get_data_mut().keys[if right { 2 } else { 1 }] = down;
            let m = match (right, down) {
                (false, true) => WM_LBUTTONDOWN,
                (false, false) => WM_LBUTTONUP,
                (true, true) => WM_RBUTTONDOWN,
                (true, false) => WM_RBUTTONUP,
            };
            post(uc, m, 0, lp);
        }
    }
}

pub fn inject_key(uc: &mut Uc, vk: u8, down: bool) {
    uc.get_data_mut().keys[vk as usize] = down;
    post(uc, if down { WM_KEYDOWN } else { WM_KEYUP }, vk as u32, if down { 1 } else { 0xc000_0001 });
    if down && matches!(vk, 0x0d | 0x1b | 0x08 | 0x09) {
        post(uc, WM_CHAR, vk as u32, 1);
    }
}

/// Tells the guest its window is active (done once the window exists).
pub fn activate(uc: &mut Uc) {
    post(uc, WM_ACTIVATEAPP, 1, 0);
    post(uc, WM_ACTIVATE, 1, 0);
    post(uc, WM_SETFOCUS, 0, 0);
}
