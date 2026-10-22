# Rising Lands (Microïds, 1998) on modern systems

This project runs the original 1998 game on current hardware (Linux, Windows, macOS; x86 and ARM)
and fixes problems of the original. It does **not** contain any game data: you need your own copy
of the game disc.

## How it works

`RISING.EXE` is a 32-bit Windows program that only needs about a hundred Windows and DirectX
functions. `rl-emu` runs its machine code on a CPU emulator (Unicorn) and provides those functions
itself: window and input through SDL2, DirectDraw (the 800x600 8-bit screen), DirectSound, CD audio,
and AVI movies (Cinepak + MS ADPCM). Because the original game logic runs unchanged, the campaign,
AI and rules are exactly the original ones.

Fixes live in the host side or as small patches:

* **Frame rate**: the original game loop had no limiter and ran as fast as the CPU allowed. The host
  now holds the loop to `RL_FPS` frames per second (default 30).
* Saved games and other writable files go to a per-user directory instead of the install folder.

## Quick start

```sh
# 1. Extract the disc image (BIN/CUE, MODE2/2352) into a folder
cargo run --release -p rl-tool -- disc RISING_LANDS_BR_.BIN original

# 2. Play (INSTALHD is the game folder, `original` is the disc root)
cargo run --release -p rl-emu -- original/INSTALHD original
```

The disc layout is `INSTALHD/` (the game), `WORLD/` and `EVENTS/` (levels and movies) and the CD
audio tracks as `track02.raw`, `track03.raw` (raw 44.1 kHz 16-bit stereo; extract them from the
CUE with any tool).

## Options (environment variables)

| variable | meaning |
|---|---|
| `RL_FPS` | frames per second the game loop is held to (default 30) |
| `RL_USERDIR` | where saved games go (default: the platform's per-user data directory) |
| `RL_LOG_CALLS` | log every Windows/DirectX call the game makes (debugging) |
| `RL_SCRIPT`, `RL_SHOT`, `RL_SECS` | headless runs: scripted input and PNG screenshots, no window |

Headless script example (clicks "Um jogador" and saves a screenshot):

```sh
RL_SCRIPT="20:move:400,296;22:click:400,296;30:shot:/tmp/menu.png;31:quit" \
  target/release/rl-emu original/INSTALHD original
```

## Other tools

* `rl-assets`: decoders for the game's file formats (sprites, screens, maps, rules). See
  `docs/FORMATS.md`.
* `rl-tool`: extracts the disc, converts graphics to PNG, renders maps.
* `rl-game`: a small map viewer built on `rl-assets`.
