# How `rl-emu` runs RISING.EXE

`RISING.EXE` is a 32-bit Windows PE (Watcom C, flat model, no relocations needed: base `0x400000`).
It imports only 109 functions, so it is run on a software CPU (Unicorn/QEMU) with the imports
implemented on the host.

## Guest memory map

| range | use |
|---|---|
| `0x00400000..0x0052e000` | the image (`.text`, data, `.bss`; `.bss` has raw pointer 0 and must stay zero) |
| `0x0f000000..0x0f100000` | stack |
| `0x10000000..0x30000000` | guest heap (first-fit allocator with free list, see `State::alloc`) |
| `0x7e000000..` | GDT and a minimal TEB (`fs:[0]` is the SEH chain, Watcom's start-up reads it) |
| `0x7f000000..` | import stubs (`ret N`), the callback trampoline and COM method stubs |

## Calling the host

Every import slot points at a 16-byte stub in `0x7f000000` that is `ret N` (stdcall) or `ret`
(cdecl). A code hook on that region runs the host function with the arguments read from the guest
stack; because the stub then executes its own `ret N`, stack cleanup is done by the guest. COM
objects (DirectDraw, DirectSound, DirectPlay) are pointers to vtables whose slots are more stubs;
`com.rs` looks the method up by stub address. **Argument counts must be exact** (a wrong `ret N`
corrupts the guest stack: DirectSound `Lock` is 8 words including `this`).

Control transfers (calling a window procedure or an enumeration callback, exiting) are not done
inside the hook. The handler stores an `Action` and stops the CPU; `Machine::step` performs it:
it pushes the arguments and the trampoline address, runs the guest function, and when the
trampoline is hit resumes the interrupted stub (so its `ret N` runs with `EAX` set as wanted).
`Action::Call` can carry a list of further calls (used for enumeration callbacks) and temporary
guest memory to free at the end.

The CPU is never stopped by timeouts (a timeout could land between a stub's hook and its `ret` and
lose the handler's result, which crashed multiplayer). Instead `PeekMessage`/`GetMessage` yield to
the host deterministically (events, frame pacing), once per game loop pass.

## Frame pacing

The game polls messages once per loop pass. The first `PeekMessage` that finds no message waits
until the next frame deadline (`RL_FPS`, default 30), which is what fixes the uncapped loop.

## Win32 behaviours worth knowing

* `GetPrivateProfileInt/String` read `RISING.INI` for real (unit and building statistics).
* `C:` is the install directory with a per-user overlay for anything written; `D:` is the disc.
  `GetDriveTypeA` reports `D:` as a CD-ROM and the volume label is `RISING_LANDS_BR_`.
* `PeekMessage` honours `PM_NOREMOVE`; the game peeks and then calls `GetMessage`.
* The mouse position the game polls is always clamped to the screen.
* Movies: the first menu click plays `EVENTS\INTROFR.AVI` through MCI (open, window, play, status
  polling, close). Escape skips it. The host decodes Cinepak + MS ADPCM itself (`video.rs`).
* CD music: MCI strings (`open ... cdaudio`, `play cdrud from N`, `status ... length track N`) are
  served from `trackNN.raw` in the disc root.
* Sound volumes (F10, `options.rs`): the game has three sound banks in `INSTALHD\SOUND`: `P` (unit sounds, keyed by
  unit type and class), `B` (building noises) and `A` (the rest). Its sound loader (`0x48aad4`, EDX = bank struct, `P` at
  `0x522000`, EBX = the channel record, which starts with the request key `type << 16 | class << 8`) runs right before it
  creates the DirectSound buffer, so a code hook tags the next buffer (`audio::Kind`) and the mixer scales each kind.
  Speech = bank `P`; the builder's hammer = `P` type 7 class `0x25` (found with `RL_AUDIO_LOG`); everything else is
  an effect. The CD music has its own volume. `RL_AUDIO_LOG=1` logs each new request (once per bank and key, on
  stderr; `RL_AUDIO_LOG=2` adds the per-buffer mixer lines), which is how to tell more sounds apart.

## Multiplayer

DirectPlay is replaced by `net.rs`: TCP star topology with `TCP_NODELAY`, UDP discovery on port
47624 (LAN broadcast, `RL_JOIN` addresses). `dplay.rs` implements the IDirectPlay2/3 methods the
game uses (EnumSessions, EnumPlayers, Open, CreatePlayer, GetSessionDesc, Receive, Send, ...), and
turns joins and leaves into `DPMSG_CREATEPLAYERORGROUP` / `DPMSG_DESTROYPLAYERORGROUP` messages.
