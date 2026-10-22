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
    pub display: (u32, u32),
    pub primary: Option<u32>,
    pub frames_presented: u64,
    pub log: bool,
}

pub enum Obj {
    DDraw,
    Surface(Surface),
    Palette([u8; 1024]),
    DSound,
    DsBuffer { mem: u32, size: u32 },
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

const DSOUND: &[Method] = &[
    ("QueryInterface", 3), ("AddRef", 1), ("Release", 1), ("CreateSoundBuffer", 4), ("GetCaps", 2),
    ("DuplicateSoundBuffer", 3), ("SetCooperativeLevel", 3), ("Compact", 1), ("GetSpeakerConfig", 2),
    ("SetSpeakerConfig", 2), ("Initialize", 2),
];

const DSBUFFER: &[Method] = &[
    ("QueryInterface", 3), ("AddRef", 1), ("Release", 1), ("GetCaps", 2), ("GetCurrentPosition", 3), ("GetFormat", 4),
    ("GetVolume", 2), ("GetPan", 2), ("GetFrequency", 2), ("GetStatus", 2), ("Initialize", 3), ("Lock", 9), ("Play", 4),
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
    p
}

pub fn direct_draw_create(uc: &mut Uc, out: u32) -> u32 {
    let o = new_object(uc, "IDirectDraw", DDRAW, Obj::DDraw);
    wr32(uc, out, o);
    0
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
        (_, "AddRef") | (_, "Release") => 1,
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
        ("IDirectSound", "CreateSoundBuffer") => {
            let desc = a[1];
            let flags = rd32(uc, desc + 4);
            let size = if flags & 1 != 0 { 0 } else { rd32(uc, desc + 8) };
            let mem = uc.get_data_mut().alloc(size.max(16));
            let b = new_object(uc, "IDirectSoundBuffer", DSBUFFER, Obj::DsBuffer { mem, size });
            wr32(uc, a[2], b);
            0
        }
        ("IDirectSound", _) => 0,
        ("IDirectSoundBuffer", "Lock") => {
            let (mem, size) = match uc.get_data().com.objs.get(&this) {
                Some(Obj::DsBuffer { mem, size }) => (*mem, *size),
                _ => return Some(0x8878_0000 + 0x32),
            };
            let (off, bytes) = (a[1].min(size), a[2].min(size));
            wr32(uc, a[3], mem + off);
            wr32(uc, a[4], bytes);
            if a[5] != 0 {
                wr32(uc, a[5], 0);
            }
            if a[6] != 0 {
                wr32(uc, a[6], 0);
            }
            0
        }
        ("IDirectSoundBuffer", "GetStatus") => {
            wr32(uc, a[1], 0);
            0
        }
        ("IDirectSoundBuffer", "GetCurrentPosition") => {
            if a[1] != 0 {
                wr32(uc, a[1], 0);
            }
            if a[2] != 0 {
                wr32(uc, a[2], 0);
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
