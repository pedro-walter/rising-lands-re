# The thief ("Ladrão" / `[VOLEUR]`)

The thief is the unit the PT-BR release calls **ARQUEIRO** (`[VOLEUR]` in `RISING.INI`, unit type 9 in the
26-record table at `0x4965ae`). It is an ordinary ranged unit until it is ordered to steal; the manual calls that
"turning the archer into a thief". There is no separate unit: the order puts the unit in action state `0x17`, and the
state machine in `FUN_0045442c` (case `0x17`) does the rest. Everything below was read from the code and then checked
by running the original game under `rl-emu` (see "How it was checked").

## Giving the order

Select the archer, click the target-icon in the interface bar, then:

* **a resource icon** (metal scraps, food or stone) and then an enemy **Armazém** (`CAVE`, building type 3), or
* **the technology icon** and then an enemy **Laboratório** (`LABORATOIRE`, building type 7).

The hover code (around `0x429e89`) accepts type 3 when the selected kind is 0-2 and type 7 when it is 3. That is the
only place the building type is checked: the execution code does not look at it.

The click becomes network command `0x22` with `(building, kind)`; `FUN_004733b8` applies it to **every live unit in the
clan's selection list**, calling `FUN_004456c9(unit, building, kind)`. That sets `unit+2 = 0x17`, `unit+0xc0 = building`,
`unit+0xc4 = kind` (0 metal scraps, 1 food, 2 stone, 3 technology) and paths to the nearest tile of the building
(`FUN_0041aee2`). Ordering any other command before the thief enters cancels the theft at no cost, as the manual says.
None of the AI commands documented in `docs/AI.md` is `0x22`, so as far as I found only a human (or a network peer)
steals (the thieves in the AI raid groups of `MONSTRE.INI` were not seen to use it).

## What happens each tick (case `0x17`)

1. The target is dropped (the unit goes idle, nothing is spent) if the building is in state 3 (`building+5 == 3`), if its
   owner is `200` (under construction / nobody's), or if its owner is the thief's own clan.
   **Allies are not excluded**: nothing here looks at the alliance mask.
2. If the thief is not on the building's footprint plus a one-tile ring (`FUN_0042381f`) it keeps walking; if its path has
   ended it re-paths to the nearest tile.
3. Once adjacent it **rolls exactly once** (below). The action ends on that tick: `FUN_00457058` clears the order and then
   the unit's state byte is set to 0, i.e. **the thief is removed whether it won or lost**. Manual: "se o Arqueiro obtém
   sucesso na sua missão, ele desaparecerá para sempre", and the same is true when it fails.

## The roll

Per clan there is one byte, the **steal counter** `c`, at `0x4e19bc + clan` (4 bytes, clans 0-3). It is set to
`0x32` = **50** for every clan when a map is loaded (seen: `32 32 32 32` at the start of mission 1) and is stored and
restored with save games (the save/load routines at `0x46b669`/`0x46b7a7` write and read the 4 bytes).

```
r = rand() % 100            // FUN_004704dc, Park-Miller (16807 mod 2^31-1), one global seed at 0x497164
success  <=>  r <= c        // jbe at 0x45610a
```

so **P(success) = (c + 1) / 100**: 51 % for the first thief of a mission, never above 51 %, never below 1 %.

The counter is only ever pushed down:

| result | counter after | player sees |
|---|---|---|
| failure | `c - 5` (floored at 0) | message 502 "A tentativa de roubo fracassou!!!" |
| success, resource theft | `c / 2` (integer) | message 501 "Nosso LADRÃO teve sucesso!" |
| success, tech theft, thief's clan was behind | `c / 2` | message 501 |
| success, tech theft, thief's clan was *not* behind | unchanged | nothing (but see the grudge below) |

Nothing raises it again (the only writes are the two initialisers and the three above). The counter belongs to the
**thief's clan**, not to the target: it is shared by every victim, and it is shared by all future thieves that clan sends in
this mission. The messages are only shown when the thief's owner is the local player.

### The odds, worked out

Per attempt, by counter value:

| `c` | 50 | 45 | 40 | 30 | 25 | 22 | 12 | 6 | 3 | 1 | 0 |
|---|---|---|---|---|---|---|---|---|---|---|---|
| P(success) | 51 % | 46 % | 41 % | 31 % | 26 % | 23 % | 13 % | 7 % | 4 % | 2 % | 1 % |

Sending thieves one after another from a fresh mission (counter 50), including how each result moves the counter:

| thief # | P(this one succeeds) | expected successes so far | P(at least one success so far) |
|---|---|---|---|
| 1 | 51.0 % | 0.51 | 51.0 % |
| 2 | 35.8 % | 0.87 | 73.5 % |
| 3 | 25.7 % | 1.12 | 84.4 % |
| 4 | 18.3 % | 1.31 | 90.0 % |
| 5 | 12.2 % | 1.43 | 93.1 % |
| 6 | 7.2 % | 1.50 | 94.9 % |
| 10 | 1.2 % | 1.59 | 97.2 % |

* A chain of successes goes 50 → 25 → 12 → 6 → 3 → 1 → 0, i.e. the 2nd, 3rd, 4th... **consecutive** successes have
  26 %, 13 %, 7 %, 4 %... odds. Stealing twice in a row from the same raid is a ~13 % event.
* A chain of failures goes 50 → 45 → 40 → ... → 0 in steps of 5, so a failure costs 5 points of odds, a success costs
  half of what is left.
* Over an unlimited number of thieves a clan gets about **5.5 successes in total** in one mission, and needs about
  **4.9 thieves** for its first one when it starts from scratch. (Computed from the transition rule above with a
  small script; the rule itself is the part verified in the game.)
* The first thief is the best one: every later one has worse odds, whatever the target.
* Multiplayer: the roll uses the same lock-stepped RNG as everything else, so all peers agree on the result.

Cost of a thief: `COUTPATATE=8` (8 food), `TMPSRECRUTE=1000`, no stone or scraps (`[VOLEUR]` in `RISING.INI`).

## Resource theft (`kind` 0-2)

Success runs, in this order:

1. **Grudge event 3** (-5): `FUN_00475118(victim, thief clan, 3)`, see `docs/AI.md`. The AI clan that was robbed likes the
   thief's clan 5 less (a human victim has no attitude to change; this matters for AI victims).
2. The three stocks of the victim are read with `FUN_0046769d`. For the chosen resource,
   `stolen = floor((stock - w) / 2)`, where `w` is the first u16 of the stock entry (`0x4e2ea8 + (clan*4+kind)*8`; the
   current stock is the second u16). At the start of a mission `w` equals the starting stock (30), and by my reading
   of the code harvest deposits (`FUN_004678f2`) add to the current stock only, while spending subtracts from both
   (`FUN_0046792c`), so `w` acts as a floor: **the starting stock cannot be stolen until it has been spent**, and
   after a theft that took more than `w`, `w` becomes 0.
3. The victim loses `stolen` (`FUN_004235bd` → `FUN_0046792c`, floored at 0).
4. The thief's clan gains `stolen` through `FUN_0042340f` into a storage found by `FUN_00461222(clan, type 3)`, capped by
   that clan's storage capacity (words at `0x4dfb2c + clan*6`: storage count and a per-storage figure, 100 at the start;
   the product is the limit shared by the three resources). The excess is lost. **If the thief's clan has no storage at
   all, nothing is gained**: the victim still loses the goods.
5. The counter is halved, and the thief is removed.

Measured (mission 1 data, clan 1 as the thief, clan 0 the victim): victim food 500 with `w` = 30 → 265 (stole
`(500-30)/2 = 235`); next theft 265 with `w` = 0 → 133 (stole 132). The thief's clan had no storage and stayed at 100.

## Technology theft (`kind` 3)

1. **Grudge event 4** (-10) is fired first, before anything is checked.
2. `A` = the victim's *currently researched area* (byte `+4` of its record at `0x5080f4 + clan*16`; the four areas are
   agriculture, military, civil, religion). The theft only does anything if the thief's clan has **fewer** items done in
   `A` than the victim (levels are the bytes `0x5080f4 + clan*16 + area`). Otherwise the roll was a "success" that gives
   nothing: no message, the counter stays, the thief is gone, the victim still dislikes you by 10.
3. Otherwise `FUN_0047149d(thief clan)`: the thief's clan gets **+1 level in *its own currently researched area*** (not
   in `A`), its research progress is reset to 0 and the item's effect is applied (`FUN_00470f98`). Message 501, the
   counter is halved, the thief is removed. The victim loses nothing.

**Fix in `rl-emu`:** that silent case used to show nothing, which looked like a lost message. `rl-emu` now shows message 503,
"O roubo não rendeu nada: não há tecnologia a roubar!" (added to `EVENTS.INI` when it is read, `Vfs::TEXT_FIXES`; the
install is untouched), to the thief's owner. The stub is `patch_silent_tech_theft` in `machine.rs`; the counter and the
rest of the logic are unchanged. Seen in a mission 18 save: the player led every clan, so every rolled success was silent.

Quirk worth knowing: the comparison uses the victim's current area, the reward goes to the thief clan's current area. In
the test the victim led agriculture 6 to 2, the thief's clan was researching civil, and the gain was **civil 4 → 5**.
If the thief's clan is researching something nearly done, the theft effectively completes it and restarts the area's
progress; the code does not check for a maxed-out area (not analysed what happens then).

## Failure

`c - 5`, the thief is removed, message 502. **No grudge, nothing is taken.** The victim is not told and nothing in the
game records that it happened.

## Oddities and possible bug-fix material

* The known original crash when the pointer re-enters the window: the steal action reads
  `FUN_0041a4f4(building under the cursor)` with the "nothing there" value `0xffff`, which indexed far outside the table.
  `rl-emu` patches the accessor (`machine.rs`, `apply_patches`); the stealing logic itself is untouched.
* A clan without a storage still destroys the victim's goods when it succeeds (step 4 above).
* The tech reward/target area mismatch above looks unintended.
* Allies can be robbed: this handler has no alliance check, and the only side effect on the victim is grudge event 3 or 4
  (there is no alliance break, unlike an attack on an ally, events 6+10).
* The counter is per clan for the whole mission, so sending ten thieves at once is no better than sending them one by
  one: each roll sees the counter left by the previous one.

## How it was checked

`rl-emu` has a console command for this, `steal <unit slot> <building slot> <0-3>` (it calls `FUN_004456c9`; see
`docs/DEBUG.md`). Mission 1 was started headless, a thief of clan 1 spawned next to the clan 0 sanctuary (`spawn 9 14 14
1`) and ordered at it, then the counters and stocks were read back with `peek`. Eight rolls in total:

| attempt | counter before → after | outcome | effect seen |
|---|---|---|---|
| food A1 | 50 → 45 | failure | stock unchanged |
| food A2 | 45 → 22 | success | 500 → 265 |
| food B1 | 50 → 25 | success | 500 → 265 |
| food B2 | 25 → 20 | failure | unchanged |
| food B3 | 20 → 10 | success | 265 → 133 |
| tech C1 | 50 → 45 | failure | levels unchanged |
| tech C2 | 45 → 22 | success | thief clan civil 4 → 5, victim unchanged |
| tech C3 | 22 → 17 | failure | unchanged |

Every transition is `c - 5` or `c / 2`. That the success probability is exactly `(c+1)/100` comes from the compare at
`0x456108-0x45610a` (`rand() % 100` against `c`, `jbe` means success), not from a measured frequency: eight rolls cannot
show it.

## Not checked

* Whether the thief's clan gains the goods when it does own a storage (I could not get the capacity words above 0 by
  cloning a building record; only the no-storage case was run).
* What the unit does when its target is destroyed on the way, beyond the code reading above.
* Whether the RNG seed is part of save games (no code writes `0x497164` besides the generator itself, so probably not:
  reloading a save before a theft may give a different roll).
