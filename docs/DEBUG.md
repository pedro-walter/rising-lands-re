# Debug console and tooling

Press **F12** in the window to open the console (Enter runs, Esc closes). From scripts use
`RL_SCRIPT="45:dbg:res comida 500"`. Launch flags: `--skip-intro`, `--new-game` (mission 1).

| Command | Effect |
|---|---|
| `res` | show the stockpile; `res <metal\|comida\|pedra\|all> <n> [player]` sets it |
| `instant [on\|off]` | buildings finish the moment a builder starts them |
| `tech` | researched count per area; `tech <agricultura\|militar\|civil\|religiao> <n> [player]` researches up to `n` (applies each effect through the game's own function) |
| `spawn <type 0-25> [x y] [player]` | creates a unit (default: next to the first building) by calling the game's own unit init |
| `tab <addr> <stride> <n> [deref]` | dump the non-empty records of a table |
| `ents` | list the entity table (buildings) |
| `peek <hex> [n]`, `poke <hex> <v> [1\|2\|4]` | read/write guest memory |
| `snap`, `diff`, `diff up` | memory snapshot and what changed since (`up`: u16 words that grew, re-arms itself) |

Env vars: `RL_LOG_SLOW` (host calls > 5 ms, late frames), `RL_LOG_INI=<section substring>` (INI reads with the
caller), `RL_WATCH=addr:len` + `RL_WATCH_MOD=stride:offset` (logs guest writes with a short call stack).
Script commands: `dbg`, `find a,b`, `mem addr:len`, `bld`.

## Memory map found so far (guest addresses)

* Stockpile: 8-byte entries at `0x4e2ea8`, index `player*4 + kind` (0 metal scraps, 1 food, 2 stone); current amount is the u16 at +2 (cap 9999).
* Entity table (buildings): 1000 slots of `0x114` bytes at `0x49a0c8`. +2 tile (`y<<8|x`), +4 owner (200 while under construction),
  +5 state (0 free, 2 under construction, 4/5 idle), +8 type (2 sanctuary, 3 storage, ...), +0xa build progress (u16, counts up by 2 to
  the u16 at `0x495d90 + type*0x34`; reaching it finishes the building).
* Research definitions (from `RISING.INI`): 30 items per area, 12 bytes each, areas at `0x50817c`, `0x5082e4`, `0x50844c`, `0x5085b4`
  (agriculture, military, civil, religion). Per-player progress: bytes at `0x5080f4 + player*16 + area`, total at `0x50871c + player`;
  `FUN_00470f98(AL=player, EDX=item, BX=area, ECX=1)` applies an item's effect.
* Unit stats from `RISING.INI`: 0x32-byte records at `0x4965ae` (26 unit types); recruit time at +0.
* Game speed/menus: screen id byte at `0x4e2f78`, frame counter `0x508f7c`.
* Units: pointer at `0x4e19a0` to 0x114-byte records (slot 0 unused). +0 owner, +1 type (0-25), +2 state (7 idle), +8/+0x7c/+0x7e tile (`y<<8|x`).
  `FUN_00457b87(EAX=slot, EDX=tile, CL=type)` initialises a unit; training completion (`FUN_00420e4c`) then calls `0x457122`, `0x4530fd`, `0x44fe42`, `0x445fbc(EAX=slot, EDX=tile)`.
* Building training: +0x74 active flag, +0x7a queue, +0x8c progress (done at recruit time * 100).
