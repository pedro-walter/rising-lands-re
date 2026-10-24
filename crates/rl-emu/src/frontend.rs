//! SDL2 window: presents the guest's primary surface and feeds input to the guest.

use crate::{com, machine::*, options::{Options, Prefs}, overlay};
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
const WM_LBUTTONDBLCLK: u32 = 0x0203;
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
    /// The open prompt is the debug console (F12) rather than the host-address one.
    console: bool,
    last_frame: Option<(u32, u32, Vec<u8>)>,
    was_overlay: bool,
    /// Buttons pressed during this event pass, and releases held back one pass so that a quick
    /// click is still visible to the game, which polls the button state once per frame.
    pressed_now: Vec<usize>,
    late_release: Vec<usize>,
    /// W/A/S/D scroll the map like the arrow keys (`RL_WASD=1`, toggled with Alt+W).
    wasd: bool,
    /// Last toast text drawn, so an unchanged overlay is not re-rendered every pass.
    toast_drawn: Option<String>,
    /// The frame looks like the in-game view (the game draws its own pointer on the map only).
    ingame: bool,
    cursor_drawn: Option<(i32, i32)>,
    /// The volume options (F10), when open.
    options: Option<Options>,
    /// The on/off switches of the options menu.
    prefs: Prefs,
    mixer: crate::audio::Shared,
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
        // SDL orders the keypad digits 1..9 and then 0, so `Kp0..=Kp9` would be an empty range.
        Keycode::Kp0 => 0x60,
        _ if (Keycode::Kp1.into_i32()..=Keycode::Kp9.into_i32()).contains(&c) => 0x61 + (c - Keycode::Kp1.into_i32()) as u8,
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
        // The game draws its own pointer. SDL only hides the system one while it is over the window,
        // so it still shows outside. Grabbing is opt-in (`RL_GRAB`).
        sdl.mouse().show_cursor(false);
        if std::env::var_os("RL_GRAB").is_some() {
            canvas.window_mut().set_grab(true);
        }
        video.text_input().start();
        let audio_dev = crate::audio::open(&sdl, mixer.clone());
        if audio_dev.is_none() {
            eprintln!("warning: no audio device");
        }
        // The device stops when dropped; keep it for the life of the program.
        std::mem::forget(audio_dev);
        let events = sdl.event_pump()?;
        Ok(Frontend { _sdl: sdl, canvas, tex, tex_size: (800, 600), _creator: creator, events, shown: 0, sent_activate: false, movie_tex: None, prompt: None, console: false, last_frame: None, was_overlay: false, pressed_now: Vec::new(), late_release: Vec::new(), wasd: std::env::var_os("RL_WASD").is_some(), toast_drawn: None, ingame: false, cursor_drawn: None, options: None, prefs: Prefs::default(), mixer })
    }

    /// Applies saved switches at start-up.
    pub fn set_prefs(&mut self, prefs: Prefs) {
        self.prefs = prefs;
        self.apply_fullscreen();
    }

    fn apply_fullscreen(&mut self) {
        let kind = if self.prefs.fullscreen { sdl2::video::FullscreenType::Desktop } else { sdl2::video::FullscreenType::Off };
        let _ = self.canvas.window_mut().set_fullscreen(kind);
    }

    /// Handles window events; returns false when the user asked to quit.
    pub fn pump(&mut self, uc: &mut Uc) -> bool {
        if !self.sent_activate && !uc.get_data().windows.is_empty() {
            self.sent_activate = true;
            activate(uc);
        }
        if uc.get_data().ui.want_host_address && self.prompt.is_none() {
            self.console = false;
            uc.get_data_mut().ui.want_host_address = false;
            self.prompt = Some(uc.get_data().ui.last_host.clone());
        }
        for vk in self.late_release.drain(..) {
            uc.get_data_mut().keys[vk] = false;
        }
        self.pressed_now.clear();
        uc.get_data_mut().block_edges = !self.prefs.edge_scroll && self.ingame;
        let events: Vec<Event> = self.events.poll_iter().collect();
        for e in events {
            // While the prompt is open it gets all the keyboard input.
            if let Some(text) = self.prompt.as_mut() {
                match &e {
                    Event::TextInput { text: t, .. } => {
                        let console = self.console;
                        text.extend(t.chars().filter(|c| c.is_ascii_alphanumeric() || ".:-_[]".contains(*c) || (console && *c == ' ')))
                    }
                    Event::KeyDown { keycode: Some(Keycode::Backspace), .. } => {
                        text.pop();
                    }
                    Event::KeyDown { keycode: Some(Keycode::Return | Keycode::KpEnter), .. } => {
                        let t = text.trim().to_string();
                        self.prompt = None;
                        if self.console {
                            let r = crate::debug::run(uc, &t);
                            uc.get_data_mut().ui.toast(r);
                        } else if !t.is_empty() {
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
            // The volume options take all input while open (the game keeps running underneath).
            if self.options.is_some() {
                if !self.options_event(uc, &e) {
                    continue;
                }
                if !matches!(e, Event::Quit { .. }) {
                    continue;
                }
            }
            if let Event::KeyDown { keycode: Some(Keycode::F10), repeat: false, .. } = e {
                self.options = Some(Options::default());
                continue;
            }
            if let Event::KeyDown { keycode: Some(Keycode::F12), repeat: false, .. } = e {
                self.console = true;
                self.prompt = Some(String::new());
                continue;
            }
            match e {
                Event::Quit { .. } => return false,
                Event::MouseMotion { x, y, .. } => {
                    uc.get_data_mut().cursor = (x, y);
                    post(uc, WM_MOUSEMOVE, 0, ((y as u32) << 16) | (x as u32 & 0xffff));
                }
                Event::MouseButtonDown { mouse_btn, x, y, clicks, .. } | Event::MouseButtonUp { mouse_btn, x, y, clicks, .. } => {
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
                    // The game's window class has CS_DBLCLKS and it only raises its "map click" flag on
                    // WM_LBUTTONDBLCLK (single presses just set the button-down flag), which is how
                    // double-clicking a building ejects a settler.
                    let m = if m == WM_LBUTTONDOWN && clicks >= 2 { WM_LBUTTONDBLCLK } else { m };
                    if down {
                        uc.get_data_mut().keys[vk] = true;
                        self.pressed_now.push(vk);
                    } else if self.pressed_now.contains(&vk) {
                        self.late_release.push(vk);
                    } else {
                        uc.get_data_mut().keys[vk] = false;
                    }
                    uc.get_data_mut().cursor = (x, y);
                    post(uc, m, 0, ((y as u32) << 16) | (x as u32 & 0xffff));
                }
                Event::KeyDown { keycode: Some(k @ (Keycode::Plus | Keycode::Equals | Keycode::KpPlus | Keycode::Minus | Keycode::KpMinus)), keymod, repeat: false, .. }
                    if keymod.intersects(sdl2::keyboard::Mod::LCTRLMOD | sdl2::keyboard::Mod::RCTRLMOD) =>
                {
                    // Ctrl +/-: game speed (the loop's frame rate), in steps of 5.
                    let st = uc.get_data_mut();
                    let fps = (1.0 / st.frame_interval.as_secs_f64()).round() as i32;
                    let up = matches!(k, Keycode::Plus | Keycode::Equals | Keycode::KpPlus);
                    let fps = (fps + if up { 5 } else { -5 }).clamp(5, 120);
                    st.frame_interval = std::time::Duration::from_secs_f64(1.0 / fps as f64);
                    st.ui.toast(format!("VELOCIDADE: {fps} QUADROS POR SEGUNDO"));
                    self.was_overlay = true;
                }
                // F1-F3: 1x, 1.5x, 2x. The original skipped frames to go faster; here the pace scales instead.
                // Past 2x the display's vsync caps what the host can show, so there is nothing above.
                // F4 is the game's own frame-skip toggle; it is swallowed.
                Event::KeyDown { keycode: Some(k @ (Keycode::F1 | Keycode::F2 | Keycode::F3)), repeat: false, .. } => {
                    let (speed, label) = match k {
                        Keycode::F1 => (1.0, "1X"),
                        Keycode::F2 => (1.5, "1,5X"),
                        _ => (2.0, "2X"),
                    };
                    let st = uc.get_data_mut();
                    st.speed = speed;
                    st.ui.toast(format!("VELOCIDADE: {label}"));
                    self.was_overlay = true;
                }
                Event::KeyDown { keycode: Some(Keycode::F4), .. } => {}
                Event::KeyUp { keycode: Some(Keycode::F1 | Keycode::F2 | Keycode::F3 | Keycode::F4), .. } => {}
                Event::KeyDown { keycode: Some(Keycode::Return), keymod, repeat: false, .. } if keymod.intersects(sdl2::keyboard::Mod::LALTMOD | sdl2::keyboard::Mod::RALTMOD) => {
                    self.prefs.fullscreen = !self.prefs.fullscreen;
                    self.apply_fullscreen();
                    self.save_options(uc, self.mixer.lock().map(|m| m.volumes).unwrap_or_default());
                }
                Event::KeyDown { keycode: Some(Keycode::W), keymod, repeat: false, .. } if keymod.intersects(sdl2::keyboard::Mod::LALTMOD | sdl2::keyboard::Mod::RALTMOD) => {
                    self.wasd = !self.wasd;
                    uc.get_data_mut().ui.toast(if self.wasd { "WASD ROLA O MAPA" } else { "WASD DESLIGADO" });
                }
                // The game polls the arrow keys for scrolling, so WASD only has to set their state.
                Event::KeyDown { keycode: Some(k @ (Keycode::W | Keycode::A | Keycode::S | Keycode::D)), .. }
                | Event::KeyUp { keycode: Some(k @ (Keycode::W | Keycode::A | Keycode::S | Keycode::D)), .. }
                    if self.wasd =>
                {
                    let vk = match k {
                        Keycode::W => 0x26,
                        Keycode::A => 0x25,
                        Keycode::S => 0x28,
                        _ => 0x27,
                    };
                    uc.get_data_mut().keys[vk] = matches!(e, Event::KeyDown { .. });
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

    /// Feeds one event to the open options menu. Returns false once it has handled the event.
    fn options_event(&mut self, uc: &mut Uc, e: &Event) -> bool {
        let Some(o) = self.options.as_mut() else { return true };
        let (w, h) = (self.tex_size.0 as usize, self.tex_size.1 as usize);
        let mut vol = self.mixer.lock().map(|m| m.volumes).unwrap_or_default();
        let (before, prefs_before) = (vol, self.prefs);
        let mut close = false;
        match *e {
            Event::KeyDown { keycode: Some(k), .. } => match k {
                Keycode::Up => o.select(-1),
                Keycode::Down | Keycode::Tab => o.select(1),
                Keycode::Left => o.adjust(&mut vol, &mut self.prefs, -1),
                Keycode::Right => o.adjust(&mut vol, &mut self.prefs, 1),
                Keycode::Home if o.sel < crate::options::SLIDERS => *vol.slot(o.sel) = 0,
                Keycode::End if o.sel < crate::options::SLIDERS => *vol.slot(o.sel) = 100,
                Keycode::Escape | Keycode::F10 | Keycode::Return | Keycode::KpEnter => close = true,
                _ => {}
            },
            Event::MouseButtonDown { mouse_btn: MouseButton::Left, x, y, .. } => {
                o.pointer(&mut vol, &mut self.prefs, w, h, x, y, true);
            }
            Event::MouseMotion { x, y, .. } => {
                uc.get_data_mut().cursor = (x, y);
                if o.drag.is_some() {
                    o.pointer(&mut vol, &mut self.prefs, w, h, x, y, false);
                }
            }
            Event::MouseButtonUp { mouse_btn: MouseButton::Left, .. } => o.drag = None,
            Event::Quit { .. } => return true,
            _ => {}
        }
        if vol != before {
            if let Ok(mut m) = self.mixer.lock() {
                m.volumes = vol;
            }
        }
        if self.prefs.fullscreen != prefs_before.fullscreen {
            self.apply_fullscreen();
        }
        if close {
            self.options = None;
            self.save_options(uc, vol);
            // The next frame must redraw without the panel.
            self.was_overlay = true;
        }
        false
    }

    fn save_options(&self, uc: &Uc, vol: crate::audio::Volumes) {
        let _ = std::fs::write(uc.get_data().vfs.overlay.join(crate::options::FILE), vol.to_text() + &self.prefs.to_text());
    }

    /// The game draws its pointer only over the map; elsewhere (menus, bottom bar) we draw one.
    fn host_cursor(&self, uc: &Uc) -> Option<(i32, i32)> {
        let (x, y) = uc.get_data().cursor;
        (!(self.ingame && y < 545)).then_some((x, y))
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
        let overlay_on = self.prompt.is_some() || self.options.is_some() || toast.is_some();
        let same_toast = self.toast_drawn == toast;
        let hc = if self.options.is_some() { Some(uc.get_data().cursor) } else { self.host_cursor(uc) };
        if n == self.shown && same_toast && self.prompt.is_none() && self.options.is_none() && overlay_on == self.was_overlay && hc == self.cursor_drawn {
            return;
        }
        self.cursor_drawn = hc;
        let new_frame = n != self.shown;
        self.toast_drawn = toast.clone();
        self.shown = n;
        self.was_overlay = overlay_on;
        if n != 0 && (new_frame || self.last_frame.is_none()) {
            if let Some(f) = com::snapshot(uc) {
                self.ingame = looks_ingame(&f);
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
            let label = if self.console { "CONSOLE - ENTER EXECUTA, ESC FECHA (AJUDA: help)" } else { "ENDERECO DO ANFITRIAO (IP OU IP:PORTA) - ENTER CONFIRMA, ESC CANCELA" };
            let line = format!("{text}_");
            let bw = (overlay::text_width(label, 2) + 24).min(stride - 8);
            let (bx, by) = ((stride - bw) / 2, h as usize / 2 - 30);
            overlay::fill_rect(&mut rgba, stride, bx, by, bw, 70, [20, 12, 8, 255]);
            overlay::fill_rect(&mut rgba, stride, bx, by, bw, 2, [220, 170, 90, 255]);
            overlay::fill_rect(&mut rgba, stride, bx, by + 68, bw, 2, [220, 170, 90, 255]);
            overlay::draw_text(&mut rgba, stride, bx + 12, by + 12, label, 2, [230, 200, 140, 255]);
            overlay::draw_text(&mut rgba, stride, bx + 12, by + 40, &line, 3, [255, 255, 255, 255]);
        }
        if let Some(o) = &self.options {
            let mut vol = self.mixer.lock().map(|m| m.volumes).unwrap_or_default();
            o.draw(&mut rgba, stride, h as usize, &mut vol, self.prefs);
        }
        if let Some((cx, cy)) = hc {
            overlay::draw_arrow(&mut rgba, w as usize, h as usize, cx, cy);
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

/// The in-game screen has a grey stone bar along the bottom (rows 545..600 of an 800x600 frame).
fn looks_ingame(f: &(u32, u32, Vec<u8>)) -> bool {
    let (w, h, px) = (f.0 as usize, f.1 as usize, &f.2);
    if (w, h) != (800, 600) {
        return false;
    }
    let grey = (0..20).filter(|i| {
        let o = (590 * w + 300 + i * 9) * 4;
        let (r, g, b) = (px[o] as i32, px[o + 1] as i32, px[o + 2] as i32);
        (r - g).abs() < 16 && (g - b).abs() < 16 && (50..170).contains(&r)
    });
    grey.count() >= 12
}

/// Synthesises input for scripted (headless) runs.
pub fn inject_mouse(uc: &mut Uc, x: i32, y: i32, button: Option<(bool, bool)>) {
    inject_mouse_msg(uc, x, y, button, false)
}

/// `dbl` turns a left press into the double-click message.
pub fn inject_mouse_msg(uc: &mut Uc, x: i32, y: i32, button: Option<(bool, bool)>, dbl: bool) {
    uc.get_data_mut().cursor = (x, y);
    let lp = ((y as u32) << 16) | (x as u32 & 0xffff);
    match button {
        None => post(uc, WM_MOUSEMOVE, 0, lp),
        Some((right, down)) => {
            uc.get_data_mut().keys[if right { 2 } else { 1 }] = down;
            let m = match (right, down) {
                (false, true) if dbl => WM_LBUTTONDBLCLK,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::testing::{bare_uc, com_uc};
    use sdl2::keyboard::Mod;

    fn msgs(uc: &mut Uc) -> Vec<(u32, u32, u32)> {
        uc.get_data_mut().msgs.drain(..).map(|m| (m.message, m.wparam, m.lparam)).collect()
    }

    #[test]
    fn host_keys_map_to_windows_virtual_keys() {
        assert_eq!(vk_of(Keycode::Left), Some(0x25));
        assert_eq!(vk_of(Keycode::Return), Some(0x0d));
        assert_eq!(vk_of(Keycode::KpEnter), Some(0x0d));
        assert_eq!(vk_of(Keycode::RShift), Some(0x10));
        assert_eq!(vk_of(Keycode::LCtrl), Some(0x11));
        assert_eq!(vk_of(Keycode::RAlt), Some(0x12));
        assert_eq!(vk_of(Keycode::Delete), Some(0x2e));
        assert_eq!(vk_of(Keycode::Space), Some(0x20));
        assert_eq!(vk_of(Keycode::A), Some(b'A'));
        assert_eq!(vk_of(Keycode::Z), Some(b'Z'));
        assert_eq!(vk_of(Keycode::Num0), Some(b'0'));
        assert_eq!(vk_of(Keycode::Num9), Some(b'9'));
        assert_eq!(vk_of(Keycode::F1), Some(0x70));
        assert_eq!(vk_of(Keycode::F11), Some(0x7a));
        assert_eq!(vk_of(Keycode::Kp0), Some(0x60));
        assert_eq!(vk_of(Keycode::Kp1), Some(0x61));
        assert_eq!(vk_of(Keycode::Kp5), Some(0x65));
        assert_eq!(vk_of(Keycode::Kp9), Some(0x69));
        assert_eq!(vk_of(Keycode::Backquote), None);
    }

    #[test]
    fn injected_input_becomes_window_messages() {
        let mut uc = bare_uc();
        inject_mouse(&mut uc, 10, 20, None);
        assert_eq!(uc.get_data().cursor, (10, 20));
        inject_mouse(&mut uc, 5, 6, Some((false, true)));
        inject_mouse(&mut uc, 5, 6, Some((false, false)));
        inject_mouse(&mut uc, 5, 6, Some((true, true)));
        inject_mouse(&mut uc, 5, 6, Some((true, false)));
        inject_mouse_msg(&mut uc, 7, 8, Some((false, true)), true);
        let lp = |x: u32, y: u32| (y << 16) | x;
        assert_eq!(
            msgs(&mut uc),
            [(WM_MOUSEMOVE, 0, lp(10, 20)), (WM_LBUTTONDOWN, 0, lp(5, 6)), (WM_LBUTTONUP, 0, lp(5, 6)), (WM_RBUTTONDOWN, 0, lp(5, 6)), (WM_RBUTTONUP, 0, lp(5, 6)), (WM_LBUTTONDBLCLK, 0, lp(7, 8))]
        );
        assert!(uc.get_data().keys[1] && !uc.get_data().keys[2]);
        inject_key(&mut uc, 0x41, true);
        inject_key(&mut uc, 0x41, false);
        inject_key(&mut uc, 0x0d, true);
        assert_eq!(msgs(&mut uc), [(WM_KEYDOWN, 0x41, 1), (WM_KEYUP, 0x41, 0xc000_0001), (WM_KEYDOWN, 0x0d, 1), (WM_CHAR, 0x0d, 1)]);
        activate(&mut uc);
        assert_eq!(msgs(&mut uc), [(WM_ACTIVATEAPP, 1, 0), (WM_ACTIVATE, 1, 0), (WM_SETFOCUS, 0, 0)]);
    }

    #[test]
    fn toast_replaces_the_previous_message() {
        let mut ui = Ui::default();
        ui.toast("one");
        ui.toast(String::from("two"));
        assert_eq!(ui.toast.as_ref().map(|t| t.0.as_str()), Some("two"));
    }

    #[test]
    fn ingame_view_is_recognised_by_the_stone_bar() {
        let bar = |c: [u8; 4]| {
            let mut px = vec![0u8; 800 * 600 * 4];
            for x in 0..800 {
                px[(590 * 800 + x) * 4..(590 * 800 + x) * 4 + 4].copy_from_slice(&c);
            }
            (800, 600, px)
        };
        assert!(looks_ingame(&bar([100, 100, 100, 255])));
        assert!(!looks_ingame(&bar([200, 40, 40, 255])), "coloured menu");
        assert!(!looks_ingame(&bar([10, 10, 10, 255])), "too dark");
        assert!(!looks_ingame(&(640, 480, vec![0; 640 * 480 * 4])), "only the 800x600 layout is checked");
    }

    /// SDL's Rust wrapper cannot queue text-input events, so push the raw event.
    fn push_text(text: &str) {
        // SAFETY: a zeroed SDL_Event is valid; the text event is fully initialised before it is pushed, and SDL copies it.
        unsafe {
            let mut e: sdl2::sys::SDL_Event = std::mem::zeroed();
            e.text.type_ = sdl2::sys::SDL_EventType::SDL_TEXTINPUT as u32;
            for (i, b) in text.bytes().take(31).enumerate() {
                e.text.text[i] = b as _;
            }
            assert_eq!(sdl2::sys::SDL_PushEvent(&mut e), 1);
        }
    }

    /// Everything that needs a window runs in one test: SDL allows a single live context, and the
    /// dummy video/audio drivers keep it headless.
    #[test]
    fn window_events_are_translated_for_the_guest() {
        std::env::set_var("SDL_VIDEODRIVER", "dummy");
        std::env::set_var("SDL_AUDIODRIVER", "dummy");
        let mixer: crate::audio::Shared = Default::default();
        let mx = mixer.clone();
        let mut fe = match Frontend::new(1, mixer) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("skipping: no dummy SDL video driver here ({e})");
                return;
            }
        };
        let mut uc = com_uc();
        let ev = fe._sdl.event().unwrap();
        let key = |k: Keycode, down: bool, md: Mod| {
            let (timestamp, window_id, scancode, repeat) = (0, 0, None, false);
            if down { Event::KeyDown { timestamp, window_id, keycode: Some(k), scancode, keymod: md, repeat } } else { Event::KeyUp { timestamp, window_id, keycode: Some(k), scancode, keymod: md, repeat } }
        };
        let mouse = |down: Option<bool>, btn: MouseButton, x: i32, y: i32, clicks: u8| match down {
            Some(true) => Event::MouseButtonDown { timestamp: 0, window_id: 0, which: 0, mouse_btn: btn, clicks, x, y },
            Some(false) => Event::MouseButtonUp { timestamp: 0, window_id: 0, which: 0, mouse_btn: btn, clicks, x, y },
            None => Event::MouseMotion { timestamp: 0, window_id: 0, which: 0, mousestate: sdl2::mouse::MouseState::from_sdl_state(0), x, y, xrel: 0, yrel: 0 },
        };
        let none = Mod::empty();
        // Window creation queues focus events; start from a clean slate.
        for _ in 0..3 {
            fe.pump(&mut uc);
        }
        msgs(&mut uc);

        // Keyboard and mouse.
        for e in [key(Keycode::Left, true, none), key(Keycode::Left, false, none), key(Keycode::Return, true, none), mouse(None, MouseButton::Left, 12, 34, 0), mouse(Some(true), MouseButton::Left, 12, 34, 1), mouse(Some(false), MouseButton::Left, 12, 34, 1), mouse(Some(true), MouseButton::Right, 1, 2, 1), mouse(Some(true), MouseButton::Left, 3, 4, 2)] {
            ev.push_event(e).unwrap();
        }
        assert!(fe.pump(&mut uc));
        let got = msgs(&mut uc);
        let kinds: Vec<u32> = got.iter().map(|m| m.0).collect();
        assert_eq!(kinds, [WM_KEYDOWN, WM_KEYUP, WM_KEYDOWN, WM_CHAR, WM_MOUSEMOVE, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_RBUTTONDOWN, WM_LBUTTONDBLCLK]);
        assert_eq!(got[0].1, 0x25);
        assert_eq!(uc.get_data().cursor, (3, 4));
        // The left press and release in one pass: the release is held back one pass so the game sees the button.
        assert!(uc.get_data().keys[1], "released late, not in the same pass");
        assert!(fe.pump(&mut uc));
        assert!(!uc.get_data().keys[1], "the next pass applies the late release");

        // Speed keys and Ctrl +/-.
        ev.push_event(key(Keycode::F3, true, none)).unwrap();
        assert!(fe.pump(&mut uc));
        assert_eq!(uc.get_data().speed, 2.0);
        assert!(uc.get_data().ui.toast.as_ref().is_some_and(|t| t.0.contains("2X")));
        ev.push_event(key(Keycode::F1, true, none)).unwrap();
        ev.push_event(key(Keycode::KpPlus, true, Mod::LCTRLMOD)).unwrap();
        fe.pump(&mut uc);
        assert_eq!(uc.get_data().speed, 1.0);
        assert_eq!((1.0 / uc.get_data().frame_interval.as_secs_f64()).round() as i32, 35);
        ev.push_event(key(Keycode::Minus, true, Mod::RCTRLMOD)).unwrap();
        fe.pump(&mut uc);
        assert_eq!((1.0 / uc.get_data().frame_interval.as_secs_f64()).round() as i32, 30);

        // WASD scrolling is opt-in with Alt+W, and then sets the arrow-key state.
        ev.push_event(key(Keycode::W, true, none)).unwrap();
        fe.pump(&mut uc);
        assert!(!uc.get_data().keys[0x26], "WASD is off by default");
        assert_eq!(msgs(&mut uc).iter().filter(|m| m.1 == b'W' as u32).count(), 1);
        ev.push_event(key(Keycode::W, true, Mod::LALTMOD)).unwrap();
        ev.push_event(key(Keycode::W, true, none)).unwrap();
        ev.push_event(key(Keycode::A, true, none)).unwrap();
        fe.pump(&mut uc);
        assert!(uc.get_data().keys[0x26] && uc.get_data().keys[0x25]);
        ev.push_event(key(Keycode::W, false, none)).unwrap();
        fe.pump(&mut uc);
        assert!(!uc.get_data().keys[0x26]);

        // Text typed outside the prompt reaches the game as WM_CHAR.
        msgs(&mut uc);
        push_text("hi");
        fe.pump(&mut uc);
        assert_eq!(msgs(&mut uc), [(WM_CHAR, b'h' as u32, 1), (WM_CHAR, b'i' as u32, 1)]);

        // F12 opens the console prompt: keys go to it instead of the game, Enter runs the command.
        ev.push_event(key(Keycode::F12, true, none)).unwrap();
        push_text("poke 401000 9 1");
        ev.push_event(key(Keycode::Return, true, none)).unwrap();
        fe.pump(&mut uc);
        assert_eq!(crate::machine::rd_bytes(&uc, 0x40_1000, 1), [9]);
        assert!(msgs(&mut uc).is_empty(), "nothing leaked to the game");
        assert!(fe.prompt.is_none());
        // The host-address prompt: asked for by the join screen, Backspace edits, Enter submits, Escape cancels.
        uc.get_data_mut().ui.want_host_address = true;
        uc.get_data_mut().ui.last_host = "10.0.0.1".into();
        ev.push_event(key(Keycode::Backspace, true, none)).unwrap();
        push_text("5 !");
        ev.push_event(key(Keycode::Return, true, none)).unwrap();
        fe.pump(&mut uc);
        assert_eq!(uc.get_data().ui.submitted_host.as_deref(), Some("10.0.0.5"));
        uc.get_data_mut().ui.want_host_address = true;
        ev.push_event(key(Keycode::Escape, true, none)).unwrap();
        fe.pump(&mut uc);
        assert!(fe.prompt.is_none());
        assert_eq!(uc.get_data().ui.submitted_host.as_deref(), Some("10.0.0.5"));

        // F10 opens the volume options: keys and mouse adjust the sliders instead of reaching the game.
        let vols = || mx.lock().unwrap().volumes;
        msgs(&mut uc);
        ev.push_event(key(Keycode::F10, true, none)).unwrap();
        ev.push_event(key(Keycode::Left, true, none)).unwrap();
        ev.push_event(key(Keycode::Down, true, none)).unwrap();
        for _ in 0..3 {
            ev.push_event(key(Keycode::Left, true, none)).unwrap();
        }
        ev.push_event(key(Keycode::Down, true, none)).unwrap();
        ev.push_event(key(Keycode::End, true, none)).unwrap();
        ev.push_event(key(Keycode::Down, true, none)).unwrap();
        ev.push_event(key(Keycode::Home, true, none)).unwrap();
        ev.push_event(mouse(Some(true), MouseButton::Right, 400, 300, 1)).unwrap();
        fe.pump(&mut uc);
        assert!(fe.options.is_some());
        assert_eq!(vols(), crate::audio::Volumes { music: 95, hammer: 85, effects: 100, speech: 0 });
        assert!(msgs(&mut uc).is_empty(), "nothing leaked to the game");
        assert!(!uc.get_data().keys[3], "right button did not reach the game");
        // Dragging with the mouse: press on the first bar's middle, move, release.
        let bar = |row: usize| (crate::options::bar_x(800), crate::options::row_y(800, 600, row) as i32 + 4);
        let (bx, by) = bar(0);
        ev.push_event(mouse(Some(true), MouseButton::Left, bx as i32 + 100, by, 1)).unwrap();
        ev.push_event(mouse(None, MouseButton::Left, bx as i32 + 40, by + 90, 0)).unwrap();
        ev.push_event(mouse(Some(false), MouseButton::Left, bx as i32 + 40, by + 90, 1)).unwrap();
        ev.push_event(mouse(None, MouseButton::Left, bx as i32 + 190, by, 0)).unwrap();
        fe.pump(&mut uc);
        assert_eq!(vols().music, 20, "released drag stops following the mouse");
        assert_eq!(uc.get_data().cursor, (bx as i32 + 190, by));
        // Drawn over a frame, with the pointer, then closed with Escape: the volumes are saved.
        let dir = std::env::temp_dir().join(format!("rl-options-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        uc.get_data_mut().vfs.overlay = dir.clone();
        ev.push_event(key(Keycode::Escape, true, none)).unwrap();
        fe.pump(&mut uc);
        assert!(fe.options.is_none());
        let saved = std::fs::read_to_string(dir.join(crate::options::FILE)).unwrap();
        assert_eq!(crate::audio::Volumes::parse(&saved), vols());
        std::fs::remove_dir_all(&dir).ok();
        ev.push_event(key(Keycode::F10, true, none)).unwrap();
        ev.push_event(key(Keycode::F10, true, none)).unwrap();
        fe.pump(&mut uc);
        assert!(fe.options.is_none(), "F10 toggles it closed again");

        // The switches: F10, down to "edge scroll", Left turns it off; saved with the volumes.
        uc.get_data_mut().vfs.overlay = dir.clone();
        std::fs::create_dir_all(&dir).unwrap();
        fe.ingame = true;
        for k in [Keycode::F10, Keycode::Up, Keycode::Up, Keycode::Left, Keycode::Escape] {
            ev.push_event(key(k, true, none)).unwrap();
        }
        fe.pump(&mut uc);
        assert!(!fe.prefs.edge_scroll && !fe.prefs.fullscreen);
        fe.pump(&mut uc);
        assert!(uc.get_data().block_edges, "the game no longer sees the pointer at the edges");
        let saved = std::fs::read_to_string(dir.join(crate::options::FILE)).unwrap();
        assert_eq!(Prefs::parse(&saved), fe.prefs);
        fe.ingame = false;
        std::fs::remove_dir_all(&dir).ok();

        // Drawing: a toast and a prompt over a presented frame, then a movie frame.
        let mem = uc.get_data_mut().alloc(640 * 480);
        uc.get_data_mut().com.objs.insert(0x100, crate::com::Obj::Surface(crate::com::Surface { w: 640, h: 480, mem, palette: None, back: None, key: None }));
        uc.get_data_mut().com.primary = Some(0x100);
        uc.get_data_mut().com.frames_presented = 1;
        uc.get_data_mut().ui.toast("hello");
        fe.present(&mut uc);
        assert!(fe.last_frame.is_some() && fe.toast_drawn.as_deref() == Some("hello"));
        fe.prompt = Some("abc".into());
        fe.present(&mut uc);
        uc.get_data_mut().com.frames_presented = 2;
        fe.present(&mut uc);
        assert_eq!(fe.shown, 2);
        assert_eq!(fe.tex_size, (640, 480), "the texture follows the game's resolution");
        let avi = crate::video::Avi::parse(b"RIFF\0\0\0\0AVI ").unwrap();
        let mut player = crate::video::Player::new(avi);
        player.avi.width = 4;
        player.avi.height = 4;
        player.dec = crate::video::Cinepak::new(4, 4);
        player.dirty = true;
        uc.get_data_mut().video = Some(player);
        fe.present(&mut uc);
        assert!(fe.movie_tex.is_some());
        assert!(!uc.get_data().video.as_ref().unwrap().dirty);

        // Closing the window ends the loop.
        ev.push_event(Event::Quit { timestamp: 0 }).unwrap();
        assert!(!fe.pump(&mut uc));
    }
}
