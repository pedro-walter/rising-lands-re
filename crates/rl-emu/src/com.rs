//! DirectDraw and DirectSound COM objects, implemented on the host.
//!
//! A COM object in the guest is a pointer to a pointer to a vtable. Each vtable
//! slot points at a `ret N` stub; the code hook in [`crate::machine`] routes the
//! call here by stub address.

use crate::machine::*;
use std::collections::HashMap;

#[derive(Default)]
pub struct Com {
    pub next_stub: u32,
    methods: HashMap<u32, (&'static str, &'static str)>,
    vtables: HashMap<&'static str, u32>,
    pub objs: HashMap<u32, Obj>,
    pub refs: HashMap<u32, u32>,
    pub display: (u32, u32),
    pub primary: Option<u32>,
    pub frames_presented: u64,
    pub dplay: Option<crate::dplay::Dp>,
}

pub enum Obj {
    DDraw,
    Surface(Surface),
    Palette([u8; 1024]),
    DSound,
    DsBuffer { mem: u32, size: u32, primary: bool },
    DPlay,
}

pub struct Surface {
    pub w: u32,
    pub h: u32,
    pub mem: u32,
    pub palette: Option<u32>,
    pub back: Option<u32>,
    pub key: Option<u32>,
}

type Method = (&'static str, u32);

const DDRAW: &[Method] = &[
    ("QueryInterface", 3), ("AddRef", 1), ("Release", 1), ("Compact", 1), ("CreateClipper", 4), ("CreatePalette", 5),
    ("CreateSurface", 4), ("DuplicateSurface", 3), ("EnumDisplayModes", 5), ("EnumSurfaces", 5), ("FlipToGDISurface", 1),
    ("GetCaps", 3), ("GetDisplayMode", 2), ("GetFourCCCodes", 3), ("GetGDISurface", 2), ("GetMonitorFrequency", 2),
    ("GetScanLine", 2), ("GetVerticalBlankStatus", 2), ("Initialize", 2), ("RestoreDisplayMode", 1),
    ("SetCooperativeLevel", 3), ("SetDisplayMode", 4), ("WaitForVerticalBlank", 3),
];

const SURFACE: &[Method] = &[
    ("QueryInterface", 3), ("AddRef", 1), ("Release", 1), ("AddAttachedSurface", 2), ("AddOverlayDirtyRect", 2),
    ("Blt", 6), ("BltBatch", 4), ("BltFast", 6), ("DeleteAttachedSurface", 3), ("EnumAttachedSurfaces", 3),
    ("EnumOverlayZOrders", 4), ("Flip", 3), ("GetAttachedSurface", 3), ("GetBltStatus", 2), ("GetCaps", 2),
    ("GetClipper", 2), ("GetColorKey", 3), ("GetDC", 2), ("GetFlipStatus", 2), ("GetOverlayPosition", 3),
    ("GetPalette", 2), ("GetPixelFormat", 2), ("GetSurfaceDesc", 2), ("Initialize", 3), ("IsLost", 1), ("Lock", 5),
    ("ReleaseDC", 2), ("Restore", 1), ("SetClipper", 2), ("SetColorKey", 3), ("SetOverlayPosition", 3),
    ("SetPalette", 2), ("Unlock", 2), ("UpdateOverlay", 6), ("UpdateOverlayDisplay", 2), ("UpdateOverlayZOrder", 3),
    ("GetDDInterface", 2), ("PageLock", 2), ("PageUnlock", 2),
];

const PALETTE: &[Method] =
    &[("QueryInterface", 3), ("AddRef", 1), ("Release", 1), ("GetCaps", 2), ("GetEntries", 5), ("Initialize", 4), ("SetEntries", 5)];

const DPLAY: &[Method] = &[
    ("QueryInterface", 3), ("AddRef", 1), ("Release", 1), ("AddPlayerToGroup", 3), ("Close", 1), ("CreateGroup", 6),
    ("CreatePlayer", 7), ("DeletePlayerFromGroup", 3), ("DestroyGroup", 2), ("DestroyPlayer", 2), ("EnumGroupPlayers", 6),
    ("EnumGroups", 5), ("EnumPlayers", 5), ("EnumSessions", 6), ("GetCaps", 3), ("GetGroupData", 5), ("GetGroupName", 4),
    ("GetMessageCount", 3), ("GetPlayerAddress", 4), ("GetPlayerCaps", 4), ("GetPlayerData", 5), ("GetPlayerName", 4),
    ("GetSessionDesc", 3), ("Initialize", 2), ("Open", 3), ("Receive", 6), ("Send", 6), ("SetGroupData", 5),
    ("SetGroupName", 4), ("SetPlayerData", 5), ("SetPlayerName", 4), ("SetSessionDesc", 3),
    // IDirectPlay3
    ("AddGroupToGroup", 3), ("CreateGroupInGroup", 7), ("DeleteGroupFromGroup", 3), ("EnumConnections", 5),
    ("EnumGroupsInGroup", 6), ("GetGroupConnectionSettings", 5), ("InitializeConnection", 3), ("SecureOpen", 4),
    ("SendChatMessage", 5), ("SetGroupConnectionSettings", 4), ("StartSession", 4), ("GetGroupFlags", 3),
    ("GetGroupParent", 3), ("GetPlayerAccount", 5), ("GetPlayerFlags", 3),
];

const DSOUND: &[Method] = &[
    ("QueryInterface", 3), ("AddRef", 1), ("Release", 1), ("CreateSoundBuffer", 4), ("GetCaps", 2),
    ("DuplicateSoundBuffer", 3), ("SetCooperativeLevel", 3), ("Compact", 1), ("GetSpeakerConfig", 2),
    ("SetSpeakerConfig", 2), ("Initialize", 2),
];

const DSBUFFER: &[Method] = &[
    ("QueryInterface", 3), ("AddRef", 1), ("Release", 1), ("GetCaps", 2), ("GetCurrentPosition", 3), ("GetFormat", 4),
    ("GetVolume", 2), ("GetPan", 2), ("GetFrequency", 2), ("GetStatus", 2), ("Initialize", 3), ("Lock", 8), ("Play", 4),
    ("SetCurrentPosition", 2), ("SetFormat", 2), ("SetVolume", 2), ("SetPan", 2), ("SetFrequency", 2), ("Stop", 1),
    ("Unlock", 5), ("Restore", 1),
];

fn vtable(uc: &mut Uc, iface: &'static str, list: &'static [Method]) -> u32 {
    if let Some(&v) = uc.get_data().com.vtables.get(iface) {
        return v;
    }
    let vt = uc.get_data_mut().alloc(list.len() as u32 * 4);
    for (i, (name, nargs)) in list.iter().enumerate() {
        let stub = uc.get_data().com.next_stub;
        uc.get_data_mut().com.next_stub += 16;
        let bytes = nargs * 4;
        wr_bytes(uc, stub, &[0xc2, bytes as u8, (bytes >> 8) as u8]);
        uc.get_data_mut().com.methods.insert(stub, (iface, name));
        wr32(uc, vt + i as u32 * 4, stub);
    }
    uc.get_data_mut().com.vtables.insert(iface, vt);
    vt
}

fn new_object(uc: &mut Uc, iface: &'static str, list: &'static [Method], obj: Obj) -> u32 {
    let vt = vtable(uc, iface, list);
    let p = uc.get_data_mut().alloc(16);
    wr32(uc, p, vt);
    uc.get_data_mut().com.objs.insert(p, obj);
    uc.get_data_mut().com.refs.insert(p, 1);
    p
}

pub fn direct_draw_create(uc: &mut Uc, out: u32) -> u32 {
    let o = new_object(uc, "IDirectDraw", DDRAW, Obj::DDraw);
    wr32(uc, out, o);
    0
}

pub fn new_dplay_object(uc: &mut Uc) -> u32 {
    new_object(uc, "IDirectPlay", DPLAY, Obj::DPlay)
}

pub fn direct_sound_create(uc: &mut Uc, out: u32) -> u32 {
    let o = new_object(uc, "IDirectSound", DSOUND, Obj::DSound);
    wr32(uc, out, o);
    0
}

fn new_surface(uc: &mut Uc, w: u32, h: u32) -> u32 {
    let mem = uc.get_data_mut().alloc(w * h);
    new_object(uc, "IDirectDrawSurface", SURFACE, Obj::Surface(Surface { w, h, mem, palette: None, back: None, key: None }))
}

const DDERR_UNSUPPORTED: u32 = 0x8876_020c;

/// (x, y, w, h) clipped to a surface; `rect` points at a RECT or is NULL.
fn rect_of(uc: &Uc, rect: u32, w: u32, h: u32) -> (u32, u32, u32, u32) {
    if rect == 0 {
        return (0, 0, w, h);
    }
    let (l, t, r, b) = (rd32(uc, rect) as i32, rd32(uc, rect + 4) as i32, rd32(uc, rect + 8) as i32, rd32(uc, rect + 12) as i32);
    let (l, t) = (l.clamp(0, w as i32) as u32, t.clamp(0, h as i32) as u32);
    let (r, b) = (r.clamp(l as i32, w as i32) as u32, b.clamp(t as i32, h as i32) as u32);
    (l, t, r - l, b - t)
}

fn surf_info(uc: &Uc, obj: u32) -> Option<(u32, u32, u32, Option<u32>)> {
    match uc.get_data().com.objs.get(&obj)? {
        Obj::Surface(s) => Some((s.w, s.h, s.mem, s.key)),
        _ => None,
    }
}

fn blit(uc: &mut Uc, dst: u32, dpos: (u32, u32), src: u32, srect: (u32, u32, u32, u32), key: Option<u32>) {
    let (Some((dw, dh, dmem, _)), Some((sw, _sh, smem, _))) = (surf_info(uc, dst), surf_info(uc, src)) else { return };
    let (sx, sy, w, h) = srect;
    let w = w.min(dw.saturating_sub(dpos.0));
    let h = h.min(dh.saturating_sub(dpos.1));
    for row in 0..h {
        let line = rd_bytes(uc, smem + (sy + row) * sw + sx, w as usize);
        let da = dmem + (dpos.1 + row) * dw + dpos.0;
        match key {
            None => wr_bytes(uc, da, &line),
            Some(k) => {
                let mut cur = rd_bytes(uc, da, w as usize);
                for (d, s) in cur.iter_mut().zip(&line) {
                    if *s as u32 != k {
                        *d = *s;
                    }
                }
                wr_bytes(uc, da, &cur);
            }
        }
    }
}

pub fn call_method(uc: &mut Uc, addr: u32, a: &[u32; 14]) -> Option<u32> {
    let (iface, name) = *uc.get_data().com.methods.get(&addr)?;
    if uc.get_data().log_calls {
        eprintln!("[com] {iface}::{name}({:#x}, {:#x}, {:#x}, {:#x}, {:#x})", a[0], a[1], a[2], a[3], a[4]);
    }
    let this = a[0];
    Some(match (iface, name) {
        (_, "AddRef") => {
            let r = uc.get_data_mut().com.refs.entry(this).or_insert(0);
            *r += 1;
            *r
        }
        (_, "Release") => {
            let r = uc.get_data_mut().com.refs.entry(this).or_insert(1);
            *r = r.saturating_sub(1);
            let n = *r;
            if n == 0 {
                // Stop and forget sound buffers so they don't keep playing or mixing.
                uc.get_data().mixer.lock().unwrap().voices.remove(&this);
            }
            n
        }
        ("IDirectPlay", "QueryInterface") => {
            // Any interface the game asks for is served by the same object type.
            let o = new_dplay_object(uc);
            wr32(uc, a[2], o);
            0
        }
        (_, "QueryInterface") => 0x8000_4002,
        ("IDirectDraw", "SetCooperativeLevel") => 0,
        ("IDirectDraw", "SetDisplayMode") => {
            uc.get_data_mut().com.display = (a[1], a[2]);
            0
        }
        ("IDirectDraw", "RestoreDisplayMode") | ("IDirectDraw", "FlipToGDISurface") => 0,
        ("IDirectDraw", "WaitForVerticalBlank") => 0,
        ("IDirectDraw", "GetCaps") => 0,
        ("IDirectDraw", "GetDisplayMode") => {
            let (w, h) = uc.get_data().com.display;
            wr32(uc, a[1] + 4, 0x2 | 0x4 | 0x1000 | 0x8000); // HEIGHT|WIDTH|PIXELFORMAT|REFRESH-ish
            wr32(uc, a[1] + 8, h);
            wr32(uc, a[1] + 12, w);
            wr32(uc, a[1] + 16, w);
            wr32(uc, a[1] + 72, 32);
            wr32(uc, a[1] + 76, 0x20);
            wr32(uc, a[1] + 84, 8);
            0
        }
        ("IDirectDraw", "CreatePalette") => {
            let mut e = [0u8; 1024];
            if a[2] != 0 {
                e.copy_from_slice(&rd_bytes(uc, a[2], 1024));
            }
            let p = new_object(uc, "IDirectDrawPalette", PALETTE, Obj::Palette(e));
            wr32(uc, a[3], p);
            0
        }
        ("IDirectDraw", "CreateSurface") => {
            let desc = a[1];
            let (flags, h, w, caps) = (rd32(uc, desc + 4), rd32(uc, desc + 8), rd32(uc, desc + 12), rd32(uc, desc + 104));
            let (w, h) = if caps & 0x200 != 0 { uc.get_data().com.display } else { (w, h) };
            let s = new_surface(uc, w, h);
            if caps & 0x200 != 0 {
                uc.get_data_mut().com.primary = Some(s);
                if flags & 0x20 != 0 {
                    let back = new_surface(uc, w, h);
                    if let Some(Obj::Surface(p)) = uc.get_data_mut().com.objs.get_mut(&s) {
                        p.back = Some(back);
                    }
                }
            }
            if std::env::var_os("RL_LOG_SURF").is_some() {
                if let Some(Obj::Surface(sf)) = uc.get_data().com.objs.get(&s) {
                    eprintln!("[surf] {w}x{h} caps={caps:#x} mem={:#x}", sf.mem);
                }
            }
            wr32(uc, a[2], s);
            0
        }
        ("IDirectDrawSurface", "GetAttachedSurface") => match uc.get_data().com.objs.get(&this) {
            Some(Obj::Surface(Surface { back: Some(b), .. })) => {
                wr32(uc, a[2], *b);
                0
            }
            _ => 0x8876_0000 + 0x16a,
        },
        ("IDirectDrawSurface", "Lock") | ("IDirectDrawSurface", "GetSurfaceDesc") => {
            let Some((w, h, mem, _)) = surf_info(uc, this) else { return Some(0x8876_0000 + 0x91) };
            let desc = if name == "Lock" { a[2] } else { a[1] };
            wr32(uc, desc + 4, 0x1000f | 0x8); // CAPS|HEIGHT|WIDTH|PITCH|PIXELFORMAT, LPSURFACE below
            wr32(uc, desc + 8, h);
            wr32(uc, desc + 12, w);
            wr32(uc, desc + 16, w);
            wr32(uc, desc + 36, mem);
            // DDPIXELFORMAT: 8-bit palettized.
            wr32(uc, desc + 72, 32);
            wr32(uc, desc + 76, 0x20);
            wr32(uc, desc + 84, 8);
            0
        }
        ("IDirectDrawSurface", "Unlock") => 0,
        ("IDirectDrawSurface", "SetPalette") => {
            if let Some(Obj::Surface(s)) = uc.get_data_mut().com.objs.get_mut(&this) {
                s.palette = Some(a[1]);
            }
            0
        }
        ("IDirectDrawSurface", "SetColorKey") => {
            // DDCOLORKEY: low, high; the game only uses single-colour keys.
            let k = if a[2] != 0 { Some(rd32(uc, a[2])) } else { None };
            if let Some(Obj::Surface(s)) = uc.get_data_mut().com.objs.get_mut(&this) {
                s.key = k;
            }
            0
        }
        ("IDirectDrawSurface", "IsLost") | ("IDirectDrawSurface", "Restore") => 0,
        ("IDirectDrawSurface", "Blt") => {
            let (flags, fx) = (a[4], a[5]);
            let Some((dw, dh, dmem, _)) = surf_info(uc, this) else { return Some(0x8876_0000 + 0x91) };
            let drect = rect_of(uc, a[1], dw, dh);
            if flags & 0x400 != 0 {
                // DDBLT_COLORFILL
                let color = rd32(uc, fx + 80) as u8;
                for row in 0..drect.3 {
                    wr_bytes(uc, dmem + (drect.1 + row) * dw + drect.0, &vec![color; drect.2 as usize]);
                }
            } else if a[2] != 0 {
                let Some((sw, sh, _, skey)) = surf_info(uc, a[2]) else { return Some(0x8876_0000 + 0x91) };
                let srect = rect_of(uc, a[3], sw, sh);
                let key = if flags & 0x8000 != 0 { skey } else { None };
                blit(uc, this, (drect.0, drect.1), a[2], srect, key);
            }
            note_present(uc, this);
            0
        }
        ("IDirectDrawSurface", "BltFast") => {
            let Some((sw, sh, _, skey)) = surf_info(uc, a[3]) else { return Some(0x8876_0000 + 0x91) };
            let srect = rect_of(uc, a[4], sw, sh);
            let key = if a[5] & 1 != 0 { skey } else { None };
            blit(uc, this, (a[1], a[2]), a[3], srect, key);
            note_present(uc, this);
            0
        }
        ("IDirectDrawSurface", "Flip") => {
            if let Some(Obj::Surface(Surface { back: Some(b), w, h, .. })) = uc.get_data().com.objs.get(&this) {
                let (b, w, h) = (*b, *w, *h);
                blit(uc, this, (0, 0), b, (0, 0, w, h), None);
            }
            note_present(uc, this);
            0
        }
        ("IDirectDrawSurface", "GetDC") | ("IDirectDrawSurface", "ReleaseDC") => DDERR_UNSUPPORTED,
        ("IDirectDrawPalette", "SetEntries") => {
            let (start, count, src) = (a[2] as usize, a[3] as usize, a[4]);
            let data = rd_bytes(uc, src, count * 4);
            if let Some(Obj::Palette(e)) = uc.get_data_mut().com.objs.get_mut(&this) {
                let n = count.min(256usize.saturating_sub(start));
                e[start * 4..(start + n) * 4].copy_from_slice(&data[..n * 4]);
            }
            0
        }
        ("IDirectDrawPalette", "GetEntries") => 0,
        ("IDirectPlay", m) => return crate::dplay::call_method(uc, m, a, addr),
        ("IDirectSound", "CreateSoundBuffer") => {
            let desc = a[1];
            let flags = rd32(uc, desc + 4);
            let primary = flags & 1 != 0;
            let size = if primary { 0 } else { rd32(uc, desc + 8) };
            let mem = zeroed_alloc(uc, size.max(16));
            let b = new_object(uc, "IDirectSoundBuffer", DSBUFFER, Obj::DsBuffer { mem, size, primary });
            if !primary {
                let mut v = crate::audio::Voice::new(size as usize);
                // The sound loader's hook says which bank the sound being started comes from.
                v.kind = std::mem::take(&mut uc.get_data_mut().sound_kind);
                apply_format(uc, rd32(uc, desc + 16), &mut v);
                uc.get_data().mixer.lock().unwrap().voices.insert(b, v);
            }
            wr32(uc, a[2], b);
            0
        }
        ("IDirectSound", "DuplicateSoundBuffer") => {
            let Some(Obj::DsBuffer { mem, size, primary }) = uc.get_data().com.objs.get(&a[1]).map(|o| match o {
                Obj::DsBuffer { mem, size, primary } => Obj::DsBuffer { mem: *mem, size: *size, primary: *primary },
                _ => Obj::DSound,
            }) else {
                return Some(0x8878_0000 + 0x32);
            };
            let nb = new_object(uc, "IDirectSoundBuffer", DSBUFFER, Obj::DsBuffer { mem, size, primary });
            sync_voice(uc, a[1]);
            let src = uc.get_data().mixer.lock().unwrap().voices.get(&a[1]).cloned();
            if let Some(mut v) = src {
                v.playing = false;
                v.pos = 0.0;
                uc.get_data().mixer.lock().unwrap().voices.insert(nb, v);
            }
            wr32(uc, a[2], nb);
            0
        }
        ("IDirectSound", "GetCaps") => {
            // DSCAPS: report a plain stereo 16-bit card with software buffers.
            let (size, flags) = (rd32(uc, a[1]), 0x2u32 | 0x4 | 0x8 | 0x10 | 0x20);
            wr_bytes(uc, a[1], &vec![0u8; size as usize]);
            wr32(uc, a[1], size);
            wr32(uc, a[1] + 4, flags);
            wr32(uc, a[1] + 8, 8000);
            wr32(uc, a[1] + 12, 48000);
            0
        }
        ("IDirectSound", _) => 0,
        ("IDirectSoundBuffer", "Lock") => {
            let (mem, size) = match uc.get_data().com.objs.get(&this) {
                Some(Obj::DsBuffer { mem, size, .. }) => (*mem, *size),
                _ => return Some(0x8878_0000 + 0x32),
            };
            let (off, mut bytes) = (a[1].min(size), a[2]);
            if a[7] & 2 != 0 {
                bytes = size; // DSBLOCK_ENTIREBUFFER
            }
            let first = bytes.min(size - off);
            if uc.get_data().log_calls {
                eprintln!("[com]   lock mem={mem:#x} size={size:#x} off={off:#x} bytes={bytes:#x} first={first:#x} args={:x?}", &a[..8]);
            }
            wr32(uc, a[3], mem + off);
            wr32(uc, a[4], first);
            if a[5] != 0 {
                // Wrapped part, if any.
                let rest = bytes - first;
                wr32(uc, a[5], if rest > 0 { mem } else { 0 });
                wr32(uc, a[6], rest.min(size));
            }
            0
        }
        ("IDirectSoundBuffer", "Unlock") => {
            // Copy what the guest wrote into the mixer's private copy.
            let mem = match uc.get_data().com.objs.get(&this) {
                Some(Obj::DsBuffer { mem, .. }) => *mem,
                _ => return Some(0),
            };
            for (p, n) in [(a[1], a[2]), (a[3], a[4])] {
                if p != 0 && n != 0 {
                    let bytes = rd_bytes(uc, p, n as usize);
                    let off = (p - mem) as usize;
                    if let Some(v) = uc.get_data().mixer.lock().unwrap().voices.get_mut(&this) {
                        let end = (off + bytes.len()).min(v.data.len());
                        v.data[off..end].copy_from_slice(&bytes[..end - off]);
                    }
                }
            }
            0
        }
        ("IDirectSoundBuffer", "Play") => {
            sync_voice(uc, this);
            if let Some(v) = uc.get_data().mixer.lock().unwrap().voices.get_mut(&this) {
                v.playing = true;
                v.looping = a[3] & 1 != 0;
            }
            0
        }
        ("IDirectSoundBuffer", "Stop") => {
            if let Some(v) = uc.get_data().mixer.lock().unwrap().voices.get_mut(&this) {
                v.playing = false;
            }
            0
        }
        ("IDirectSoundBuffer", "SetCurrentPosition") => {
            if let Some(v) = uc.get_data().mixer.lock().unwrap().voices.get_mut(&this) {
                v.pos = (a[1] as usize / (v.channels as usize * v.bits as usize / 8).max(1)) as f64;
            }
            0
        }
        ("IDirectSoundBuffer", "SetVolume") => {
            if let Some(v) = uc.get_data().mixer.lock().unwrap().voices.get_mut(&this) {
                v.volume = a[1] as i32;
            }
            0
        }
        ("IDirectSoundBuffer", "SetPan") => {
            if let Some(v) = uc.get_data().mixer.lock().unwrap().voices.get_mut(&this) {
                v.pan = a[1] as i32;
            }
            0
        }
        ("IDirectSoundBuffer", "SetFrequency") => {
            if let Some(v) = uc.get_data().mixer.lock().unwrap().voices.get_mut(&this) {
                v.freq = if a[1] == 0 { None } else { Some(a[1]) };
            }
            0
        }
        ("IDirectSoundBuffer", "SetFormat") => {
            if let Some(Obj::DsBuffer { primary: false, .. }) = uc.get_data().com.objs.get(&this) {
                let mut v = uc.get_data().mixer.lock().unwrap().voices.get(&this).cloned();
                if let Some(v) = v.as_mut() {
                    apply_format(uc, a[1], v);
                }
                if let Some(v) = v {
                    uc.get_data().mixer.lock().unwrap().voices.insert(this, v);
                }
            }
            0
        }
        ("IDirectSoundBuffer", "GetStatus") => {
            sync_voice(uc, this);
            let st = match uc.get_data().mixer.lock().unwrap().voices.get(&this) {
                Some(v) if v.playing => 1 | if v.looping { 4 } else { 0 },
                _ => 0,
            };
            wr32(uc, a[1], st);
            0
        }
        ("IDirectSoundBuffer", "GetCurrentPosition") => {
            sync_voice(uc, this);
            let pos = uc.get_data().mixer.lock().unwrap().voices.get(&this).map_or(0, |v| v.pos as u32 * (v.channels as u32 * v.bits as u32 / 8));
            if a[1] != 0 {
                wr32(uc, a[1], pos);
            }
            if a[2] != 0 {
                wr32(uc, a[2], pos);
            }
            0
        }
        ("IDirectSoundBuffer", _) => 0,
        _ => {
            eprintln!("[com] UNIMPLEMENTED {iface}::{name}({:#x}, {:#x}, {:#x}, {:#x})", a[0], a[1], a[2], a[3]);
            0x8876_0000 + 0x0b0
        }
    })
}

/// Copies a secondary buffer's guest memory into the mixer's copy. The game fills its buffers by
/// writing straight into the memory it got from `Lock` (also after `Unlock`), so `Unlock` alone
/// sees only part of the sound and speech was cut short.
fn sync_voice(uc: &mut Uc, buf: u32) {
    let (mem, size) = match uc.get_data().com.objs.get(&buf) {
        Some(Obj::DsBuffer { mem, size, primary: false }) => (*mem, *size),
        _ => return,
    };
    let bytes = rd_bytes(uc, mem, size as usize);
    if let Some(v) = uc.get_data().mixer.lock().unwrap().voices.get_mut(&buf) {
        if v.data.len() == bytes.len() {
            v.data = bytes;
        }
    }
}

/// Reads a WAVEFORMATEX at `fmt` into a voice.
fn apply_format(uc: &Uc, fmt: u32, v: &mut crate::audio::Voice) {
    if fmt == 0 {
        return;
    }
    let b = rd_bytes(uc, fmt, 16);
    v.channels = u16::from_le_bytes([b[2], b[3]]).max(1);
    v.rate = u32::from_le_bytes([b[4], b[5], b[6], b[7]]).max(1);
    v.bits = u16::from_le_bytes([b[14], b[15]]);
    if v.bits != 16 {
        v.bits = 8;
    }
}

fn note_present(uc: &mut Uc, surface: u32) {
    if uc.get_data().com.primary == Some(surface) {
        uc.get_data_mut().com.frames_presented += 1;
    }
}

/// The primary surface as RGBA, if one exists.
pub fn snapshot(uc: &mut Uc) -> Option<(u32, u32, Vec<u8>)> {
    let p = uc.get_data().com.primary?;
    let (w, h, mem, pal) = match uc.get_data().com.objs.get(&p)? {
        Obj::Surface(s) => (s.w, s.h, s.mem, s.palette),
        _ => return None,
    };
    let entries = match pal.and_then(|p| uc.get_data().com.objs.get(&p)) {
        Some(Obj::Palette(e)) => *e,
        _ => [0u8; 1024],
    };
    let px = rd_bytes(uc, mem, (w * h) as usize);
    let mut out = Vec::with_capacity(px.len() * 4);
    for i in px {
        let e = &entries[i as usize * 4..i as usize * 4 + 3];
        out.extend_from_slice(&[e[0], e[1], e[2], 255]);
    }
    Some((w, h, out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::testing::com_uc;

    /// Calls `name` on COM object `obj` through its vtable stub, the way the code hook would.
    fn call(uc: &mut Uc, obj: u32, name: &str, args: &[u32]) -> u32 {
        let vt = rd32(uc, obj);
        let stub = (0..70).map(|i| rd32(uc, vt + i * 4)).find(|s| uc.get_data().com.methods.get(s).is_some_and(|m| m.1 == name)).unwrap_or_else(|| panic!("no method {name}"));
        let mut a = [0u32; 14];
        a[0] = obj;
        a[1..1 + args.len()].copy_from_slice(args);
        call_method(uc, stub, &a).unwrap()
    }

    fn scratch(uc: &mut Uc, n: u32) -> u32 {
        zeroed_alloc(uc, n)
    }

    fn ddraw(uc: &mut Uc) -> u32 {
        let out = scratch(uc, 4);
        assert_eq!(direct_draw_create(uc, out), 0);
        rd32(uc, out)
    }

    /// DDSURFACEDESC with the given flags, size and caps.
    fn desc(uc: &mut Uc, flags: u32, w: u32, h: u32, caps: u32) -> u32 {
        let d = scratch(uc, 108);
        wr32(uc, d + 4, flags);
        wr32(uc, d + 8, h);
        wr32(uc, d + 12, w);
        wr32(uc, d + 104, caps);
        d
    }

    fn surface(uc: &mut Uc, dd: u32, w: u32, h: u32) -> u32 {
        let (d, out) = (desc(uc, 7, w, h, 0), scratch(uc, 4));
        assert_eq!(call(uc, dd, "CreateSurface", &[d, out, 0]), 0);
        rd32(uc, out)
    }

    fn mem_of(uc: &Uc, s: u32) -> u32 {
        match &uc.get_data().com.objs[&s] {
            Obj::Surface(s) => s.mem,
            _ => panic!("not a surface"),
        }
    }

    #[test]
    fn vtables_are_built_once_and_stubs_pop_their_arguments() {
        let mut uc = com_uc();
        let (a, b) = (ddraw(&mut uc), ddraw(&mut uc));
        assert_ne!(a, b);
        assert_eq!(rd32(&uc, a), rd32(&uc, b), "objects of one interface share a vtable");
        let stub = rd32(&uc, rd32(&uc, a) + 3 * 4);
        assert_eq!(uc.get_data().com.methods[&stub], ("IDirectDraw", "Compact"));
        assert_eq!(rd_bytes(&uc, stub, 3), [0xc2, 4, 0], "ret 4: Compact takes only `this`");
        let blt = uc.get_data().com.methods.iter().find(|(_, m)| m.1 == "Blt").map(|(s, _)| *s);
        assert!(blt.is_none(), "surface vtable not built yet");
        assert!(call_method(&mut uc, 0x1234, &[0; 14]).is_none(), "unknown stub address");
    }

    #[test]
    fn refcounts_and_unknown_interfaces() {
        let mut uc = com_uc();
        let dd = ddraw(&mut uc);
        assert_eq!(call(&mut uc, dd, "AddRef", &[]), 2);
        assert_eq!(call(&mut uc, dd, "Release", &[]), 1);
        assert_eq!(call(&mut uc, dd, "Release", &[]), 0);
        assert_eq!(call(&mut uc, dd, "Release", &[]), 0, "never goes negative");
        assert_eq!(call(&mut uc, dd, "QueryInterface", &[0, 0]), 0x8000_4002);
    }

    #[test]
    fn display_mode_is_reported_back() {
        let mut uc = com_uc();
        let dd = ddraw(&mut uc);
        assert_eq!(call(&mut uc, dd, "SetCooperativeLevel", &[0, 0]), 0);
        assert_eq!(call(&mut uc, dd, "SetDisplayMode", &[800, 600, 8]), 0);
        let d = scratch(&mut uc, 108);
        assert_eq!(call(&mut uc, dd, "GetDisplayMode", &[d]), 0);
        assert_eq!((rd32(&uc, d + 12), rd32(&uc, d + 8), rd32(&uc, d + 84)), (800, 600, 8));
        assert_eq!(uc.get_data().com.display, (800, 600));
    }

    #[test]
    fn primary_surface_with_back_buffer_flip_and_snapshot() {
        let mut uc = com_uc();
        let dd = ddraw(&mut uc);
        call(&mut uc, dd, "SetDisplayMode", &[4, 2, 8]);
        // Palette: entry 5 = (10, 20, 30).
        let (ents, pal_out) = (scratch(&mut uc, 1024), scratch(&mut uc, 4));
        wr_bytes(&mut uc, ents + 20, &[10, 20, 30, 0]);
        assert_eq!(call(&mut uc, dd, "CreatePalette", &[0, ents, pal_out]), 0);
        let pal = rd32(&uc, pal_out);
        // Primary (caps 0x200) with a back buffer (flags 0x20).
        let (d, out) = (desc(&mut uc, 0x20, 0, 0, 0x200), scratch(&mut uc, 4));
        assert_eq!(call(&mut uc, dd, "CreateSurface", &[d, out, 0]), 0);
        let prim = rd32(&uc, out);
        assert_eq!(uc.get_data().com.primary, Some(prim));
        assert!(snapshot(&mut uc).is_some_and(|(w, h, _)| (w, h) == (4, 2)));
        let back_out = scratch(&mut uc, 4);
        assert_eq!(call(&mut uc, prim, "GetAttachedSurface", &[0, back_out]), 0);
        let back = rd32(&uc, back_out);
        assert_eq!(call(&mut uc, back, "GetAttachedSurface", &[0, back_out]), 0x8876_016a);
        assert_eq!(call(&mut uc, prim, "SetPalette", &[pal]), 0);
        // Fill the back buffer with colour 5 (DDBLT_COLORFILL = 0x400; colour at fx+80) and flip.
        let fx = scratch(&mut uc, 100);
        wr32(&mut uc, fx + 80, 5);
        assert_eq!(call(&mut uc, back, "Blt", &[0, 0, 0, 0x400, fx]), 0);
        assert_eq!(call(&mut uc, prim, "Flip", &[0, 0]), 0);
        assert_eq!(uc.get_data().com.frames_presented, 1);
        let (w, h, rgba) = snapshot(&mut uc).unwrap();
        assert_eq!((w, h, rgba.len()), (4, 2, 32));
        assert_eq!(&rgba[..4], &[10, 20, 30, 255]);
        assert!(call(&mut uc, prim, "IsLost", &[]) == 0 && call(&mut uc, prim, "Restore", &[]) == 0);
        assert_eq!(call(&mut uc, prim, "GetDC", &[0]), DDERR_UNSUPPORTED);
    }

    #[test]
    fn blt_honours_rects_and_colour_keys() {
        let mut uc = com_uc();
        let dd = ddraw(&mut uc);
        let (src, dst) = (surface(&mut uc, dd, 4, 4), surface(&mut uc, dd, 8, 8));
        let (sm, dm) = (mem_of(&uc, src), mem_of(&uc, dst));
        wr_bytes(&mut uc, dm, &[9; 64]);
        // Source 4x4: row r has value r+1, except column 0 which is the key colour 0.
        for r in 0..4u32 {
            wr_bytes(&mut uc, sm + r * 4, &[0, r as u8 + 1, r as u8 + 1, r as u8 + 1]);
        }
        // Plain copy of source rect (1,0)-(3,2) to dest (2,3) with BltFast.
        let rect = scratch(&mut uc, 16);
        for (i, v) in [1u32, 0, 3, 2].iter().enumerate() {
            wr32(&mut uc, rect + i as u32 * 4, *v);
        }
        assert_eq!(call(&mut uc, dst, "BltFast", &[2, 3, src, rect, 0]), 0);
        assert_eq!(rd_bytes(&uc, dm + 3 * 8 + 2, 3), [1, 1, 9]);
        assert_eq!(rd_bytes(&uc, dm + 4 * 8 + 2, 2), [2, 2]);
        // Colour-keyed copy leaves destination pixels where the source equals the key.
        let keyp = scratch(&mut uc, 8);
        wr32(&mut uc, keyp, 0);
        assert_eq!(call(&mut uc, src, "SetColorKey", &[0x8, keyp]), 0);
        assert_eq!(call(&mut uc, dst, "BltFast", &[0, 0, src, 0, 1]), 0);
        assert_eq!(rd_bytes(&uc, dm, 4), [9, 1, 1, 1], "the key pixel is skipped");
        // Blt with DDBLT_KEYSRC (0x8000) behaves the same; clipped at the destination edge.
        assert_eq!(call(&mut uc, dst, "Blt", &[0, src, 0, 0x8000, 0]), 0);
        assert_eq!(rd_bytes(&uc, dm, 4), [9, 1, 1, 1]);
        assert_eq!(call(&mut uc, dst, "BltFast", &[6, 6, src, 0, 0]), 0, "overhang is clipped, not an error");
        // Blt from a missing source is an error; colour-fill on a rect.
        assert_eq!(call(&mut uc, dst, "Blt", &[0, 0xdead, 0, 0, 0]), 0x8876_0091);
        wr32(&mut uc, rect, 6);
        wr32(&mut uc, rect + 4, 6);
        wr32(&mut uc, rect + 8, 100);
        wr32(&mut uc, rect + 12, 100);
        let fx = scratch(&mut uc, 100);
        wr32(&mut uc, fx + 80, 0x77);
        call(&mut uc, dst, "Blt", &[rect, 0, 0, 0x400, fx]);
        assert_eq!(rd_bytes(&uc, dm + 7 * 8 + 6, 2), [0x77, 0x77]);
        assert_eq!(rd_bytes(&uc, dm + 5 * 8 + 6, 2), [9, 9], "row above the rect untouched");
    }

    #[test]
    fn lock_and_get_surface_desc_expose_the_pixel_memory() {
        let mut uc = com_uc();
        let dd = ddraw(&mut uc);
        let s = surface(&mut uc, dd, 16, 8);
        let d = scratch(&mut uc, 108);
        assert_eq!(call(&mut uc, s, "Lock", &[0, d, 0, 0]), 0);
        assert_eq!((rd32(&uc, d + 12), rd32(&uc, d + 8), rd32(&uc, d + 16)), (16, 8, 16));
        assert_eq!(rd32(&uc, d + 36), mem_of(&uc, s));
        assert_eq!(call(&mut uc, s, "Unlock", &[0]), 0);
        let d2 = scratch(&mut uc, 108);
        assert_eq!(call(&mut uc, s, "GetSurfaceDesc", &[d2]), 0);
        assert_eq!(rd32(&uc, d2 + 84), 8);
        let pal_out = scratch(&mut uc, 4);
        assert_eq!(call(&mut uc, dd, "CreatePalette", &[0, 0, pal_out]), 0);
        let pal = rd32(&uc, pal_out);
        // SetEntries(start=250, count=10): only 6 fit.
        let src = scratch(&mut uc, 40);
        wr_bytes(&mut uc, src, &[1; 40]);
        assert_eq!(call(&mut uc, pal, "SetEntries", &[0, 250, 10, src]), 0);
        match &uc.get_data().com.objs[&pal] {
            Obj::Palette(e) => assert_eq!((e[250 * 4], e[255 * 4], e[0]), (1, 1, 0)),
            _ => panic!(),
        }
        assert_eq!(call(&mut uc, s, "GetDC", &[0]), DDERR_UNSUPPORTED);
        // Unknown object handles give DDERR_INVALIDOBJECT-style errors; unimplemented methods give a code.
        assert_eq!(call(&mut uc, dd, "Compact", &[]), 0x8876_0000 + 0x0b0);
    }

    #[test]
    fn surface_lookup_failures() {
        let mut uc = com_uc();
        let dd = ddraw(&mut uc);
        let s = surface(&mut uc, dd, 4, 4);
        uc.get_data_mut().com.objs.remove(&s);
        let d = scratch(&mut uc, 108);
        assert_eq!(call(&mut uc, s, "Lock", &[0, d, 0, 0]), 0x8876_0091);
    }

    fn wave_format(uc: &mut Uc, channels: u16, rate: u32, bits: u16) -> u32 {
        let f = scratch(uc, 16);
        wr_bytes(uc, f + 2, &channels.to_le_bytes());
        wr32(uc, f + 4, rate);
        wr_bytes(uc, f + 14, &bits.to_le_bytes());
        f
    }

    fn dsound_buffer(uc: &mut Uc, ds: u32, size: u32, fmt: u32) -> u32 {
        let (d, out) = (scratch(uc, 32), scratch(uc, 4));
        wr32(uc, d + 8, size);
        wr32(uc, d + 16, fmt);
        assert_eq!(call(uc, ds, "CreateSoundBuffer", &[d, out, 0]), 0);
        rd32(uc, out)
    }

    fn dsound(uc: &mut Uc) -> u32 {
        let out = scratch(uc, 4);
        assert_eq!(direct_sound_create(uc, out), 0);
        rd32(uc, out)
    }

    #[test]
    fn new_buffers_take_the_kind_of_the_sound_being_loaded_and_duplicates_keep_it() {
        use crate::audio::Kind;
        let mut uc = com_uc();
        let ds = dsound(&mut uc);
        let fmt = wave_format(&mut uc, 1, 22050, 8);
        let plain = dsound_buffer(&mut uc, ds, 16, fmt);
        uc.get_data_mut().sound_kind = Kind::Building;
        let hammer = dsound_buffer(&mut uc, ds, 16, fmt);
        assert_eq!(uc.get_data().sound_kind, Kind::Effect, "the hook's kind is used up by one buffer");
        let again = dsound_buffer(&mut uc, ds, 16, fmt);
        let out = scratch(&mut uc, 4);
        assert_eq!(call(&mut uc, ds, "DuplicateSoundBuffer", &[hammer, out]), 0);
        let dup = rd32(&uc, out);
        let m = uc.get_data().mixer.lock().unwrap();
        assert_eq!((m.voices[&plain].kind, m.voices[&hammer].kind, m.voices[&again].kind, m.voices[&dup].kind), (Kind::Effect, Kind::Building, Kind::Effect, Kind::Building));
    }

    #[test]
    fn sound_buffers_lock_unlock_and_play_through_the_mixer() {
        let mut uc = com_uc();
        let ds = dsound(&mut uc);
        let fmt = wave_format(&mut uc, 2, 44100, 16);
        let buf = dsound_buffer(&mut uc, ds, 64, fmt);
        {
            let m = uc.get_data().mixer.lock().unwrap();
            let v = &m.voices[&buf];
            assert_eq!((v.channels, v.rate, v.bits, v.data.len()), (2, 44100, 16, 64));
        }
        // Lock the whole buffer (flag 2): wrap pointers are zero.
        let (p1, n1, p2, n2) = (scratch(&mut uc, 4), scratch(&mut uc, 4), scratch(&mut uc, 4), scratch(&mut uc, 4));
        assert_eq!(call(&mut uc, buf, "Lock", &[0, 0, p1, n1, p2, n2, 2]), 0);
        let (mem, n) = (rd32(&uc, p1), rd32(&uc, n1));
        assert_eq!(n, 64);
        assert_eq!((rd32(&uc, p2), rd32(&uc, n2)), (0, 0));
        wr_bytes(&mut uc, mem, &[1, 2, 3, 4]);
        assert_eq!(call(&mut uc, buf, "Unlock", &[mem, 4, 0, 0]), 0);
        assert_eq!(&uc.get_data().mixer.lock().unwrap().voices[&buf].data[..4], [1, 2, 3, 4]);
        // Wrapping lock: 16 bytes starting 8 before the end.
        assert_eq!(call(&mut uc, buf, "Lock", &[56, 16, p1, n1, p2, n2, 0]), 0);
        assert_eq!((rd32(&uc, n1), rd32(&uc, n2)), (8, 8));
        assert_eq!(rd32(&uc, p2), mem);
        // Playing, looping flag, status, position, volume/pan/frequency.
        assert_eq!(call(&mut uc, buf, "Play", &[0, 0, 1]), 0);
        let st = scratch(&mut uc, 4);
        call(&mut uc, buf, "GetStatus", &[st]);
        assert_eq!(rd32(&uc, st), 1 | 4);
        call(&mut uc, buf, "SetVolume", &[(-600i32) as u32]);
        call(&mut uc, buf, "SetPan", &[(-100i32) as u32]);
        call(&mut uc, buf, "SetFrequency", &[11025]);
        call(&mut uc, buf, "SetCurrentPosition", &[16]);
        {
            let m = uc.get_data().mixer.lock().unwrap();
            let v = &m.voices[&buf];
            assert_eq!((v.volume, v.pan, v.freq, v.pos), (-600, -100, Some(11025), 4.0));
        }
        call(&mut uc, buf, "SetFrequency", &[0]);
        assert_eq!(uc.get_data().mixer.lock().unwrap().voices[&buf].freq, None);
        let (cur, wr) = (scratch(&mut uc, 4), scratch(&mut uc, 4));
        call(&mut uc, buf, "GetCurrentPosition", &[cur, wr]);
        assert_eq!((rd32(&uc, cur), rd32(&uc, wr)), (16, 16));
        assert_eq!(call(&mut uc, buf, "Stop", &[]), 0);
        call(&mut uc, buf, "GetStatus", &[st]);
        assert_eq!(rd32(&uc, st), 0);
    }

    #[test]
    fn duplicate_buffers_copy_the_sound_but_not_the_playback_state() {
        let mut uc = com_uc();
        let ds = dsound(&mut uc);
        let fmt = wave_format(&mut uc, 1, 22050, 8);
        let buf = dsound_buffer(&mut uc, ds, 32, fmt);
        let (p1, n1) = (scratch(&mut uc, 4), scratch(&mut uc, 4));
        call(&mut uc, buf, "Lock", &[0, 32, p1, n1, 0, 0, 0]);
        let mem = rd32(&uc, p1);
        wr_bytes(&mut uc, mem, &[7; 32]);
        call(&mut uc, buf, "Play", &[0, 0, 0]);
        let out = scratch(&mut uc, 4);
        assert_eq!(call(&mut uc, ds, "DuplicateSoundBuffer", &[buf, out]), 0);
        let dup = rd32(&uc, out);
        assert_ne!(dup, buf);
        {
            let m = uc.get_data().mixer.lock().unwrap();
            assert_eq!(m.voices[&dup].data, vec![7; 32]);
            assert!(!m.voices[&dup].playing);
            assert!(m.voices[&buf].playing);
        }
        assert_eq!(call(&mut uc, ds, "DuplicateSoundBuffer", &[0xbad, out]), 0x8878_0032);
        // Releasing the last reference removes the voice from the mixer.
        assert_eq!(call(&mut uc, dup, "Release", &[]), 0);
        assert!(!uc.get_data().mixer.lock().unwrap().voices.contains_key(&dup));
        assert_eq!(call(&mut uc, ds, "SetCooperativeLevel", &[0, 2]), 0);
    }

    #[test]
    fn primary_buffer_has_no_voice_and_format_changes_only_touch_secondary_ones() {
        let mut uc = com_uc();
        let ds = dsound(&mut uc);
        let (d, out) = (scratch(&mut uc, 32), scratch(&mut uc, 4));
        wr32(&mut uc, d + 4, 1); // DSBCAPS_PRIMARYBUFFER
        assert_eq!(call(&mut uc, ds, "CreateSoundBuffer", &[d, out, 0]), 0);
        let prim = rd32(&uc, out);
        assert!(uc.get_data().mixer.lock().unwrap().voices.is_empty());
        let fmt = wave_format(&mut uc, 2, 44100, 16);
        assert_eq!(call(&mut uc, prim, "SetFormat", &[fmt]), 0);
        assert_eq!(call(&mut uc, prim, "Play", &[0, 0, 1]), 0);
        let sec = dsound_buffer(&mut uc, ds, 16, 0);
        let fmt = wave_format(&mut uc, 2, 11025, 12);
        call(&mut uc, sec, "SetFormat", &[fmt]);
        let m = uc.get_data().mixer.lock().unwrap();
        let v = &m.voices[&sec];
        assert_eq!((v.channels, v.rate, v.bits), (2, 11025, 8), "anything but 16 bits is treated as 8");
        drop(m);
        let caps = scratch(&mut uc, 96);
        wr32(&mut uc, caps, 96);
        assert_eq!(call(&mut uc, ds, "GetCaps", &[caps]), 0);
        assert_eq!((rd32(&uc, caps), rd32(&uc, caps + 8), rd32(&uc, caps + 12)), (96, 8000, 48000));
    }
}
