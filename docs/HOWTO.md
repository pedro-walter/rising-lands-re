# How to play Rising Lands from the archive.org disc image

This repository contains **no game data**. You supply the disc image; the tools here extract it and
run the original `RISING.EXE` on a CPU emulator. Rising Lands is still the property of its
rights holders (Microïds). Check that you may use a copy where you live; the project neither
ships nor hosts the game.

## 1. Get the disc image

The Portuguese (Brazil) release is preserved at <https://archive.org/details/risinglandsbr>.
Despite the usual "ISO" name, the item is a **BIN/CUE pair**, not an `.iso`. You need both files:

| file | size (bytes) | SHA-1 |
|---|---|---|
| `RISING_LANDS_BR_.BIN` | 705418896 | `801e7a9bd00954c281f23767568222b23f3c4981` |
| `RISING_LANDS_BR_.CUE` | 210 | `00b9fcafc786e7e8bab915bcbe98f1601ccc1a2f` |

```sh
mkdir -p ~/rising-lands && cd ~/rising-lands
curl -LO https://archive.org/download/risinglandsbr/RISING_LANDS_BR_.BIN
curl -LO https://archive.org/download/risinglandsbr/RISING_LANDS_BR_.CUE
sha1sum RISING_LANDS_BR_.BIN RISING_LANDS_BR_.CUE      # macOS: shasum -a 1; Windows: certutil -hashfile <file> SHA1
```

Keep the two files side by side with the same base name: `rl-tool` finds the CUE next to the BIN
to cut out the CD audio tracks.

The disc is a Mode 2/2352 data track (the game files) followed by two audio tracks (the music).
Mounting it normally gives you only the data track, so use `rl-tool` below rather than copying
files by hand; otherwise the music is missing.

If you only have an `.iso` from somewhere else, it will not work: a plain 2048-byte-sector ISO has
no audio tracks and is not the raw 2352-byte layout `rl-tool` reads.

## 2. Install the build requirements

* **Rust** (stable): <https://rustup.rs>
* **A C compiler and CMake**: Unicorn, the CPU emulator, is built from source.
* **Linux only**: the SDL2 development package. The system SDL is needed for sound.
  * Debian/Ubuntu: `sudo apt install build-essential cmake libsdl2-dev`
  * Fedora: `sudo dnf install gcc gcc-c++ cmake SDL2-devel`
  * Arch: `sudo pacman -S base-devel cmake sdl2`
* **Windows**: Visual Studio Build Tools (C++ workload) and CMake. SDL is built for you.
* **macOS**: `xcode-select --install` and `brew install cmake`. SDL is built for you.

## 3. Build

```sh
git clone https://github.com/pedro-walter/rising-lands-re.git
cd rising-lands-re
cargo build --release -p rl-tool -p rl-emu
```

The first build compiles Unicorn and takes several minutes.

## 4. Extract the disc

From the repository folder (adjust the path to where you saved the BIN):

```sh
cargo run --release -p rl-tool -- disc ~/rising-lands/RISING_LANDS_BR_.BIN original
```

This creates `./original/`, which is git-ignored:

```
original/
  INSTALHD/      RISING.EXE, RISING.INI, RISING.RUL, sprites, screens, sounds
  WORLD/         the mission maps
  EVENTS/
  FONTE.FNT, TICKONE.WAV, manual.txt
  track02.raw    CD audio (music)
  track03.raw
```

You should see `extracted N files` and `extracted 2 audio tracks`. The DirectX redistributable
folder on the disc is skipped on purpose; it is not needed.

## 5. Play

```sh
cargo run --release -p rl-emu
```

With no arguments it uses `./original/INSTALHD` and `./original`. Other launches:

```sh
cargo run --release -p rl-emu -- --skip-intro    # no movies
cargo run --release -p rl-emu -- --new-game      # skip the menus, start mission 1
cargo run --release -p rl-emu -- --mission 5     # jump to mission 5 (about 2 minutes)
```

Useful keys: `Alt+Enter` full screen, `F10` options (music, effects, voices, edge scrolling),
`F1`-`F3` game speed, `Esc` skips a movie, `F12` debug console. The README lists all of them, plus
the multiplayer setup and the `RL_*` environment variables.

Saved games go to your per-user data directory (or `RL_USERDIR`), never into `original/`.

## 6. If something goes wrong

| symptom | cause and fix |
|---|---|
| `error: sector beyond image` or no files extracted | The BIN is truncated. Compare its size and SHA-1 with the table above and download again. |
| `extracted 0 audio tracks` / no music | The CUE is missing or not next to the BIN with the same base name. |
| No sound at all (Linux) | The bundled SDL has no PulseAudio/PipeWire/ALSA backend. Install `libsdl2-dev` (step 2), then do a clean rebuild (`cargo clean`, then step 3). |
| CMake or compiler errors while building Unicorn | Install CMake and a C compiler (step 2). |
| `warning: building accessor patch not applied (different RISING.EXE?)` | You are using another release of the game. It still runs, but the one crash fix is skipped. This project targets the archive.org PT-BR image. |
| The game runs too fast or too slow | Set `RL_FPS` (default 30), or press `Ctrl +` / `Ctrl -` in game. |
| Opening menu does nothing visible | The first menu click plays the intro movie; press `Esc`. |

## 7. Optional: check your setup

```sh
cargo test --workspace                       # needs no game data
RL_DATA=original/INSTALHD cargo test -p rl-assets   # decodes every real asset in your copy
```

The second command is a good proof that the extraction is complete.
