//! Debug console: commands that poke the running game's memory (stockpile, cheats, ...).
//! Opened with F12 in the window, or from scripts with `dbg <command>`.

use crate::machine::{rd32, rd_bytes, reg, set_reg, wr32, wr_bytes, Uc};
use unicorn_engine::RegisterX86 as R;

/// Per-player resource table: 8-byte entries indexed `player * 4 + kind`, current amount at +2 (u16).
const RES_TABLE: u32 = 0x4e2ea8;
/// Pointer to the player records.
const PLAYERS: u32 = 0x5080a8;
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

pub const HELP: &str = "res <metal|comida|pedra|all> <n> [jogador] | tech [area n [jogador]] | instant [on|off] | spawn <tipo> [x y] [jogador] | tab <addr> <tam> <n> [deref] | peek <addr> [n] | poke <addr> <valor> [1|2|4] | clas | att <a> <b> [valor] | ally <a> <b> [0|1] | diplo <proponente> <ia> | aiexec <jogador> [n] | grudge <vitima> <ofensor> <evento> | steal <unidade> <predio> <0-3> | snap | diff | help";

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
    let eip = reg(uc, R::EIP);
    let stopped_early = uc.get_data_mut().pending.take().is_some();
    let _ = uc.context_restore(&ctx);
    let d = uc.get_data_mut();
    (d.skip_once, d.last_hooked) = saved;
    match res {
        Ok(()) if !stopped_early => Ok(eax),
        Ok(()) => Err("a funcao do jogo pediu para ceder o controle".into()),
        Err(e) => Err(format!("falha no jogo em {eip:#x}: {e:?}")),
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
        // `spawn <type 0-25> [x y] [player]`: create a unit the way a finished training does.
        ["spawn", args @ ..] => {
            let n: Vec<u32> = args.iter().filter_map(|a| a.parse().ok()).collect();
            let Some(&ty) = n.first().filter(|&&t| t < 26) else { return "USO: spawn <tipo 0-25> [x y] [jogador]".into() };
            let player = n.get(3).copied().unwrap_or(0);
            let (x, y) = if n.len() >= 3 {
                (n[1], n[2])
            } else {
                // Next to the first building when no tile is given.
                let t = u16::from_le_bytes([rd_bytes(uc, BLD_TABLE + 2, 2)[0], rd_bytes(uc, BLD_TABLE + 3, 2)[0]]) as u32;
                ((t & 0xff) + 2, (t >> 8) + 4)
            };
            let base = rd32(uc, 0x4e19a0);
            let Some(slot) = (1..800u32).find(|&i| rd_bytes(uc, base + i * 0x114 + 2, 1)[0] == 0) else { return "SEM ESPACO NA TABELA".into() };
            let rec = base + slot * 0x114;
            wr_bytes(uc, rec, &[player as u8]);
            wr_bytes(uc, rec + 2, &[7]);
            let tile = (y << 8) | x;
            for (f, regs) in [(0x457b87, [slot, tile, 0, ty]), (0x457122, [slot, 1000, 0, 0]), (0x4530fd, [slot, 0, 0, 0]), (0x44fe42, [slot, tile, 0, 0]), (0x445fbc, [slot, tile, 0, 0])] {
                let r = call_guest(uc, f, regs, &[]);
                if let Err(e) = r {
                    return format!("ERRO EM {f:#x}: {e}");
                }
            }
            format!("UNIDADE {ty} CRIADA NO SLOT {slot} EM ({x},{y})")
        }
        // `tab <hex addr> <stride> <count> [deref]`: dump the non-empty records of a table (deref: the address holds a pointer).
        ["tab", a, stride, count, rest @ ..] => {
            let (Some(mut a), Ok(stride), Ok(count)) = (hex(a), stride.parse::<usize>(), count.parse::<usize>()) else { return "USO: tab <addr> <tamanho> <n> [deref]".into() };
            if !rest.is_empty() {
                a = rd32(uc, a);
            }
            let mem = rd_bytes(uc, a, stride * count);
            let mut n = 0;
            for (i, rec) in mem.chunks(stride).enumerate() {
                if rec.iter().any(|&b| b != 0 && b != 0xff) {
                    n += 1;
                    eprintln!("[tab] {i}: {rec:02x?}");
                }
            }
            format!("{n} REGISTROS EM {a:#x}")
        }
        // Player records (0x14 bytes at [0x5080a8]): controller (+1), ally mask (+6), attitude toward each clan (+0xc, i16).
        ["clas"] => {
            let base = rd32(uc, PLAYERS);
            for p in 0..4u32 {
                let r = rd_bytes(uc, base + p * 0x14, 0x14);
                let att: Vec<i16> = (0..4).map(|o| i16::from_le_bytes([r[0xc + o * 2], r[0xd + o * 2]])).collect();
                eprintln!("[clan] {p} ctrl={} aliados={:04b} atitude={att:?}", r[1], r[6] & 15);
            }
            "OK".into()
        }
        ["att", a, b, rest @ ..] => {
            let (Ok(a), Ok(b)) = (a.parse::<u32>(), b.parse::<u32>()) else { return "USO: att <a> <b> [valor]".into() };
            if a > 3 || b > 3 {
                return "CLA 0-3".into();
            }
            let addr = rd32(uc, PLAYERS) + a * 0x14 + 0xc + b * 2;
            if let Some(v) = rest.first().and_then(|v| v.parse::<i16>().ok()) {
                wr_bytes(uc, addr, &v.to_le_bytes());
            }
            let v = rd_bytes(uc, addr, 2);
            format!("atitude[{a}][{b}] = {}", i16::from_le_bytes([v[0], v[1]]))
        }
        ["ally", a, b, rest @ ..] => {
            let (Ok(a), Ok(b)) = (a.parse::<u32>(), b.parse::<u32>()) else { return "USO: ally <a> <b> [0|1]".into() };
            if a > 3 || b > 3 {
                return "CLA 0-3".into();
            }
            let addr = rd32(uc, PLAYERS) + a * 0x14 + 6;
            let mut m = rd_bytes(uc, addr, 1)[0];
            match rest.first() {
                Some(&"1") => m |= 1 << b,
                Some(&"0") => m &= !(1 << b),
                _ => {}
            }
            wr_bytes(uc, addr, &[m]);
            format!("{a} aliado de {b}: {}", m >> b & 1)
        }
        // `grudge <vitima> <ofensor> <evento 0-21>`: fires one of the game's goodwill events (table at 0x497288).
        ["grudge", a, b, e] => {
            let (Ok(a), Ok(b), Ok(e)) = (a.parse::<u32>(), b.parse::<u32>(), e.parse::<u32>()) else { return "USO: grudge <vitima> <ofensor> <evento>".into() };
            match call_guest(uc, 0x475118, [a, b, e, 0], &[]) {
                Ok(_) => "EVENTO APLICADO".into(),
                Err(e) => e,
            }
        }
        // `steal <unidade> <predio> <0-3>`: orders a unit to steal from a building, like the thief command (0-2 resource, 3 technology).
        ["steal", u, b, k] => {
            let (Ok(u), Ok(b), Ok(k)) = (u.parse::<u32>(), b.parse::<u32>(), k.parse::<u32>()) else { return "USO: steal <unidade> <predio> <0-3>".into() };
            match call_guest(uc, 0x4456c9, [u, 3, b, k], &[]) {
                Ok(_) => "ORDEM DE ROUBO DADA".into(),
                Err(e) => e,
            }
        }
        // `aiexec <clan> [n]`: runs the AI's command executor on that clan's task list n times (the game only reaches it
        // for clans 1-3 when its build and economy modules are idle, which takes minutes).
        ["aiexec", p, rest @ ..] => {
            let Ok(p) = p.parse::<u32>() else { return "USO: aiexec <jogador> [n]".into() };
            let n: u32 = rest.first().and_then(|n| n.parse().ok()).unwrap_or(1);
            for _ in 0..n {
                if let Err(e) = call_guest(uc, 0x47d314, [0x5148b0 + p.min(4) * 16, 0, 0, 0], &[]) {
                    return e;
                }
            }
            format!("EXECUTOR RODOU {n}x")
        }
        // `diplo <proponente> <ia>`: what happens when a messenger of `proponente` enters the sanctuary of the AI clan `ia`.
        ["diplo", a, b] => {
            let (Ok(a), Ok(b)) = (a.parse::<u32>(), b.parse::<u32>()) else { return "USO: diplo <proponente> <ia>".into() };
            match call_guest(uc, 0x47fc1b, [a, b, 0, 0], &[0, 0, 0, 0]) {
                Ok(_) => "PROPOSTA ENVIADA".into(),
                Err(e) => e,
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::testing::bare_uc;

    /// Guest with player records at 0x590000 and the resource table zeroed.
    fn game() -> Uc<'static> {
        let mut uc = bare_uc();
        wr32(&mut uc, PLAYERS, 0x59_0000);
        uc
    }

    #[test]
    fn parsing_helpers() {
        assert_eq!(hex("0x4e2ea8"), Some(0x4e2ea8));
        assert_eq!(hex("ff"), Some(255));
        assert_eq!(hex("zz"), None);
        assert_eq!([kind_of("scrap"), kind_of("food"), kind_of("pedras"), kind_of("3"), kind_of("x")], [Some(0), Some(1), Some(2), Some(3), None]);
        assert_eq!(res_addr(1, 2), RES_TABLE + 9 * 8 + 2);
    }

    #[test]
    fn call_guest_passes_registers_and_stack_and_restores_state() {
        let mut uc = game();
        // add eax, edx ; add eax, [esp+4] ; ret
        wr_bytes(&mut uc, 0x40_1000, &[0x01, 0xd0, 0x03, 0x44, 0x24, 0x04, 0xc3]);
        let esp = reg(&uc, R::ESP);
        assert_eq!(call_guest(&mut uc, 0x40_1000, [1, 20, 0, 0], &[300]), Ok(321));
        assert_eq!(reg(&uc, R::ESP), esp);
    }

    #[test]
    fn call_guest_reports_the_faulting_address() {
        let mut uc = game();
        // mov eax, [0x12345678] ; ret
        wr_bytes(&mut uc, 0x40_1000, &[0xa1, 0x78, 0x56, 0x34, 0x12, 0xc3]);
        let e = call_guest(&mut uc, 0x40_1000, [0; 4], &[]).unwrap_err();
        assert!(e.starts_with("falha no jogo em 0x401000"), "{e}");
    }

    #[test]
    fn res_sets_and_reads_stock() {
        let mut uc = game();
        assert_eq!(run(&mut uc, "res comida 50"), "comida = 50");
        assert_eq!(rd_bytes(&uc, res_addr(1, 0), 2), 50u16.to_le_bytes());
        assert_eq!(run(&mut uc, "res pedra 99999 1"), "QUANTIDADE INVALIDA");
        run(&mut uc, "res pedra 20000 1");
        assert_eq!(rd_bytes(&uc, res_addr(2, 1), 2), 9999u16.to_le_bytes(), "capped at 9999");
        assert_eq!(run(&mut uc, "res all 7"), "TODOS = 7");
        assert_eq!(run(&mut uc, "res"), "metal 7, comida 7, pedra 7, extra 7");
        assert_eq!(run(&mut uc, "res ouro 1"), "RECURSO DESCONHECIDO: ouro");
    }

    #[test]
    fn peek_and_poke() {
        let mut uc = game();
        assert_eq!(run(&mut uc, "poke 401000 258 2"), "0x401000 = 258");
        assert_eq!(run(&mut uc, "peek 0x401000 4"), "02 01 00 00");
        run(&mut uc, "poke 401010 65537 4");
        assert_eq!(rd32(&uc, 0x40_1010), 65537);
        run(&mut uc, "poke 401020 300 1");
        assert_eq!(rd_bytes(&uc, 0x40_1020, 2), [44, 0], "width 1 keeps only the low byte");
        assert_eq!(run(&mut uc, "peek zz"), "ENDERECO INVALIDO");
        assert_eq!(run(&mut uc, "poke 401000 abc"), "ARGUMENTO INVALIDO");
    }

    #[test]
    fn attitude_and_alliance_bits() {
        let mut uc = game();
        assert_eq!(run(&mut uc, "att 1 2 -30"), "atitude[1][2] = -30");
        assert_eq!(run(&mut uc, "att 1 2"), "atitude[1][2] = -30");
        assert_eq!(rd_bytes(&uc, 0x59_0000 + 0x14 + 0xc + 4, 2), (-30i16).to_le_bytes());
        assert_eq!(run(&mut uc, "ally 0 2 1"), "0 aliado de 2: 1");
        assert_eq!(run(&mut uc, "ally 0 3 1"), "0 aliado de 3: 1");
        assert_eq!(rd_bytes(&uc, 0x59_0006, 1), [0b1100]);
        assert_eq!(run(&mut uc, "ally 0 2 0"), "0 aliado de 2: 0");
        assert_eq!(run(&mut uc, "ally 0 3"), "0 aliado de 3: 1");
        assert_eq!(run(&mut uc, "att 4 0"), "CLA 0-3");
        assert_eq!(run(&mut uc, "ally 0 9"), "CLA 0-3");
        assert_eq!(run(&mut uc, "att x y"), "USO: att <a> <b> [valor]");
        assert_eq!(run(&mut uc, "clas"), "OK");
    }

    #[test]
    fn instant_toggle_and_tick_finishes_buildings() {
        let mut uc = game();
        assert_eq!(run(&mut uc, "instant"), "CONSTRUCAO INSTANTANEA LIGADA");
        assert!(uc.get_data().instant_build);
        // Slot 3 under construction (state 2, type 1, progress 0, build time 10).
        let slot = BLD_TABLE + 3 * BLD_SIZE;
        wr_bytes(&mut uc, slot + 5, &[2]);
        wr_bytes(&mut uc, slot + 8, &[1]);
        wr_bytes(&mut uc, BLD_TIME + 0x34, &10u16.to_le_bytes());
        // A finished building must be left alone.
        wr_bytes(&mut uc, BLD_TABLE + 5, &[5]);
        tick(&mut uc);
        assert_eq!(rd_bytes(&uc, slot + 0xa, 2), 14u16.to_le_bytes());
        assert_eq!(rd_bytes(&uc, BLD_TABLE + 0xa, 2), [0, 0]);
        assert_eq!(run(&mut uc, "instant off"), "CONSTRUCAO INSTANTANEA DESLIGADA");
        wr_bytes(&mut uc, slot + 0xa, &[0, 0]);
        tick(&mut uc);
        assert_eq!(rd_bytes(&uc, slot + 0xa, 2), [0, 0]);
    }

    #[test]
    fn snapshot_and_diff_count_changed_words() {
        let mut uc = game();
        run(&mut uc, "snap");
        wr_bytes(&mut uc, 0x40_2000, &5u16.to_le_bytes());
        wr_bytes(&mut uc, 0x40_2100, &[1, 0, 2, 0]);
        assert_eq!(run(&mut uc, "diff up"), "UP: 3 PALAVRAS");
        assert_eq!(run(&mut uc, "diff up"), "UP: 0 PALAVRAS", "snapshot is renewed");
        run(&mut uc, "snap");
        wr_bytes(&mut uc, 0x40_2000, &9u16.to_le_bytes());
        assert_eq!(run(&mut uc, "diff"), "DIFF: 1 PALAVRAS");
    }

    #[test]
    fn tab_lists_non_empty_records() {
        let mut uc = game();
        wr_bytes(&mut uc, 0x40_3008, &[7]);
        assert_eq!(run(&mut uc, "tab 403000 8 4"), "1 REGISTROS EM 0x403000");
        wr32(&mut uc, 0x40_3100, 0x40_3000);
        assert_eq!(run(&mut uc, "tab 403100 8 4 deref"), "1 REGISTROS EM 0x403000");
        assert_eq!(run(&mut uc, "tab zz 8 4"), "USO: tab <addr> <tamanho> <n> [deref]");
    }

    #[test]
    fn ents_reports_used_slots_and_first_free() {
        let mut uc = game();
        wr_bytes(&mut uc, BLD_TABLE + 5, &[5]);
        assert_eq!(run(&mut uc, "ents"), "1 ENTIDADES, PRIMEIRO LIVRE: Some(1)");
    }

    #[test]
    fn usage_errors_and_unknown_commands() {
        let mut uc = game();
        assert_eq!(run(&mut uc, ""), "");
        assert!(run(&mut uc, "nada").starts_with("COMANDOS: res "));
        assert_eq!(run(&mut uc, "spawn 99"), "USO: spawn <tipo 0-25> [x y] [jogador]");
        assert_eq!(run(&mut uc, "grudge a b c"), "USO: grudge <vitima> <ofensor> <evento>");
        assert_eq!(run(&mut uc, "aiexec x"), "USO: aiexec <jogador> [n]");
        assert_eq!(run(&mut uc, "diplo a b"), "USO: diplo <proponente> <ia>");
        assert!(run(&mut uc, "tech nope 3").starts_with("AREA: "));
        assert_eq!(run(&mut uc, "tech agricultura x"), "QUANTIDADE INVALIDA");
        assert_eq!(run(&mut uc, "tech"), "agricultura 0, militar 0, civil 0, religiao 0");
    }

    /// Replaces the game functions the console calls with `mov eax, N ; ret`, to check the wiring and arguments.
    fn stub_game_functions(uc: &mut Uc) {
        for addr in [0x470f98u32, 0x4456c9, 0x475118, 0x47fc1b, 0x47d314, 0x457b87, 0x457122, 0x4530fd, 0x44fe42, 0x445fbc] {
            // Records the register arguments it was called with at 0x403000 + (addr & 0xff) * 16.
            let log = 0x40_3000 + (addr & 0xff) * 16;
            let mut code = vec![0xa3];
            code.extend_from_slice(&log.to_le_bytes()); // mov [log], eax
            code.extend_from_slice(&[0x89, 0x15]);
            code.extend_from_slice(&(log + 4).to_le_bytes()); // mov [log+4], edx
            code.extend_from_slice(&[0x89, 0x1d]);
            code.extend_from_slice(&(log + 8).to_le_bytes()); // mov [log+8], ebx
            code.extend_from_slice(&[0x89, 0x0d]);
            code.extend_from_slice(&(log + 12).to_le_bytes()); // mov [log+12], ecx
            code.extend_from_slice(&[0xb8, 1, 0, 0, 0, 0xc3]); // mov eax, 1 ; ret
            wr_bytes(uc, addr, &code);
        }
    }

    fn args_of(uc: &Uc, addr: u32) -> [u32; 4] {
        let log = 0x40_3000 + (addr & 0xff) * 16;
        [rd32(uc, log), rd32(uc, log + 4), rd32(uc, log + 8), rd32(uc, log + 12)]
    }

    #[test]
    fn grudge_diplo_and_aiexec_call_the_game_with_the_right_arguments() {
        let mut uc = game();
        stub_game_functions(&mut uc);
        assert_eq!(run(&mut uc, "grudge 1 2 9"), "EVENTO APLICADO");
        assert_eq!(args_of(&uc, 0x475118), [1, 2, 9, 0]);
        assert_eq!(run(&mut uc, "diplo 0 2"), "PROPOSTA ENVIADA");
        assert_eq!(&args_of(&uc, 0x47fc1b)[..2], [0, 2]);
        assert_eq!(run(&mut uc, "steal 16 0 3"), "ORDEM DE ROUBO DADA");
        assert_eq!(args_of(&uc, 0x4456c9), [16, 3, 0, 3], "unit, (unused), building, kind");
        assert_eq!(run(&mut uc, "aiexec 2 3"), "EXECUTOR RODOU 3x");
        assert_eq!(args_of(&uc, 0x47d314)[0], 0x5148b0 + 2 * 16);
        run(&mut uc, "aiexec 9");
        assert_eq!(args_of(&uc, 0x47d314)[0], 0x5148b0 + 4 * 16, "the clan index is capped at 4");
    }

    #[test]
    fn game_faults_are_reported_not_fatal() {
        let mut uc = game();
        // Nothing but zero bytes at the target: `add [eax], al` through eax=a faults or runs on; either way a text answer comes back.
        wr_bytes(&mut uc, 0x475118, &[0xa1, 0x78, 0x56, 0x34, 0x12, 0xc3]);
        assert!(run(&mut uc, "grudge 0 1 2").starts_with("falha no jogo em"));
        wr_bytes(&mut uc, 0x47fc1b, &[0xa1, 0x78, 0x56, 0x34, 0x12, 0xc3]);
        assert!(run(&mut uc, "diplo 0 1").starts_with("falha no jogo"));
        wr_bytes(&mut uc, 0x47d314, &[0xa1, 0x78, 0x56, 0x34, 0x12, 0xc3]);
        assert!(run(&mut uc, "aiexec 1").starts_with("falha no jogo"));
        wr_bytes(&mut uc, 0x470f98, &[0xa1, 0x78, 0x56, 0x34, 0x12, 0xc3]);
        assert!(run(&mut uc, "tech militar 3").starts_with("falha no jogo"));
    }

    #[test]
    fn tech_researches_missing_items_and_updates_the_totals() {
        let mut uc = game();
        stub_game_functions(&mut uc);
        wr_bytes(&mut uc, TECH_DONE + 16 + 1, &[2]); // player 1, area "militar": 2 done
        wr_bytes(&mut uc, TECH_TOTAL + 1, &[10]);
        assert_eq!(run(&mut uc, "tech militar 5 1"), "militar: 5 DESCOBERTAS");
        assert_eq!(rd_bytes(&uc, TECH_DONE + 16 + 1, 1), [5]);
        assert_eq!(rd_bytes(&uc, TECH_TOTAL + 1, 1), [13]);
        assert_eq!(args_of(&uc, 0x470f98), [1, 4, 1, 1], "the last call researches item 4 of area 1 for player 1");
        // Asking for fewer than already known changes nothing; the count is capped at 30.
        run(&mut uc, "tech 1 3 1");
        assert_eq!(rd_bytes(&uc, TECH_DONE + 16 + 1, 1), [5]);
        run(&mut uc, "tech civil 99");
        assert_eq!(rd_bytes(&uc, TECH_DONE + 2, 1), [30]);
        assert_eq!(run(&mut uc, "tech"), "agricultura 0, militar 0, civil 30, religiao 0");
    }

    #[test]
    fn spawn_creates_a_unit_in_the_first_free_slot() {
        let mut uc = game();
        stub_game_functions(&mut uc);
        let table = 0x50_0000u32;
        wr32(&mut uc, 0x4e19a0, table);
        wr_bytes(&mut uc, table + 0x114 * 1 + 2, &[7]); // slot 1 is taken
        assert_eq!(run(&mut uc, "spawn 4 10 12 2"), "UNIDADE 4 CRIADA NO SLOT 2 EM (10,12)");
        assert_eq!(rd_bytes(&uc, table + 0x114 * 2, 1), [2], "owner");
        assert_eq!(rd_bytes(&uc, table + 0x114 * 2 + 2, 1), [7], "state");
        assert_eq!(args_of(&uc, 0x457b87), [2, (12 << 8) | 10, 0, 4]);
        assert_eq!(args_of(&uc, 0x445fbc)[..2], [2, (12 << 8) | 10]);
        // Without coordinates it goes next to the first building (tile in the entity table, +2 columns, +4 rows).
        wr_bytes(&mut uc, BLD_TABLE + 2, &(5u16 << 8 | 3).to_le_bytes());
        assert!(run(&mut uc, "spawn 1").contains("EM (5,9)"));
        // A full table answers politely.
        for i in 1..800u32 {
            wr_bytes(&mut uc, table + i * 0x114 + 2, &[7]);
        }
        assert_eq!(run(&mut uc, "spawn 1 1 1"), "SEM ESPACO NA TABELA");
        // A game function that faults aborts the spawn with its address (fresh guest: Unicorn may keep
        // translated copies of code that was already executed and then overwritten).
        let mut uc = game();
        stub_game_functions(&mut uc);
        wr32(&mut uc, 0x4e19a0, table);
        wr_bytes(&mut uc, 0x445fbc, &[0xa1, 0x78, 0x56, 0x34, 0x12, 0xc3]);
        let r = run(&mut uc, "spawn 1 1 1");
        assert!(r.starts_with("ERRO EM 0x445fbc"), "{r}");
    }
}
