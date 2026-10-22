//! Debug console: commands that poke the running game's memory (stockpile, cheats, ...).
//! Opened with F12 in the window, or from scripts with `dbg <command>`.

use crate::machine::{rd32, rd_bytes, reg, set_reg, wr32, wr_bytes, Uc};
use unicorn_engine::RegisterX86 as R;

/// Per-player resource table: 8-byte entries indexed `player * 4 + kind`, current amount at +2 (u16).
const RES_TABLE: u32 = 0x4e2ea8;
const RES_NAMES: [&str; 4] = ["metal", "comida", "pedra", "extra"];

fn res_addr(kind: u32, player: u32) -> u32 {
    RES_TABLE + (player * 4 + kind) * 8 + 2
}

fn kind_of(s: &str) -> Option<u32> {
    match s {
        "0" | "metal" | "scrap" | "restos" => Some(0),
        "1" | "food" | "comida" => Some(1),
        "2" | "stone" | "pedra" | "pedras" => Some(2),
        "3" | "extra" => Some(3),
        _ => None,
    }
}

fn set_u16(uc: &mut Uc, addr: u32, v: u16) {
    wr_bytes(uc, addr, &v.to_le_bytes());
}

pub const HELP: &str = "res <metal|comida|pedra|all> <n> [jogador] | tech [area n [jogador]] | instant [on|off] | peek <addr> [n] | poke <addr> <valor> [1|2|4] | snap | diff | help";

/// Memory snapshot taken by `snap`, for `diff` (region start, bytes).
static SNAP: std::sync::Mutex<Vec<(u64, Vec<u8>)>> = std::sync::Mutex::new(Vec::new());

fn hex(s: &str) -> Option<u32> {
    u32::from_str_radix(s.trim_start_matches("0x"), 16).ok()
}

/// Writable guest regions worth diffing (the game's data and its heap, not the stack or the screen).
fn data_regions(uc: &Uc) -> Vec<(u64, usize)> {
    uc.mem_regions().unwrap_or_default().iter().filter(|r| r.perms & 2 != 0 && r.end - r.begin < 0x400_0000).map(|r| (r.begin, (r.end - r.begin + 1) as usize)).collect()
}

/// Slots of the building table (0x114 bytes each, 1000 of them). Byte +5 is the state (2 = under
/// construction, 5 = finished), byte +8 the building type and, while under construction, the u16
/// at +0xa counts progress up to the type's build time (u16 at `BLD_TIME + type * 0x34`).
const BLD_TABLE: u32 = 0x49a0c8;
const BLD_TIME: u32 = 0x495d90;
const BLD_SLOTS: u32 = 1000;
const BLD_SIZE: u32 = 0x114;

/// Where a called guest function returns to: inside the PE header, never executed.
const CALL_TRAP: u32 = 0x40_0010;

/// Runs a guest function (Watcom register convention: EAX, EDX, EBX, ECX, then the stack) to its
/// return and gives back EAX, leaving the interrupted guest state untouched. Only call this while
/// the CPU is stopped, e.g. from the window's event loop or a script step.
pub fn call_guest(uc: &mut Uc, target: u32, regs: [u32; 4], stack: &[u32]) -> Result<u32, String> {
    let mut ctx = uc.context_alloc().map_err(|e| format!("{e:?}"))?;
    uc.context_save(&mut ctx).map_err(|e| format!("{e:?}"))?;
    let saved = {
        let d = uc.get_data();
        (d.skip_once, d.last_hooked)
    };
    let mut esp = reg(uc, R::ESP) - 0x100;
    for a in stack.iter().rev() {
        esp -= 4;
        wr32(uc, esp, *a);
    }
    esp -= 4;
    wr32(uc, esp, CALL_TRAP);
    for (r, v) in [R::EAX, R::EDX, R::EBX, R::ECX].into_iter().zip(regs) {
        set_reg(uc, r, v);
    }
    set_reg(uc, R::ESP, esp);
    let res = uc.emu_start(target as u64, CALL_TRAP as u64, 0, 0);
    let eax = reg(uc, R::EAX);
    let stopped_early = uc.get_data_mut().pending.take().is_some();
    let _ = uc.context_restore(&ctx);
    let d = uc.get_data_mut();
    (d.skip_once, d.last_hooked) = saved;
    match res {
        Ok(()) if !stopped_early => Ok(eax),
        Ok(()) => Err("a funcao do jogo pediu para ceder o controle".into()),
        Err(e) => Err(format!("falha no jogo: {e:?}")),
    }
}

/// Techs: research items per area (0..4), 30 per area. `TECH_DONE` holds, per player (16 bytes) and
/// area, how many items are done; `TECH_TOTAL` is the per-player grand total.
const TECH_DONE: u32 = 0x5080f4;
const TECH_TOTAL: u32 = 0x50871c;
const AREAS: [&str; 4] = ["agricultura", "militar", "civil", "religiao"];

/// Called once per frame: applies the toggles that need to keep acting.
pub fn tick(uc: &mut Uc) {
    if uc.get_data().instant_build {
        let mem = rd_bytes(uc, BLD_TABLE, (BLD_SLOTS * BLD_SIZE) as usize);
        for i in 0..BLD_SLOTS {
            let o = (i * BLD_SIZE) as usize;
            if mem[o + 5] == 2 {
                let max = u16::from_le_bytes(rd_bytes(uc, BLD_TIME + mem[o + 8] as u32 * 0x34, 2).try_into().unwrap());
                if u16::from_le_bytes([mem[o + 0xa], mem[o + 0xb]]) <= max {
                    // One step past the end: the builder's next tick sees it as done.
                    set_u16(uc, BLD_TABLE + i * BLD_SIZE + 0xa, max + 4);
                }
            }
        }
    }
}

/// Runs one console command; the returned text is shown to the user.
pub fn run(uc: &mut Uc, line: &str) -> String {
    let w: Vec<&str> = line.split_whitespace().collect();
    match w.as_slice() {
        ["res", what, n, rest @ ..] => {
            let Ok(n) = n.parse::<u16>() else { return "QUANTIDADE INVALIDA".into() };
            let player: u32 = rest.first().and_then(|p| p.parse().ok()).unwrap_or(0);
            let kinds: Vec<u32> = if *what == "all" { (0..4).collect() } else { kind_of(what).into_iter().collect() };
            if kinds.is_empty() {
                return format!("RECURSO DESCONHECIDO: {what}");
            }
            for k in &kinds {
                set_u16(uc, res_addr(*k, player), n.min(9999));
            }
            format!("{} = {n}", if *what == "all" { "TODOS" } else { what })
        }
        ["res"] => {
            let v: Vec<String> = (0..4).map(|k| format!("{} {}", RES_NAMES[k as usize], u16::from_le_bytes(rd_bytes(uc, res_addr(k, 0), 2).try_into().unwrap()))).collect();
            let _ = rd32;
            v.join(", ")
        }
        ["tech"] => {
            let t = |uc: &Uc, a| rd_bytes(uc, TECH_DONE + a, 1)[0];
            (0..4).map(|a| format!("{} {}", AREAS[a as usize], t(uc, a))).collect::<Vec<_>>().join(", ")
        }
        ["tech", area, n, rest @ ..] => {
            let Some(a) = AREAS.iter().position(|x| x == area).or_else(|| area.parse().ok().filter(|a| *a < 4)) else { return format!("AREA: {}", AREAS.join("|")) };
            let (Ok(n), player) = (n.parse::<u32>(), rest.first().and_then(|p| p.parse::<u32>().ok()).unwrap_or(0)) else { return "QUANTIDADE INVALIDA".into() };
            let n = n.min(30);
            let slot = TECH_DONE + player * 16 + a as u32;
            let cur = rd_bytes(uc, slot, 1)[0] as u32;
            for idx in cur..n {
                if let Err(e) = call_guest(uc, 0x470f98, [player, idx, a as u32, 1], &[]) {
                    return e;
                }
            }
            if n > cur {
                wr_bytes(uc, slot, &[n as u8]);
                let tot = rd_bytes(uc, TECH_TOTAL + player, 1)[0] as u32 + (n - cur);
                wr_bytes(uc, TECH_TOTAL + player, &[tot.min(255) as u8]);
            }
            format!("{}: {n} DESCOBERTAS", AREAS[a])
        }
        // Lists the entity table: slot, owner (+4), state (+5), type (+8), tile (+2: y*256+x).
        ["ents"] => {
            let mem = rd_bytes(uc, BLD_TABLE, (BLD_SLOTS * BLD_SIZE) as usize);
            let mut n = 0;
            let mut first_free = None;
            for i in 0..BLD_SLOTS as usize {
                let o = i * BLD_SIZE as usize;
                if mem[o + 5] == 0 {
                    first_free.get_or_insert(i);
                    continue;
                }
                n += 1;
                let tile = u16::from_le_bytes([mem[o + 2], mem[o + 3]]);
                eprintln!("[ent] {i} owner={} state={} type={} tile=({},{})", mem[o + 4], mem[o + 5], mem[o + 8], tile & 0xff, tile >> 8);
            }
            format!("{n} ENTIDADES, PRIMEIRO LIVRE: {first_free:?}")
        }
        ["instant", rest @ ..] => {
            let on = !matches!(rest.first(), Some(&"off"));
            uc.get_data_mut().instant_build = on;
            format!("CONSTRUCAO INSTANTANEA {}", if on { "LIGADA" } else { "DESLIGADA" })
        }
        ["peek", a, rest @ ..] => {
            let (Some(a), n) = (hex(a), rest.first().and_then(|n| n.parse::<usize>().ok()).unwrap_or(16)) else { return "ENDERECO INVALIDO".into() };
            let b = rd_bytes(uc, a, n.min(512));
            eprintln!("[peek {a:#x}] {b:02x?}");
            b.iter().map(|x| format!("{x:02x}")).collect::<Vec<_>>().join(" ")
        }
        ["poke", a, v, rest @ ..] => {
            let (Some(a), Ok(v)) = (hex(a), v.parse::<u32>()) else { return "ARGUMENTO INVALIDO".into() };
            let w: usize = rest.first().and_then(|n| n.parse().ok()).unwrap_or(2);
            wr_bytes(uc, a, &v.to_le_bytes()[..w.clamp(1, 4)]);
            format!("{a:#x} = {v}")
        }
        ["snap"] => {
            let regs = data_regions(uc);
            let snap: Vec<_> = regs.iter().filter_map(|&(b, n)| uc.mem_read_as_vec(b, n).ok().map(|d| (b, d))).collect();
            let total: usize = snap.iter().map(|s| s.1.len()).sum();
            *SNAP.lock().unwrap() = snap;
            format!("SNAP: {} KB", total / 1024)
        }
        // `diff up`: only u16 words that grew by 1..=200; the snapshot is renewed so repeated calls show steady growth.
        ["diff", "up"] => {
            let mut snap = SNAP.lock().unwrap();
            let mut n = 0;
            for (b, old) in snap.iter_mut() {
                let Ok(new) = uc.mem_read_as_vec(*b, old.len()) else { continue };
                for i in (0..old.len().saturating_sub(1)).step_by(2) {
                    let (o, v) = (u16::from_le_bytes([old[i], old[i + 1]]), u16::from_le_bytes([new[i], new[i + 1]]));
                    if v > o && v - o <= 200 && *b < 0xf00_0000 {
                        n += 1;
                        eprintln!("[up] {:#x} {o} -> {v}", *b + i as u64);
                    }
                }
                *old = new;
            }
            format!("UP: {n} PALAVRAS")
        }
        ["diff"] => {
            let snap = SNAP.lock().unwrap();
            let mut n = 0;
            for (b, old) in snap.iter() {
                let Ok(new) = uc.mem_read_as_vec(*b, old.len()) else { continue };
                let mut i = 0;
                while i + 2 <= old.len() {
                    if old[i..i + 2] != new[i..i + 2] {
                        n += 1;
                        if n <= 3000 && *b < 0xf00_0000 {
                            eprintln!("[diff] {:#x}: {:02x?} -> {:02x?}", b + i as u64, &old[i..(i + 4).min(old.len())], &new[i..(i + 4).min(new.len())]);
                        }
                    }
                    i += 2;
                }
            }
            format!("DIFF: {n} PALAVRAS")
        }
        [] => String::new(),
        _ => format!("COMANDOS: {HELP}"),
    }
}
