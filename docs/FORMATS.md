# Rising Lands file formats (reverse-engineered)

Status legend: **done** = decoder in `rl-assets` and verified against every file on the disc.

## Disc
`RISING_LANDS_BR_.BIN/.CUE`: track 1 is `MODE2/2352` ISO 9660 (data in each sector at offset 24, 2048 bytes);
tracks 2-3 are CD audio. `rl-tool disc` extracts it (skips `DIRECTX/`). The ISO has both primary (upper-case
8.3) and Joliet names; we read the primary ones, so the engine must resolve paths **case-insensitively**
(the exe asks for e.g. `\Graphics\Blocs.cmp`).

## Global game palette — done
There is one 256-colour palette for the whole game: the `CMAP` chunk of `CHARGE.LZS`.
Index 3 is magenta `(255,0,255)` and is used as an in-art colour key in some tiles.
The palette embedded in `.PIE` files is **not** used for display.

## `.CMP` sprite archives (`INSTALHD/GRAPHICS/`) — done
`u16 count`, then `count` x `{u16 w, u16 h, u32 size, u8 data[size]}`. Little-endian.
Data is a token stream over a `w*h` row-major raster (tokens may wrap rows):
`b < 0x80` skip `b` transparent pixels; `b >= 0x80` copy next `256-b` bytes as palette indices.
Sprite counts: BLOCS 1345 (32x32 terrain tiles), BUILDING 611, CREATURE 1920, INVENTE 72, MENU 40,
OTHERS 4517, TROOPS 2848, VEHICULE 915. Sprites have no hotspot/offset field in the file;
animation frame grouping lives in the exe tables (TBD).

## `.PIE` screens — done
`"Pie"`, `u16 w`, `u16 h`, `u16 ncolors(=256)`, `768` bytes RGB palette (ignored), then PackBits
(`n<0x80`: copy n+1 literals; else repeat next byte `257-n` times). All are 800x600.
`TITRE.PIE` == `MENUPRIN.PIE`.

## `.LZS` screens — done
`"LZSS"`, `u16 w`, `u16 h`, `"CMAP"`, 768 byte RGB palette, `"FORM"`, `u32 len` (LE), then Okumura LZSS
(4096 window, initial write pos 4078, flags LSB-first, 2-byte ref = 12-bit offset / 4-bit len-3).

## `.ETA` scenario maps — terrain done, rest TBD
Loader is `FUN_0046c2cf` (the savegame loader `FUN_0046b88d` is a different layout: `"SAV"`+version digit).
Map is 256x256 cells; the playable area sits in the top-left corner (mission 1 is ~70x65), the rest is filler.
Offsets: `0x00000` 64K byte layer (first row overwritten by `"V0.3"`), `0x10000` 128K `u16` BLOCS tile index
(row-major, 32x32 px per cell, top-down — not isometric), `0x30000` 64K ASCII terrain class (loader maps
`'6'`->`'B'` and sets the matching byte in layer 3 to 1), `0x40000` 64K layer 3. After 0x50000: per-clan
records, buildings, units (TBD). Render with `rl-tool map`. Tiles drawn magenta / labelled "TEMP" are
markers (editor/special objects), not terrain.
Savegame (`FUN_0046b88d`/`FUN_0046ac02`) order: 4x[64K,64K,64K], 128K, 64K, 64K, 64K, 3x16K, 80 bytes.

## TBD
`.SND/.HED` (sound banks), `RISING.INI/.RUL/.SBL`, `EVENTS.INI`, `FONTE.FNT`,
`.FLC` (standard Autodesk FLC) and `.AVI` (use ffmpeg / a Rust decoder).

## Tooling notes
`RISING.EXE` is a Watcom-compiled Win32 binary (sections `BEGTEXT`/`DGROUP`), register calling convention,
which makes Ghidra's decompiler noisy. Ghidra 12.1 + JDK 21 live in `tools/` (gitignored);
`tools/scripts/ExportDecomp.java` dumps all functions to `tools/rising_decomp.c`.

## Rule files (`RISING.INI`, `MONSTRE.INI`, `EVENTS.INI`)

Latin-1 text, `[SECTION]` + `KEY=v1,v2,...`, `;` comments (French). `RISING.INI` holds
per-unit stats (`DEGAT` = 27 damage values indexed by target unit type, `RAYON`, `PORTEE`,
`FATIGUE`, `RECHARGE`, `TMPSRECRUTE`, `COUT*`, `BLINDAGE`, `NAME` = localized name),
buildings, spells and research (`AGRICULTURE0..18`, `MILITAIRE0..`). Parsed by `rl_assets::ini`.
`RISING.RUL` / `RISING.SBL` are binary and still TBD.
