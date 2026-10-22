//! Debug console: commands that poke the running game's memory (stockpile, cheats, ...).
//! Opened with F12 in the window, or from scripts with `dbg <command>`.

use crate::machine::{rd32, rd_bytes, wr_bytes, Uc};

/// Per-player resource table: 8-byte entries indexed `player * 4 + kind`, current amount at +2 (u16).
const RES_TABLE: u32 = 0x4e2ea8;
const RES_NAMES: [&str; 4] = ["madeira", "comida", "pedra", "ouro"];

fn res_addr(kind: u32, player: u32) -> u32 {
    RES_TABLE + (player * 4 + kind) * 8 + 2
}

fn kind_of(s: &str) -> Option<u32> {
    match s {
        "0" | "wood" | "madeira" => Some(0),
        "1" | "food" | "comida" => Some(1),
        "2" | "stone" | "pedra" => Some(2),
        "3" | "gold" | "ouro" => Some(3),
        _ => None,
    }
}

fn set_u16(uc: &mut Uc, addr: u32, v: u16) {
    wr_bytes(uc, addr, &v.to_le_bytes());
}

pub const HELP: &str = "res <madeira|comida|pedra|ouro|all> <n> [jogador] | peek <addr> [n] | poke <addr> <valor> [1|2|4] | snap | diff | help";

/// Memory snapshot taken by `snap`, for `diff` (region start, bytes).
static SNAP: std::sync::Mutex<Vec<(u64, Vec<u8>)>> = std::sync::Mutex::new(Vec::new());

fn hex(s: &str) -> Option<u32> {
    u32::from_str_radix(s.trim_start_matches("0x"), 16).ok()
}

/// Writable guest regions worth diffing (the game's data and its heap, not the stack or the screen).
fn data_regions(uc: &Uc) -> Vec<(u64, usize)> {
    uc.mem_regions().unwrap_or_default().iter().filter(|r| r.perms & 2 != 0 && r.end - r.begin < 0x400_0000).map(|r| (r.begin, (r.end - r.begin + 1) as usize)).collect()
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
        ["peek", a, rest @ ..] => {
            let (Some(a), n) = (hex(a), rest.first().and_then(|n| n.parse::<usize>().ok()).unwrap_or(16)) else { return "ENDERECO INVALIDO".into() };
            let b = rd_bytes(uc, a, n.min(64));
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
        ["diff"] => {
            let snap = SNAP.lock().unwrap();
            let mut n = 0;
            for (b, old) in snap.iter() {
                let Ok(new) = uc.mem_read_as_vec(*b, old.len()) else { continue };
                let mut i = 0;
                while i + 2 <= old.len() {
                    if old[i..i + 2] != new[i..i + 2] {
                        n += 1;
                        if n <= 400 {
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
