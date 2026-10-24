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

## Requirements

Rust, a C compiler and CMake (Unicorn is built from source). On Linux also the SDL2 development
package, e.g. `sudo apt install libsdl2-dev`; the system SDL is needed for sound (a bundled SDL has
no PulseAudio/PipeWire/ALSA backend). Windows and macOS build SDL themselves.

## Quick start

```sh
# 1. Extract the disc image (BIN/CUE, MODE2/2352) into a folder: game files, levels, movies
#    and the CD audio tracks (track02.raw, track03.raw)
cargo run --release -p rl-tool -- disc RISING_LANDS_BR_.BIN original

# 2. Play (defaults to ./original/INSTALHD and ./original)
cargo run --release -p rl-emu
cargo run --release -p rl-emu -- --skip-intro   # no movies
cargo run --release -p rl-emu -- --new-game     # no movies, clicks through the menus into mission 1
cargo run --release -p rl-emu -- --mission 5    # same, then jumps to mission 5 (also `--map WORLD/MONDE05.ETA`); takes ~2 minutes
```

Keys: the game's own; `Alt+Enter` toggles fullscreen, `F11` switches 800x600 / 640x480 (the
game's own key), `F1`-`F4` set the game speed to 1x / 1.5x / 2x / 3x (the original used them to skip frames), `F6` / `F5` raise / lower it by 1x, `F12` opens the debug console (see docs/DEBUG.md), `Ctrl +` / `Ctrl -` change the base frame rate (steps of 5), `Esc` skips a
movie. The first menu click plays the intro movie; press `Esc`.

## Multiplayer

The original DirectPlay connection is replaced by a direct TCP connection:

* **Host**: choose *Vários jogadores* → the TCP/IP provider → *Iniciar uma sessão*. The game tells
  you your LAN address and, if your router supports UPnP, your public address (the port is
  forwarded automatically; set `RL_NO_UPNP=1` to prevent that).
* **Join**: *Vários jogadores* → provider → *Entrar numa sessão*. Hosts on the same LAN show up
  automatically. Otherwise a box asks for the host's address (`ip` or `ip:port`, TCP/UDP 47624).
  `RL_JOIN=ip[,ip...]` pre-fills addresses to look at.
* Messages are sent with `TCP_NODELAY`, without DirectPlay's per-message acknowledgements, which
  is what made the original so slow.

## Options (environment variables)

| variable | meaning |
|---|---|
| `RL_FPS` | frames per second the game loop is held to (default 30) |
| `RL_WASD` | start with WASD scrolling the map (`Alt+W` toggles it in game) |
| `RL_GRAB` | keep the mouse inside the window |
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
