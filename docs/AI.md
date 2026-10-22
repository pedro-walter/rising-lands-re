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

The tick rate is 30 per second at normal speed (measured: `[0x508f7c]` advances 600 in 20 s).

Separately, every `FreqAlliance` (6000 ticks = 200 s, staggered by player number) it rolls `rand(3)`: on 0 it sends
a trade offer (command 0x2a) if it has a surplus to give, otherwise (2 in 3) it runs the diplomacy script (0x27,
below).

### Grudges (`FUN_00475118`)

`attitude[victim][offender]` changes by a delta from the table at `0x497288` (16 bytes per event: delta, "applies to
everyone" flag, counter, threshold; every threshold is 1) and is clamped to +-50. Deltas: events 0-2 -3/-3/-10,
3 -5, 4 -10, 5 -25, 6 -10, 7 -5, **8 -50 (applies to everyone)**, 9 -25, 10 -25, 11 -15, 12 +5, 13 +10, 14/15 +1,
16 +5, 17 +1, 18 +10. Callers: 8 = building destroyed (`FUN_0041a2c1`, `FUN_004266e1`), 3/4 = hits in the unit state
machine, 6/10 = theft and sabotage (`FUN_00450874`), 11 = `0x466fa7` (trade negotiation, both sides AI-checked),
16 = an alliance being formed (`0x46d9ee` in the mutual-alliance setter, and `0x480e02` in the AI script that
forms one): +5 each way. Gifts and trades raise it (`FUN_00475267`).

Mission 5 (headless, 4 minutes, `ents` dumps every 25 s): red, yellow and green only farmed and built; their
soldiers stayed idle. Nothing attacked.

## The black clan (owner 4: `FUN_0047b283`, then `FUN_0047d314`)

Configured by `MONSTRE.INI` (`.\monstre.ini`) and by per-map data.

* **Waves**: timer records at `0x50a9d0` (12 bytes: tile, level mask, interval, current level, last time), 8 slots
  per map, loaded from the map (`.ETA`, see `tools/eta_waves.py`). A record with an interval is a **spawn point**
  (a map-edge tile); every `interval` x 30 ticks (= interval seconds at normal speed) the level advances to the next
  set bit of its mask and `FUN_0047a81d` spawns a squad of owner-4 units from `0x50af84[level*26 + unit type]` at that
  tile. A record with interval 0 is an **ambush point** (see below). The starting level is random per game
  (`rand & 15` in `FUN_0047a601`), the first wave comes one interval after the map starts, and the squad waits in a
  queue of up to 16. Maps 8, 13 and 23 have no table: no waves.
* **Groups** (`MONSTRE.INI` `GROUPE00..15`): value -> unit via `0x498400` = `0:soldier, 1:firekeeper, 2:thief,
  3:mastaar, 4:lizard, 5:boar, 6:footman, 7:turtle`. `GROUPE08 = soldier+firekeeper`, `GROUPE09 = soldier+thief`,
  `10 = 2 lizards`, `11 = lizard+footman`, `12 = lizard+boar+footman`, `13 = mastaar+lizard+boar+footman`, `14 = all`.
* **Target**: for each squad it picks a random clan `(rand & 3) + 1`, wrapping 4 -> 0, so 25% each of
  red, yellow, green, blue. With `HumanOnly=1` it re-rolls until the clan is not an AI. Then it rolls `rand % 100`:

| roll | meaning | default | action |
|---|---|---|---|
| < `ProbaCaveJoueur` | the local human | 5% | random **warehouse** (building type 3, `CAVE`) of the human, command 0x30 |
| < + `ProbaCaveIA` | the random clan | 2% | random warehouse of that clan, command 0x30 |
| < + `ProbaEmbuscade` | ambush | 2% | random ambush point (a wave record with interval 0), command 0x2e; falls back to the warehouse |
| otherwise | roam | 91% | random map marker tile (terrain byte `0x1c`), command 0x2f |

Each candidate must be reachable (`FUN_0042d4f8`: same region id); it tries up to 20 times. Turtle squads
(type 24) skip the roll and go to marker tiles with command 0x31. In mission 23 (0x17) it only issues command 0x12.

`MONSTRE.INI` on the disc has no `Proba*` or `HumanOnly` keys, so the defaults apply. Adding
`ProbaCaveJoueur=`, `ProbaCaveIA=`, `ProbaEmbuscade=` and `HumanOnly=` under `[GENERAL]` changes this without any
code change (the game opens `.\monstre.ini` in its install directory).

### Waves per mission (from the `.ETA` files)

Interval in seconds at normal speed; "groups" are `MONSTRE.INI` `GROUPEnn` levels, so 0 = one soldier,
1 = firekeeper, 2 = thief, 3 = mastaar, 4 = lizard, 5 = boar, 6 = footman, 7 = turtle, 8 = soldier+firekeeper,
9 = soldier+thief, 10 = 2 lizards, 11 = lizard+footman, 12 = lizard+boar+footman, 13 = mastaar+lizard+boar+footman,
14 = all seven, 15 = nothing (an empty wave). Mission 5: three spawn points at 780, 1140 and 480 s, levels 0, 1, 2, 8, 9
(single soldiers, firekeepers or thieves), so the first raid arrives after 8 minutes. Waves get bigger and more
frequent: missions 1-3 have 1-2 spawn points every 3-6 minutes with single soldiers, 7-9 add lizards, 10-12 add
footmen, 14+ add boars, 18+ add mastaars (levels 3, 13), 21+ turtles; mission 22 has 11 spawn points and mission 24
has 15. Roam markers (terrain byte 0x1c) go from 3 in mission 1 to about 250 in missions 22 and 24.

## Attack targets (the 0x24 command)

`FUN_0047d314` is a small script interpreter (a behaviour tree: records of 0x14 bytes at `0x514910`, entry point per
command in the `u16` table at `0x51a720`, filled at startup). The planner's modes are not "styles" but a **building
priority list**: `FUN_00480aed` takes mode -> list of groups (`0x498468`, 0x18 bytes per mode) -> list of building
types (`0x4984f8`), picks the first group the target clan has, then the nearest building of it.

| mode | when | target groups, in order |
|---|---|---|
| 5 | attack phase, strength > 10 | type 21; Laboratory + Library; Garage + Hospital; Market + Hangar; Barracks, Circus, Monastery, Heliport, Workshop |
| 6 | strength > 10 | Market + Hangar; Garage + Hospital; Market + Hangar; Laboratory + Library |
| 2 | second force value >= 40 | Garage + Hospital; Barracks, Circus, ...; Market + Hangar |
| 3 (command 0x25) | few defenders at home | Market + Hangar |

(Building types: 0 lighthouse, 1 barracks, 2 sanctuary, 3 storage `CAVE`, 4 farm, 5 workshop, 6 library, 7 lab, 8 hangar,
9 wall, 10/11 catapult/cannon tower, 12 portcullis, 13 circus, 14 garage, 15 monastery, 16 market, 17 crystal,
18 heliport, 19 hospital, 20 fusion monastery, from the list in `RISING.INI`; type 21 has no name there.) The
sanctuary, storage and homes are not in any list: the AI raids support buildings first.

## Alliances and victory

* `allied(a, b)` (`FUN_0046d838`) reads a bit in **a's own** mask (`record + 6`), so it is one-sided. The human toggles
  it with command 0x1d (`FUN_004637e0`); command 0x26 (`FUN_0046d927`) sets both directions and fires grudge event 16
  (+5 each). Attacking an ally clears both directions (`FUN_0046d8cb` in `FUN_00450874`).
* An objective of type 5 ("eliminate every enemy clan", `FUN_00458606`) is satisfied when every clan that is **not
  allied with you** has no units and no buildings. So allying with everybody satisfies it at once; nothing in the code
  limits the number of allies. The manual says the same ("an enemy clan is one you have no alliance with"). Other
  objective types (unit counts, resources, buildings) do not look at alliances.
* The AI is not bound by alliances: the target score gets +25 for an ally, so it can still attack one that it likes
  much less than the others. Its diplomacy script (command 0x27) needs a Market, then: with no ally it goes for the
  clan it likes best (highest attitude) and proposes (messenger to that clan's sanctuary); with allies it looks at the
  clan it likes least and, if that is an ally, **breaks the alliance** (`FUN_00480d97`). Trade offers are valued at
  `(attitude + 80 + 40 if allied)` percent of what is asked, so goodwill 0 = 80%, allied +50 = 170%.

## Carry-over between missions

Alliances and grudges live in the live player table (`[0x5080a8]`), which is reset only when a new game starts
(`FUN_00474beb` -> `FUN_0046dcb4`). At the end of a mission it is copied to a backup table (`FUN_0046dee5`), and a mission
load restores it from that copy (`FUN_0046df41`), so both carry over, as the manual says ("alliances and goodwill are
carried from one mission to the next"). The saved game stores them too (`FUN_0046ddfc`). Not tested in play.

They have a visible effect: **the map file is chosen from the grudges.** `FUN_0046c1b8` builds the name
`World\MondeNN` + suffix. For missions 10, 12 and 17 the suffix is `a`, `b` or `c` for red, yellow or green: the clan with the
lowest attitude towards you (ties go to red). Mission 9 uses `a` to `d` from a count (0-3) saved when mission 8 ends.
The variants are different maps (100,000-250,000 bytes differ in 1.3 MB). So the clan that likes you least in the
campaign is the one you face in 10, 12 and 17: be generous to one clan, or hostile to another, to steer it.

## Open questions

* How a human alliance proposal is accepted or refused (the messenger at the sanctuary: `FUN_00466dd1` is the trade
  state machine; the alliance decision itself is not decoded).
* What the mission 9 count is exactly, and what the A/B/C variants change besides the layout.
* Why mission 22 has a spawn point every 60 s at (0, 204).
* Whether the clan AI ever re-enables `Ia.INI`-style tuning: a hook on `0x47fc54` could read it for real.
