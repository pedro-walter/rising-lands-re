# How the AI works (from the decompilation)

Read from `RISING.EXE` (Ghidra + capstone), not observed in play unless a line says so. Function names are
addresses (`FUN_0047f927` = `0x47f927`).

## Who is controlled by whom

Players are 0..3 (blue, red, yellow, green) plus **owner 4 = the "black" clan** (creatures / shadow raiders).
`FUN_0046d817(p)` returns the controller of `p`: records of 0x14 bytes at `[0x5080a8]`, byte +1 is
0 none / 1 human / 2 AI. Players >= 4 always report 2. The default single-player setup
(`FUN_0046dcb4`) is blue = human, red, yellow, green = AI. The main tick (`FUN_0042c868`, case 0x12) loops over
players 0..4 and calls `FUN_0047f927(player, mission)` for every AI one.

## Red / yellow / green (`FUN_0047f927`, players 1-3)

All three run the same code with the same numbers. The first module that returns an action wins each tick:

1. defeat check: no units and no buildings for 10 checks in a row eliminates the clan (`FUN_00483d95`)
2. `FUN_00482936` (stub, does nothing)
3. `FUN_0048453c` queues builds from a template table (`PTR_DAT_00498c24[personality]`)
4. `FUN_00485183` economy: opening (build 2 key buildings, then personality 0 -> 1), farmers, stone, population
5. `FUN_004866d7` military and diplomacy planner (below)
6. `FUN_0047d314` command executor

"Personality" 0/1 (`FUN_004846a7`) is a game phase, not a clan trait. Both phases load identical defaults.

### The numbers are hard-coded

`FUN_0047fc54` is the stub of a `GetPrivateProfileInt` call: it returns its default (`mov eax, ebx; ret`).
The original read them from `\Ia.INI`, which is not on the disc. Defaults:
`FreqSurvie 10500, FreqEconomie 3500, FreqDefense 1500, FreqAttaque 1000, ForceParBat 5/3/15/10`,
`SurplusMadEco 10, SurplusMadNormal 10, SeuilRessourcesPourAttaque 40, SeuilBatimentsPourAttaque 10,
SeuilForcePourAttaque 10, FreqAlliance 6000`, resource keys (`FreqEconomie 800`, `MaxChamps 5`,
`NbPatatesParFermier 80`, ...). Unit values for army strength (`ValeurUniteType`, table `0x498cf4`):
soldier 2, firekeeper 4, captain 10, bear 8, priest 10, spyder 10, bunker 15, sun 20, rhino 12 ...

### Target choice (`FUN_004866d7`)

Every period (staggered by player number) the planner computes its army strength and then picks a target clan:

```
best = 500; for other in 0..3, other != me:
    score = attitude[me][other]            (FUN_0046dc6c, clamped to -50..+50)
    if allied(me, other): score += 25      (FUN_0046d838)
    if score < best: best = score; target = other     (strict <)
```

Lowest score is attacked. Everyone starts at 0 and the loop starts at player 0, so with no grudges **all three AI
clans pick blue (the human)**. An AI only picks another AI after a grudge (a lower score) or an alliance with blue.
Then, with `strength` = its army value:

| condition | order |
|---|---|
| attack phase and strength > 10 | command 0x24 mode 5 against target |
| else strength > 10 | command 0x24 mode 6 |
| else, defenders at home < 10 | command 0x25 mode 3 (build up) |
| else, second force value >= 40 | command 0x24 mode 2 |

Separately, every `FreqAlliance` (6000) ticks it rolls a random number: on 0 it proposes (0x27), otherwise it
may send an offer (0x2a) when it has something to trade.

### Grudges (`FUN_00475118`)

`attitude[victim][offender]` changes by a delta from the table at `0x497288` (16 bytes per event) and is
clamped to +-50. Deltas: events 0-2 -3/-3/-10, 3 -5, 4 -10, 5 -25, 6 -10, 7 -5, **8 -50 (applies to everyone)**,
9 -25, 10 -25, 11 -15, 12 +5, 13 +10, 14/15 +1. Callers: 8 = building destroyed (`FUN_0041a2c1`, `FUN_004266e1`),
3/4 = hits in the unit state machine, 6/10 = theft and sabotage (`FUN_00450874`), 11 = `0x466fa7`, 16 = `0x46d9ee`
and `0x480e02` (not decoded). Gifts and trades raise it (`FUN_00475267`).

Mission 5 (headless, 4 minutes, `ents` dumps every 25 s): red, yellow and green only farmed and built; their
soldiers stayed idle. Nothing attacked.

## The black clan (owner 4: `FUN_0047b283`, then `FUN_0047d314`)

Configured by `MONSTRE.INI` (`.\monstre.ini`) and by per-map data.

* **Waves**: timer records at `0x50a9d0` (12 bytes: id, level mask, interval, current level, last time). Every
  `interval` ticks the level advances to the next set bit of the mask and `FUN_0047a81d` spawns a squad of
  owner-4 units from `0x50af84[level*26 + unit type]`. The squad waits in a queue of up to 16.
* **Groups** (`MONSTRE.INI` `GROUPE00..15`): value -> unit via `0x498400` = `0:soldier, 1:firekeeper, 2:thief,
  3:mastaar, 4:lizard, 5:boar, 6:footman, 7:turtle`. `GROUPE08 = soldier+firekeeper`, `GROUPE09 = soldier+thief`,
  `10 = 2 lizards`, `11 = lizard+footman`, `12 = lizard+boar+footman`, `13 = mastaar+lizard+boar+footman`, `14 = all`.
* **Target**: for each squad it picks a random clan `(rand & 3) + 1`, wrapping 4 -> 0, so 25% each of
  red, yellow, green, blue. With `HumanOnly=1` it re-rolls until the clan is not an AI. Then it rolls `rand % 100`:

| roll | meaning | default | action |
|---|---|---|---|
| < `ProbaCaveJoueur` | the local human | 5% | random **warehouse** (building type 3, `CAVE`) of the human, command 0x30 |
| < + `ProbaCaveIA` | the random clan | 2% | random warehouse of that clan, command 0x30 |
| < + `ProbaEmbuscade` | ambush | 2% | random ambush point from the map's table, command 0x2e; falls back to the warehouse |
| otherwise | roam | 91% | random map marker tile (terrain byte `0x1c`), command 0x2f |

Each candidate must be reachable (`FUN_0042d4f8`: same region id); it tries up to 20 times. Turtle squads
(type 24) skip the roll and go to marker tiles with command 0x31. In mission 23 (0x17) it only issues command 0x12.

`MONSTRE.INI` on the disc has no `Proba*` or `HumanOnly` keys, so the defaults apply. Adding
`ProbaCaveJoueur=`, `ProbaCaveIA=`, `ProbaEmbuscade=` and `HumanOnly=` under `[GENERAL]` changes this without any
code change (the game opens `.\monstre.ini` in its install directory).

## Open questions

* What the 0x1c marker tiles are on each map, and how the wave timer records are stored in the `.ETA` files.
* The meaning of command codes 0x24/0x25 modes (attack styles) in `FUN_0047d314`.
* Event numbers 11 and 16 in the grudge table.
* Whether the clan AI ever re-enables `Ia.INI`-style tuning: a hook on `0x47fc54` could read it for real.
