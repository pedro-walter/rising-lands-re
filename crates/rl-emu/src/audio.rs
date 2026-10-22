//! Software mixer for DirectSound buffers and CD audio, plus the SDL playback callback.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

pub const OUT_RATE: u32 = 44100;

#[derive(Clone)]
pub struct Voice {
    pub data: Vec<u8>,
    pub channels: u16,
    pub rate: u32,
    pub bits: u16,
    /// Hundredths of a dB, `-10000..=0`.
    pub volume: i32,
    /// Hundredths of a dB, `-10000` (full left) to `10000` (full right).
    pub pan: i32,
    pub freq: Option<u32>,
    pub playing: bool,
    pub looping: bool,
    /// Play position in source frames.
    pub pos: f64,
}

impl Voice {
    pub fn new(size: usize) -> Voice {
        Voice { data: vec![0; size], channels: 1, rate: 22050, bits: 8, volume: 0, pan: 0, freq: None, playing: false, looping: false, pos: 0.0 }
    }

    fn frame_bytes(&self) -> usize {
        (self.channels as usize * self.bits as usize / 8).max(1)
    }

    pub fn frames(&self) -> usize {
        self.data.len() / self.frame_bytes()
    }

    fn sample(&self, frame: usize, ch: usize) -> f32 {
        let fb = self.frame_bytes();
        let ch = ch.min(self.channels as usize - 1);
        let at = frame * fb + ch * (self.bits as usize / 8);
        if self.bits == 16 {
            i16::from_le_bytes([self.data[at], self.data[at + 1]]) as f32 / 32768.0
        } else {
            (self.data[at] as f32 - 128.0) / 128.0
        }
    }
}

fn db_gain(hundredths: i32) -> f32 {
    10f32.powf(hundredths.clamp(-10000, 0) as f32 / 2000.0)
}

pub struct Cd {
    pub data: Arc<Vec<u8>>,
    /// Position in bytes.
    pub pos: usize,
}

#[derive(Default)]
pub struct Mixer {
    pub voices: HashMap<u32, Voice>,
    pub cd: Option<Cd>,
    pub cd_volume: f32,
}

impl Mixer {
    /// Fills interleaved stereo `i16` output.
    pub fn mix(&mut self, out: &mut [i16]) {
        let frames = out.len() / 2;
        let mut acc = vec![0f32; frames * 2];
        for v in self.voices.values_mut().filter(|v| v.playing && v.frames() > 0) {
            let step = v.freq.unwrap_or(v.rate) as f64 / OUT_RATE as f64;
            let gain = db_gain(v.volume);
            let (gl, gr) = if v.pan > 0 { (db_gain(-v.pan) * gain, gain) } else { (gain, db_gain(v.pan) * gain) };
            let n = v.frames();
            for i in 0..frames {
                let f = v.pos as usize;
                if f >= n {
                    if v.looping {
                        v.pos -= n as f64;
                    } else {
                        v.playing = false;
                        v.pos = 0.0;
                        break;
                    }
                }
                let f = v.pos as usize;
                let (l, r) = (v.sample(f, 0), v.sample(f, 1));
                acc[i * 2] += l * gl;
                acc[i * 2 + 1] += r * gr;
                v.pos += step;
            }
        }
        if let Some(cd) = self.cd.as_mut() {
            let gain = if self.cd_volume > 0.0 { self.cd_volume } else { 1.0 };
            for i in 0..frames {
                if cd.pos + 4 > cd.data.len() {
                    self.cd = None;
                    break;
                }
                let d = &cd.data[cd.pos..cd.pos + 4];
                acc[i * 2] += i16::from_le_bytes([d[0], d[1]]) as f32 / 32768.0 * gain;
                acc[i * 2 + 1] += i16::from_le_bytes([d[2], d[3]]) as f32 / 32768.0 * gain;
                cd.pos += 4;
            }
        }
        for (o, a) in out.iter_mut().zip(&acc) {
            *o = (a.clamp(-1.0, 1.0) * 32767.0) as i16;
        }
    }
}

pub type Shared = Arc<Mutex<Mixer>>;

pub struct Output(pub Shared);

impl sdl2::audio::AudioCallback for Output {
    type Channel = i16;

    fn callback(&mut self, out: &mut [i16]) {
        match self.0.lock() {
            Ok(mut m) => m.mix(out),
            Err(_) => out.fill(0),
        }
    }
}

/// Opens the default playback device. Returns `None` when there is no audio.
pub fn open(sdl: &sdl2::Sdl, mixer: Shared) -> Option<sdl2::audio::AudioDevice<Output>> {
    let audio = sdl.audio().ok()?;
    let spec = sdl2::audio::AudioSpecDesired { freq: Some(OUT_RATE as i32), channels: Some(2), samples: Some(1024) };
    let dev = audio.open_playback(None, &spec, |_| Output(mixer)).ok()?;
    dev.resume();
    Some(dev)
}
