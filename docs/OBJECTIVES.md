# Mission objectives (all 25 missions)

Every `MONDEnn.ETA` carries an objective table, read right after the **zone table** at the end of the unit tail
(loader `FUN_0046c2cf` -> `FUN_0047c04a` zones, `FUN_00458b05` objectives). `tools/eta_goals.py original/WORLD` prints
everything below straight from the files. The check is `FUN_00458606`, run every tick.

## Format

Both tables are `u16 count`, `u16 size` (10), then `count` records of 10 bytes. The objective table always has 10 slots
(unused ones start with `0xff`), copied to `0x4e1d20`; each slot has a "done" flag at `0x4e1d88 + slot`.

**Zone record** (`0x50c614`, up to 400): `u16 x1, y1, x2, y2` (cells, inclusive) and a spare `u16`. The zone of a cell is the
first record that contains it (`FUN_0047bf66`).

**Objective record**: `+0` type, `+2` `u16` value, `+4/+6/+8` up to three `u16` zone ids (`>= 0xff` ends the list).

| type | condition for the slot to be satisfied |
|---|---|
| 0 | the clan's **food** stock >= value (stockpile entries at `0x4e2ea8`, +2 is the amount) |
| 1 | **stone** stock >= value |
| 2 | **metal** stock >= value (not used by any mission) |
| 3 | discoveries the human clan completed **in its own lab this mission** (`0x50871c[clan]`) >= value. Stolen techs do not count |
| 4 | the clan owns a building of type `value` (0 lighthouse, 1 barracks, 2 sanctuary, 3 storage, 6 library) standing in any of the listed zones |
| 5 | every clan the player is **not allied with** has no units and no buildings (see `AI.md`) |
| 8, 0xff | ignored (a leading 0xff in slot 0 would make the mission unwinnable; no map has it) |

**All slots must hold together** for the mission to be won, and a slot that stops holding (for example the sanctuary is
destroyed) is un-done again. Two details outside the table:

* **Mission 6 ignores the table.** The result is instead whether cell (225, 59), the single-cell zone 0, has been revealed in the
  clan's fog-of-war map (bit 0x10 of `[0x507d88 + clan][cell]` cleared): "explore the mine".
* In missions 8 and 13 the check also counts how many rival clans already have a building of the objective's type in the zone
  (`0x4e1d85`). Only mission 9 uses it, to choose its map variant (see `AI.md`).

Only the **human clan's** state is tested; nothing here is shared with allies.

## Table

Coordinates are cells (x, y), the map is 256 x 256 with north up.

| # | briefing summary | objectives in the file |
|---|---|---|
| 1 | protect a cave; store 50 food and 50 stone | storage in z0 (x0-28, y0-25); food >= 50; stone >= 50 (listed twice) |
| 2 | sanctuary, barracks, 50 food | barracks and sanctuary in z0 (x36-63, y0-23); food >= 50 |
| 3 | build a lab, develop technology | 1 discovery |
| 4 | market, diplomat ally with red, else destroy the rival | eliminate non-allied clans |
| 5 | find the 3 other clans, eliminate non-allied | eliminate non-allied clans |
| 6 | explore the mine | **special**: cell (225, 59) revealed (the table's type 5 is ignored) |
| 7 | develop two technologies | 2 discoveries |
| 8 | cross the ocean, build a sanctuary | sanctuary in z0 (x106-127, y9-55) or z1 (x106-127, y81-127) |
| 9 | make two discoveries | 2 discoveries |
| 10 | destroy the enemy clans | eliminate non-allied clans |
| 11 | cross the swamps, 50 food, a technology | 1 discovery; food >= 50 |
| 12 | enter the enemy block, sanctuary in the north-east, destroy all | sanctuary in z0 (x118-153, y11-43) or z1 (x96-125, y0-16); eliminate non-allied clans |
| 13 | sanctuary as fast as possible in the east | sanctuary in z0 (x227-255, y14-51), z1 (x230-255, y98-134) or z2 (x232-255, y207-234); two `food >= 0` slots (always true) |
| 14 | resist the creatures, sanctuary + storage, 50 stone and 100 food | food >= 100; storage and sanctuary in z0 (x9-58, y2-45). **The 50 stone in the briefing is not checked** |
| 15 | expand the clan, destroy the enemies | **1 discovery**; eliminate non-allied clans |
| 16 | destroy the enemy clans, new technology | eliminate non-allied clans; 1 discovery |
| 17 | escape the prison, return to the clan, destroy the clans | 1 discovery; eliminate non-allied clans (the prison escape is not an objective slot) |
| 18 | find the hidden sanctuary and reactivate it | only "eliminate non-allied clans" is in the table; the briefing's sanctuary is not an objective slot (unexplained, see below) |
| 19 | sanctuary in the south-east | sanctuary in z0 (x223-255, y226-255) |
| 20 | sanctuary in the south-east | sanctuary in z0 (x232-255, y209-255) |
| 21 | sanctuary in the south-east, destroy all clans | sanctuary in z0 (x231-255, y214-255). **Destroying the clans is not checked** |
| 22 | sanctuary in the south-east, destroy all clans | sanctuary in z0 (x196-255, y223-255) or z1 (x220-255, y178-205); eliminate non-allied clans; `food >= 0` |
| 23 | conquer the Great Library in the south-east | library in z0 (x246-254, y229-238) |
| 24 | sanctuary in the south-east | sanctuary in z0 (x236-255, y237-255) |
| 25 | (no briefing text) | eliminate non-allied clans |

## Victory, defeat and the 100-tick delay

`FUN_00458606` returns "done" and the game loop (in-game screen state, `0x42cb..`) counts the ticks on which it did
(`0x4e061c`, reset only when a mission loads, so the ticks **add up, they need not be consecutive**). At 100 (about 3.3 s at
30 Hz) it switches to screen `0x13`. That screen shows the **victory** flow (Enter advances the campaign, `FUN_0047fa88`) if the
flag `0x4e1d84` is set and plays the **Game Over** movie (`events\Over_fr.avi`) if it is clear.

* `0x4e1d84` is set the moment every objective holds, and cleared again on a few other screens (0x11, 0x17) and at load.
* **`0x4e1a3a` is the number of the human clan's units still alive**, recounted every tick by `FUN_00456db5`: slots of the
  unit table (`0x4e19a0`, 0x114 bytes) whose state (+2) is neither 0 nor 4, owner = the human clan, type not 0x11 (`SUN`).
  It is `0xffff` while a mission loads. `0x4e1a38` is the same count for the other clans.
* When `0x4e1a3a` reaches **0**, the function reports "done" with `0x4e1d84` cleared, so after 100 ticks you get the **Game
  Over** movie. That is the defeat rule: **a clan with no units loses, whatever buildings it still has.** Objectives are
  not looked at in that case.

## Notes and open questions

* Missions with a technology slot (3, 7, 9, 11, 15, 16, 17) can be blocked by a clan that wins the fight first but never
  researched anything; research one thing early.
* Missions 21 and 14 show that the briefing can promise more than the table checks. Mission 18's briefing is about a hidden
  sanctuary but its table only has type 5; the sanctuary is probably handled by the mission script (events) rather than
  by this table, or the type 5 slot is satisfied from the start if no other clan has units. Not verified in play.
* Everything above is from the files and the disassembly; none of it was checked by playing, because the headless
  `--mission N` flag stalls at the saved-game menu in this environment.
