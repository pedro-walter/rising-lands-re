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
            Ok(mut m) => {
                m.mix(out);
                if std::env::var_os("RL_AUDIO_LOG").is_some() {
                    let peak = out.iter().map(|s| s.unsigned_abs()).max().unwrap_or(0);
                    let playing = m.voices.values().filter(|v| v.playing).count();
                    eprintln!("audio: peak {peak} voices {playing} cd {}", m.cd.is_some());
                }
            }
            Err(_) => out.fill(0),
        }
    }
}

/// Opens the default playback device. Returns `None` when there is no audio.
pub fn open(sdl: &sdl2::Sdl, mixer: Shared) -> Option<sdl2::audio::AudioDevice<Output>> {
    let audio = sdl.audio().map_err(|e| eprintln!("audio init: {e}")).ok()?;
    let spec = sdl2::audio::AudioSpecDesired { freq: Some(OUT_RATE as i32), channels: Some(2), samples: Some(1024) };
    let dev = audio.open_playback(None, &spec, |_| Output(mixer)).map_err(|e| eprintln!("audio open: {e}")).ok()?;
    dev.resume();
    Some(dev)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn voice16(samples: &[i16], rate: u32) -> Voice {
        let mut v = Voice::new(0);
        v.data = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        v.bits = 16;
        v.rate = rate;
        v
    }

    #[test]
    fn decibel_gain() {
        assert_eq!(db_gain(0), 1.0);
        assert!((db_gain(-2000) - 0.1).abs() < 1e-6);
        assert!((db_gain(-6000) - 0.001).abs() < 1e-6);
        assert_eq!(db_gain(500), 1.0, "clamped at unity");
        assert_eq!(db_gain(-99999), db_gain(-10000));
    }

    #[test]
    fn voice_geometry_and_samples() {
        let v = Voice::new(100);
        assert_eq!((v.channels, v.rate, v.bits, v.frames()), (1, 22050, 8, 100));
        assert_eq!(v.sample(0, 0), -1.0, "8-bit silence is 0x00 -> -1.0 (zero-filled buffer)");
        let mut v = voice16(&[16384, -16384, 8192, 0], 44100);
        v.channels = 2;
        assert_eq!(v.frames(), 2);
        assert_eq!((v.sample(0, 0), v.sample(0, 1)), (0.5, -0.5));
        assert_eq!(v.sample(1, 5), 0.0, "channel index is clamped to the last one");
        let mut m = Voice::new(1);
        m.data[0] = 192;
        assert_eq!(m.sample(0, 1), 0.5, "mono voices answer both sides");
    }

    #[test]
    fn mixes_a_playing_voice_to_both_sides_and_stops_at_the_end() {
        let mut mx = Mixer::default();
        let mut v = voice16(&[16384, 16384], 44100);
        v.playing = true;
        mx.voices.insert(1, v);
        let mut out = [0i16; 8];
        mx.mix(&mut out);
        assert_eq!(out[..4], [16383, 16383, 16383, 16383]);
        assert_eq!(out[4..], [0, 0, 0, 0], "silence after the buffer ends");
        let v = &mx.voices[&1];
        assert!(!v.playing);
        assert_eq!(v.pos, 0.0);
        // A stopped voice stays silent.
        let mut out = [1i16; 4];
        mx.mix(&mut out);
        assert_eq!(out, [0; 4]);
    }

    #[test]
    fn looping_voice_wraps() {
        let mut mx = Mixer::default();
        let mut v = voice16(&[8192, -8192], 44100);
        v.playing = true;
        v.looping = true;
        mx.voices.insert(1, v);
        let mut out = [0i16; 8];
        mx.mix(&mut out);
        assert_eq!(out[0], 8191);
        assert_eq!(out[2], -8191);
        assert_eq!(out[4], 8191, "wrapped back to the start");
        assert!(mx.voices[&1].playing);
    }

    #[test]
    fn pan_attenuates_the_far_side_and_frequency_changes_speed() {
        let mut mx = Mixer::default();
        let mut v = voice16(&[20000; 64], 44100);
        v.playing = true;
        v.pan = 2000; // right: left drops 20 dB
        mx.voices.insert(1, v);
        let mut out = [0i16; 2];
        mx.mix(&mut out);
        assert!(out[1] > 19000 && (out[0] - out[1] / 10).abs() < 10, "{out:?}");

        let mut mx = Mixer::default();
        let mut v = voice16(&[0, 1000, 2000, 3000], 44100);
        v.playing = true;
        v.freq = Some(88200); // twice as fast: every other frame
        mx.voices.insert(1, v);
        let mut out = [0i16; 4];
        mx.mix(&mut out);
        assert_eq!((out[0], out[2]), (0, 1999));
    }

    #[test]
    fn cd_track_plays_with_volume_and_clears_at_the_end() {
        let mut mx = Mixer::default();
        let data: Vec<u8> = [16384i16, -16384, 16384, -16384].iter().flat_map(|s| s.to_le_bytes()).collect();
        mx.cd = Some(Cd { data: Arc::new(data), pos: 0 });
        mx.cd_volume = 0.5;
        let mut out = [0i16; 6];
        mx.mix(&mut out);
        assert_eq!(out[..4], [8191, -8191, 8191, -8191]);
        assert_eq!(out[4..], [0, 0]);
        assert!(mx.cd.is_none());
    }

    #[test]
    fn output_is_clipped_not_wrapped() {
        let mut mx = Mixer::default();
        for id in 0..3 {
            let mut v = voice16(&[30000; 4], 44100);
            v.playing = true;
            mx.voices.insert(id, v);
        }
        let mut out = [0i16; 2];
        mx.mix(&mut out);
        assert_eq!(out, [32767, 32767]);
    }
}
