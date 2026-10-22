//! SDL2 window: presents the guest's primary surface and feeds input to the guest.

use crate::{com, machine::*, overlay};
use std::time::Instant;
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

/// Requests from the host side that need the user (the game itself has no UI for them).
#[derive(Default)]
pub struct Ui {
    /// Set when a join screen found no sessions: ask for the host's address.
    pub want_host_address: bool,
    /// A short message shown for a few seconds.
    pub toast: Option<(String, Instant)>,
    /// Address typed by the user, waiting to be handed to the network layer.
    pub submitted_host: Option<String>,
    pub last_host: String,
}

impl Ui {
    pub fn toast(&mut self, text: impl Into<String>) {
        self.toast = Some((text.into(), Instant::now()));
    }
}

pub struct Frontend {
    _sdl: Sdl,
    canvas: Canvas<Window>,
    // Leaked so the texture can live as long as the program, which keeps the types simple.
    tex: Texture<'static>,
    tex_size: (u32, u32),
    _creator: &'static TextureCreator<WindowContext>,
    events: EventPump,
    shown: u64,
    sent_activate: bool,
    movie_tex: Option<Texture<'static>>,
    /// Text being typed in the host-address prompt, when it is open.
    prompt: Option<String>,
    last_frame: Option<(u32, u32, Vec<u8>)>,
    was_overlay: bool,
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
    pub fn new(scale: u32, mixer: crate::audio::Shared) -> Result<Frontend, String> {
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
        // The game draws its own cursor; keep the pointer inside the window so it can't wander
        // off and come back at an unexpected place.
        sdl.mouse().show_cursor(false);
        if std::env::var_os("RL_NOGRAB").is_none() {
            canvas.window_mut().set_grab(true);
        }
        video.text_input().start();
        let audio_dev = crate::audio::open(&sdl, mixer);
        if audio_dev.is_none() {
            eprintln!("warning: no audio device");
        }
        // The device stops when dropped; keep it for the life of the program.
        std::mem::forget(audio_dev);
        let events = sdl.event_pump()?;
        Ok(Frontend { _sdl: sdl, canvas, tex, tex_size: (800, 600), _creator: creator, events, shown: 0, sent_activate: false, movie_tex: None, prompt: None, last_frame: None, was_overlay: false })
    }

    /// Handles window events; returns false when the user asked to quit.
    pub fn pump(&mut self, uc: &mut Uc) -> bool {
        if !self.sent_activate && !uc.get_data().windows.is_empty() {
            self.sent_activate = true;
            activate(uc);
        }
        if uc.get_data().ui.want_host_address && self.prompt.is_none() {
            uc.get_data_mut().ui.want_host_address = false;
            self.prompt = Some(uc.get_data().ui.last_host.clone());
        }
        let events: Vec<Event> = self.events.poll_iter().collect();
        for e in events {
            // While the prompt is open it gets all the keyboard input.
            if let Some(text) = self.prompt.as_mut() {
                match &e {
                    Event::TextInput { text: t, .. } => text.extend(t.chars().filter(|c| c.is_ascii_alphanumeric() || ".:-_[]".contains(*c))),
                    Event::KeyDown { keycode: Some(Keycode::Backspace), .. } => {
                        text.pop();
                    }
                    Event::KeyDown { keycode: Some(Keycode::Return | Keycode::KpEnter), .. } => {
                        let t = text.trim().to_string();
                        self.prompt = None;
                        if !t.is_empty() {
                            uc.get_data_mut().ui.last_host = t.clone();
                            uc.get_data_mut().ui.submitted_host = Some(t);
                        }
                        continue;
                    }
                    Event::KeyDown { keycode: Some(Keycode::Escape), .. } => {
                        self.prompt = None;
                        continue;
                    }
                    _ => {}
                }
                if !matches!(e, Event::Quit { .. }) {
                    continue;
                }
            }
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
                Event::KeyDown { keycode: Some(Keycode::Return), keymod, repeat: false, .. } if keymod.intersects(sdl2::keyboard::Mod::LALTMOD | sdl2::keyboard::Mod::RALTMOD) => {
                    let w = self.canvas.window_mut();
                    let full = w.fullscreen_state() != sdl2::video::FullscreenType::Off;
                    let _ = w.set_fullscreen(if full { sdl2::video::FullscreenType::Off } else { sdl2::video::FullscreenType::Desktop });
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
        // A movie takes over the whole window while it plays.
        if let Some(v) = uc.get_data_mut().video.as_mut() {
            if v.dirty {
                v.dirty = false;
                let (w, h) = (v.avi.width as u32, v.avi.height as u32);
                let tex = self.movie_tex.get_or_insert_with(|| self._creator.create_texture_streaming(PixelFormatEnum::RGB24, w, h).expect("movie texture"));
                let _ = tex.update(None, &v.dec.rgb, w as usize * 3);
                self.canvas.clear();
                let _ = self.canvas.copy(tex, None, None);
                self.canvas.present();
            }
            return;
        }
        let n = uc.get_data().com.frames_presented;
        let toast = uc.get_data().ui.toast.as_ref().filter(|(_, t)| t.elapsed().as_secs() < 8).map(|(s, _)| s.clone());
        let overlay_on = self.prompt.is_some() || toast.is_some();
        if n == self.shown && !overlay_on && !self.was_overlay {
            return;
        }
        self.shown = n;
        self.was_overlay = overlay_on;
        if n != 0 {
            if let Some(f) = com::snapshot(uc) {
                self.last_frame = Some(f);
            }
        }
        let Some((w, h, mut rgba)) = self.last_frame.clone() else { return };
        // F11 switches the game between 800x600 and 640x480.
        if (w, h) != self.tex_size {
            if let Ok(t) = self._creator.create_texture_streaming(PixelFormatEnum::ABGR8888, w, h) {
                self.tex = t;
                self.tex_size = (w, h);
                let _ = self.canvas.set_logical_size(w, h);
            }
        }
        let stride = w as usize;
        if let Some(text) = &self.prompt {
            let label = "ENDERECO DO ANFITRIAO (IP OU IP:PORTA) - ENTER CONFIRMA, ESC CANCELA";
            let line = format!("{text}_");
            let bw = (overlay::text_width(label, 2) + 24).min(stride - 8);
            let (bx, by) = ((stride - bw) / 2, h as usize / 2 - 30);
            overlay::fill_rect(&mut rgba, stride, bx, by, bw, 70, [20, 12, 8, 255]);
            overlay::fill_rect(&mut rgba, stride, bx, by, bw, 2, [220, 170, 90, 255]);
            overlay::fill_rect(&mut rgba, stride, bx, by + 68, bw, 2, [220, 170, 90, 255]);
            overlay::draw_text(&mut rgba, stride, bx + 12, by + 12, label, 2, [230, 200, 140, 255]);
            overlay::draw_text(&mut rgba, stride, bx + 12, by + 40, &line, 3, [255, 255, 255, 255]);
        }
        if let Some(t) = toast {
            let bw = overlay::text_width(&t, 2) + 20;
            overlay::fill_rect(&mut rgba, stride, 8, 8, bw.min(stride - 16), 26, [20, 12, 8, 220]);
            overlay::draw_text(&mut rgba, stride, 18, 14, &t, 2, [230, 200, 140, 255]);
        }
        let _ = self.tex.update(None, &rgba, w as usize * 4);
        self.canvas.clear();
        let _ = self.canvas.copy(&self.tex, None, None);
        self.canvas.present();
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
