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

pub const HELP: &str = "res <madeira|comida|pedra|ouro|all> <n> [jogador] | help";

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
        [] => String::new(),
        _ => format!("COMANDOS: {HELP}"),
    }
}
