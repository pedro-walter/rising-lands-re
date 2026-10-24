//! Guest machine: Unicorn CPU, memory map, import stubs and the run loop.
//!
//! Imported functions are replaced by tiny `ret N` stubs. A code hook on the stub
//! region runs the host implementation, so stdcall cleanup happens in guest code.
//! Anything that must transfer control (calling a window procedure, exiting) is
//! recorded as a [`Action`], the CPU is stopped, and the driver in [`Machine::run`]
//! performs it and resumes.

use crate::{com, pe, vfs::Vfs, win32};
use std::{
    collections::{HashMap, VecDeque},
    error::Error,
    path::Path,
    time::Instant,
};
use unicorn_engine::{uc_error, Arch, HookType, Mode, Prot, RegisterX86, Unicorn};

pub const STUB_BASE: u32 = 0x7f00_0000;
pub const STUB_SIZE: u32 = 0x10000;
/// Return address pushed for guest callbacks; hitting it resumes the interrupted stub.
pub const TRAMP: u32 = STUB_BASE;
const STACK_TOP: u32 = 0x0f10_0000;
const STACK_SIZE: u32 = 0x0010_0000;
/// Guest addresses in the original code: where a sound is loaded into a buffer, and where one is requested.
const SOUND_LOAD: u32 = 0x48aad4;
const SOUND_REQUEST: u32 = 0x48aa79;
pub const HEAP_BASE: u32 = 0x1000_0000;
const HEAP_SIZE: u32 = 0x2000_0000;

pub type Uc<'a> = Unicorn<'a, State>;

pub struct Stub {
    pub name: String,
}

pub enum Action {
    /// Run a guest function, then resume the stub that asked for it.
    Call { target: u32, args: Vec<u32>, resume: u32, set_eax: Option<u32>, rest: Vec<(u32, Vec<u32>)>, temps: Vec<u32> },
    /// Resume execution at `pc` (return from a callback).
    Resume { pc: u32, set_eax: Option<u32> },
    Exit(u32),
}

/// A guest callback in progress: what to run next and where to resume afterwards.
pub struct Frame {
    pub resume: u32,
    pub set_eax: Option<u32>,
    pub rest: Vec<(u32, Vec<u32>)>,
    /// Guest memory to free once the whole sequence is done.
    pub temps: Vec<u32>,
}

impl Action {
    /// Runs one guest function and then resumes the stub at `resume`.
    pub fn call(target: u32, args: Vec<u32>, resume: u32, set_eax: Option<u32>) -> Action {
        Action::Call { target, args, resume, set_eax, rest: Vec::new(), temps: Vec::new() }
    }
}

pub struct Msg {
    pub hwnd: u32,
    pub message: u32,
    pub wparam: u32,
    pub lparam: u32,
}

pub struct Window {
    pub proc_: u32,
}

pub struct State {
    pub stubs: Vec<Stub>,
    pub heap_next: u32,
    pub heap_sizes: HashMap<u32, u32>,
    pub heap_free: std::collections::BTreeMap<u32, u32>,
    /// Freed blocks wait here before they can be reused: the game touches some memory after
    /// freeing it, which Windows 9x tolerated.
    pub heap_quarantine: std::collections::VecDeque<(u32, u32)>,
    pub heap_quarantine_bytes: u64,
    pub pending: Option<Action>,
    pub skip_once: Option<u32>,
    /// Stub whose host handler just ran; a timeout can stop the CPU before the stub's `ret`.
    pub last_hooked: Option<u32>,
    pub frames: Vec<Frame>,
    pub vfs: Vfs,
    pub classes: HashMap<String, u32>,
    pub windows: Vec<Window>,
    pub msgs: VecDeque<Msg>,
    pub quit: bool,
    pub start: Instant,
    pub tls: Vec<u32>,
    pub last_error: u32,
    pub com: com::Com,
    pub ui: crate::frontend::Ui,
    /// AVI opened through MCI, and the movie currently playing (if any).
    pub video_open: Option<crate::video::Avi>,
    pub video: Option<crate::video::Player>,
    pub mixer: crate::audio::Shared,
    /// Bank of the sound the game is about to buffer (set by the hook in [`Machine::new`]).
    pub sound_kind: crate::audio::Kind,
    /// Track files for CD audio (`trackNN.raw` in the disc root).
    pub cd_tracks: HashMap<u32, std::sync::Arc<Vec<u8>>>,
    pub cd_format_tmsf: bool,
    pub finds: HashMap<u32, crate::vfs::Find>,
    pub next_handle: u32,
    pub cursor: (i32, i32),
    pub keys: [bool; 256],
    /// Minimum host time between two game-loop passes (the original loop was uncapped).
    pub frame_interval: std::time::Duration,
    /// F1-F4 game speed multiplier applied on top of `frame_interval`.
    pub speed: f64,
    /// Set while the click flag has been seen (see `scroll_click_assist`).
    pub click_aligned: bool,
    /// `--skip-intro`: the game's movies are reported as missing so it moves on at once.
    pub skip_movies: bool,
    /// Debug console: finish buildings the moment they are placed.
    pub instant_build: bool,
    pub next_frame: Instant,
    pub log_calls: bool,
    pub strings: HashMap<&'static str, u32>,
    pub api_counts: HashMap<String, u64>,
    pub ini: HashMap<String, String>,
}

impl State {
    pub fn new(install: &Path, disc: &Path, userdir: &Path) -> State {
        State {
            stubs: Vec::new(),
            heap_next: HEAP_BASE,
            heap_sizes: HashMap::new(),
            heap_free: Default::default(),
            heap_quarantine: Default::default(),
            heap_quarantine_bytes: 0,
            pending: None,
            skip_once: None,
            last_hooked: None,
            frames: Vec::new(),
            vfs: Vfs::new(install.to_path_buf(), disc.to_path_buf(), userdir.to_path_buf()),
            classes: HashMap::new(),
            windows: Vec::new(),
            msgs: VecDeque::new(),
            quit: false,
            start: Instant::now(),
            tls: vec![0; 64],
            last_error: 0,
            com: com::Com::default(),
            ui: Default::default(),
            video_open: None,
            video: None,
            mixer: Default::default(),
            sound_kind: Default::default(),
            cd_tracks: HashMap::new(),
            cd_format_tmsf: false,
            finds: HashMap::new(),
            next_handle: 0x2000,
            cursor: (0, 0),
            keys: [false; 256],
            frame_interval: std::time::Duration::from_secs_f64(
                1.0 / std::env::var("RL_FPS").ok().and_then(|v| v.parse::<f64>().ok()).filter(|f| *f > 0.0).unwrap_or(30.0),
            ),
            speed: 1.0,
            click_aligned: false,
            skip_movies: false,
            instant_build: false,
            next_frame: Instant::now(),
            log_calls: std::env::var_os("RL_LOG_CALLS").is_some(),
            strings: HashMap::new(),
            api_counts: HashMap::new(),
            ini: HashMap::new(),
        }
    }

    /// First-fit allocation from the guest heap (the memory may be dirty; see [`zeroed_alloc`]).
    pub fn alloc(&mut self, size: u32) -> u32 {
        let size = (size.max(1) + 15) & !15;
        let hit = self.heap_free.iter().find(|(_, &sz)| sz >= size).map(|(&a, &sz)| (a, sz));
        let addr = if let Some((a, sz)) = hit {
            self.heap_free.remove(&a);
            if sz > size {
                self.heap_free.insert(a + size, sz - size);
            }
            a
        } else {
            let a = self.heap_next;
            self.heap_next = a + size;
            assert!(self.heap_next < HEAP_BASE + HEAP_SIZE, "guest heap exhausted");
            a
        };
        self.heap_sizes.insert(addr, size);
        addr
    }

    pub fn free(&mut self, addr: u32) {
        if std::env::var_os("RL_NOFREE").is_some() {
            return;
        }
        let Some(size) = self.heap_sizes.remove(&addr) else { return };
        self.heap_quarantine.push_back((addr, size));
        self.heap_quarantine_bytes += size as u64;
        // Release the oldest blocks once 64 MB are waiting.
        while self.heap_quarantine_bytes > 64 << 20 {
            let Some((a, sz)) = self.heap_quarantine.pop_front() else { break };
            self.heap_quarantine_bytes -= sz as u64;
            self.release(a, sz);
        }
    }

    fn release(&mut self, addr: u32, size: u32) {
        let (mut start, mut len) = (addr, size);
        if let Some((&pa, &ps)) = self.heap_free.range(..addr).next_back() {
            if pa + ps == addr {
                self.heap_free.remove(&pa);
                start = pa;
                len += ps;
            }
        }
        if let Some(&ns) = self.heap_free.get(&(addr + size)) {
            self.heap_free.remove(&(addr + size));
            len += ns;
        }
        self.heap_free.insert(start, len);
    }

    pub fn new_handle(&mut self) -> u32 {
        let h = self.next_handle;
        self.next_handle += 4;
        h
    }
}

pub struct Machine {
    pub uc: Uc<'static>,
    pc: u32,
}

/// Allocates guest memory and clears it.
pub fn zeroed_alloc(uc: &mut Uc, size: u32) -> u32 {
    let a = uc.get_data_mut().alloc(size);
    let n = uc.get_data().heap_sizes[&a];
    wr_bytes(uc, a, &vec![0u8; n as usize]);
    a
}

pub fn rd32(uc: &Uc, addr: u32) -> u32 {
    let mut b = [0u8; 4];
    uc.mem_read(addr as u64, &mut b).unwrap_or_else(|e| panic!("read32 {addr:#x}: {e:?}"));
    u32::from_le_bytes(b)
}

pub fn wr32(uc: &mut Uc, addr: u32, v: u32) {
    uc.mem_write(addr as u64, &v.to_le_bytes()).unwrap_or_else(|e| panic!("write32 {addr:#x}: {e:?}"));
}

#[cfg(test)]
pub fn wr16(uc: &mut Uc, addr: u32, v: u16) {
    uc.mem_write(addr as u64, &v.to_le_bytes()).unwrap_or_else(|e| panic!("write16 {addr:#x}: {e:?}"));
}

pub fn rd_bytes(uc: &Uc, addr: u32, n: usize) -> Vec<u8> {
    let mut b = vec![0u8; n];
    uc.mem_read(addr as u64, &mut b).unwrap_or_else(|e| panic!("read {addr:#x}+{n}: {e:?}"));
    b
}

pub fn wr_bytes(uc: &mut Uc, addr: u32, b: &[u8]) {
    uc.mem_write(addr as u64, b).unwrap_or_else(|e| panic!("write {addr:#x}+{}: {e:?}", b.len()));
}

pub fn rd_cstr(uc: &Uc, addr: u32) -> String {
    if addr == 0 {
        return String::new();
    }
    let mut out = Vec::new();
    let mut a = addr;
    loop {
        let mut b = [0u8; 1];
        if uc.mem_read(a as u64, &mut b).is_err() || b[0] == 0 || out.len() > 4096 {
            break;
        }
        out.push(b[0]);
        a += 1;
    }
    out.iter().map(|&c| c as char).collect()
}

pub fn reg(uc: &Uc, r: RegisterX86) -> u32 {
    uc.reg_read(r).unwrap() as u32
}

pub fn set_reg(uc: &mut Uc, r: RegisterX86, v: u32) {
    uc.reg_write(r, v as u64).unwrap();
}

/// Writes a NUL-terminated string into a guest buffer of `cap` bytes.
pub fn wr_cstr(uc: &mut Uc, addr: u32, s: &str, cap: usize) -> usize {
    let bytes: Vec<u8> = s.chars().map(|c| c as u32 as u8).take(cap.saturating_sub(1)).collect();
    wr_bytes(uc, addr, &bytes);
    wr_bytes(uc, addr + bytes.len() as u32, &[0]);
    bytes.len()
}

/// Fixes to the game's own code. Each one is only applied when the original bytes are there.
fn apply_patches(uc: &mut Uc) {
    // `FUN_0041a4f4(building)` returns `buildings[building].kind` (stride 0x114 at 0x49a0c8) with
    // no range check. The "building under the cursor" word is 0xffff when nothing is there, so
    // the thief's steal action read far outside the table and crashed when the pointer moved over
    // anything that is not a building. Out of range now answers 0xff, which no caller matches.
    const ORIG: [u8; 8] = [0x52, 0x31, 0xd2, 0x66, 0x89, 0xc2, 0x89, 0xd0];
    const NEW: [u8; 31] = [
        0x0f, 0xb7, 0xc0, // movzx eax, ax
        0x3d, 0xe8, 0x03, 0x00, 0x00, // cmp eax, 1000
        0x73, 0x0d, // jae bad
        0x69, 0xc0, 0x14, 0x01, 0x00, 0x00, // imul eax, eax, 0x114
        0x8a, 0x80, 0xd0, 0xa0, 0x49, 0x00, // mov al, [eax+0x49a0d0]
        0xc3, // ret
        0xb0, 0xff, // bad: mov al, 0xff
        0xc3, // ret
        0x90, 0x90, 0x90, 0x90, 0x90,
    ];
    let mut cur = [0u8; 8];
    if uc.mem_read(0x41a4f4, &mut cur).is_ok() && cur == ORIG {
        wr_bytes(uc, 0x41a4f4, &NEW);
    } else {
        eprintln!("warning: building accessor patch not applied (different RISING.EXE?)");
    }
}

impl Machine {
    pub fn new(exe: &Path, install: &Path, disc: &Path, userdir: &Path) -> Result<Machine, Box<dyn Error>> {
        let bytes = std::fs::read(exe)?;
        let img = pe::load(&bytes)?;
        let state = State::new(install, disc, userdir);
        let mut uc = Unicorn::new_with_data(Arch::X86, Mode::MODE_32, state).map_err(|e| format!("unicorn: {e:?}"))?;
        let map = |uc: &mut Uc, a: u32, s: u32| uc.mem_map(a as u64, s as u64, Prot::ALL).map_err(|e| format!("map {a:#x}: {e:?}"));
        map(&mut uc, img.base, (img.size + 0xfff) & !0xfff)?;
        for (a, d) in &img.chunks {
            wr_bytes(&mut uc, *a, d);
        }
        apply_patches(&mut uc);
        // The building table (0x49a0c8, stride 0x114) is indexed by 16-bit ids that may be 0xffff
        // ("none") with no range check in dozens of accessors; zeroed pages up to the end of what
        // such an index can reach turn those reads into harmless zeros instead of a fault.
        let tail = (img.base + img.size + 0xfff) & !0xfff;
        map(&mut uc, tail, 0x0180_0000 - tail)?;
        map(&mut uc, STACK_TOP - STACK_SIZE, STACK_SIZE)?;
        map(&mut uc, HEAP_BASE, HEAP_SIZE)?;
        map(&mut uc, STUB_BASE, STUB_SIZE)?;
        wr_bytes(&mut uc, TRAMP, &[0xc3]);

        // One stub per import; `ret N` pops the stdcall arguments.
        let mut next = STUB_BASE + 16;
        for imp in &img.imports {
            let (bytes, cdecl) = win32::arg_bytes(&imp.dll, &imp.name);
            let code: Vec<u8> = if cdecl { vec![0xc3] } else { vec![0xc2, bytes as u8, (bytes >> 8) as u8] };
            wr_bytes(&mut uc, next, &code);
            wr32(&mut uc, imp.slot, next);
            let idx = uc.get_data().stubs.len();
            uc.get_data_mut().stubs.push(Stub { name: imp.name.clone() });
            assert_eq!(idx as u32 * 16 + STUB_BASE + 16, next);
            next += 16;
        }
        uc.get_data_mut().com.next_stub = next;
        // Flat GDT so guest code that saves and restores segment registers works.
        const GDT: u32 = 0x7e00_0000;
        const TEB: u32 = GDT + 0x2000;
        map(&mut uc, GDT, 0x4000)?;
        let desc = |base: u32, limit: u32, access: u64, flags: u64| -> u64 {
            (limit as u64 & 0xffff) | ((base as u64 & 0xff_ffff) << 16) | (access << 40) | (((limit as u64 >> 16) & 0xf) << 48) | (flags << 52) | ((base as u64 >> 24) << 56)
        };
        let descs = [0, desc(0, 0xfffff, 0x9a, 0xc), desc(0, 0xfffff, 0x92, 0xc), desc(TEB, 0xfff, 0x92, 0x4)];
        for (i, d) in descs.iter().enumerate() {
            wr_bytes(&mut uc, GDT + i as u32 * 8, &d.to_le_bytes());
        }
        // Minimal TEB: SEH chain end, stack bounds, self pointer.
        wr32(&mut uc, TEB, 0xffff_ffff);
        wr32(&mut uc, TEB + 4, STACK_TOP);
        wr32(&mut uc, TEB + 8, STACK_TOP - STACK_SIZE);
        wr32(&mut uc, TEB + 0x18, TEB);
        let mut mmr = [0u8; 24];
        mmr[8..16].copy_from_slice(&(GDT as u64).to_le_bytes());
        mmr[16..20].copy_from_slice(&0x1fu32.to_le_bytes());
        uc.reg_write_long(RegisterX86::GDTR, &mmr).map_err(|e| format!("gdtr: {e:?}"))?;
        for (r, sel) in [(RegisterX86::CS, 0x08u64), (RegisterX86::DS, 0x10), (RegisterX86::ES, 0x10), (RegisterX86::SS, 0x10), (RegisterX86::FS, 0x18), (RegisterX86::GS, 0x10)] {
            uc.reg_write(r, sel).map_err(|e| format!("seg {r:?}: {e:?}"))?;
        }
        set_reg(&mut uc, RegisterX86::ESP, STACK_TOP - 0x100);

        uc.add_code_hook(STUB_BASE as u64, (STUB_BASE + STUB_SIZE) as u64, |uc, addr, _| on_stub(uc, addr as u32))
            .map_err(|e| format!("hook: {e:?}"))?;
        // The game's sound loader (`FUN_0048aad4`, EDX = the bank, EBX = the channel record that starts with the
        // request key) runs right before it creates the buffer.
        uc.add_code_hook(SOUND_LOAD as u64, SOUND_LOAD as u64, |uc, _, _| {
            let key = rd32(uc, reg(uc, RegisterX86::EBX));
            let kind = crate::audio::Kind::of_sound(reg(uc, RegisterX86::EDX), key);
            uc.get_data_mut().sound_kind = kind;
        })
        .map_err(|e| format!("sound hook: {e:?}"))?;
        // RL_AUDIO_LOG also logs each new kind of sound request, once (`FUN_0048aa79`: EAX = bank letter, EDX = type << 16 | class << 8).
        if std::env::var_os("RL_AUDIO_LOG").is_some() {
            let mut seen = std::collections::HashSet::new();
            uc.add_code_hook(SOUND_REQUEST as u64, SOUND_REQUEST as u64, move |uc, _, _| {
                let (bank, key) = (reg(uc, RegisterX86::EAX) as u8 as char, reg(uc, RegisterX86::EDX));
                if seen.insert((bank, key)) {
                    eprintln!("audio: new sound request bank {bank} type {} class {:#x} (key {key:#x})", key >> 16, key >> 8 & 0xff);
                }
            })
            .map_err(|e| format!("sound hook: {e:?}"))?;
        }
        // RL_TRACE=<hex addr>[,<hex addr>...] logs the registers each time the guest reaches an address.
        if let Ok(spec) = std::env::var("RL_TRACE") {
            for a in spec.split(',').filter_map(|v| u32::from_str_radix(v.trim().trim_start_matches("0x"), 16).ok()) {
                uc.add_code_hook(a as u64, a as u64, move |uc, addr, _| {
                    let r = |x| reg(uc, x);
                    eprintln!("[trace] {addr:#x} eax={:#x} ebx={:#x} ecx={:#x} edx={:#x} esi={:#x} edi={:#x} esp={:#x}", r(RegisterX86::EAX), r(RegisterX86::EBX), r(RegisterX86::ECX), r(RegisterX86::EDX), r(RegisterX86::ESI), r(RegisterX86::EDI), r(RegisterX86::ESP));
                })
                .map_err(|e| format!("trace: {e:?}"))?;
            }
        }
        // RL_WATCH=<hex addr>[:<len>] logs guest writes to that range (debugging aid).
        if let Some(spec) = std::env::var("RL_WATCH").ok() {
            let (a, l) = spec.split_once(':').unwrap_or((&spec, "4"));
            if let (Ok(w), Ok(len)) = (u32::from_str_radix(a.trim_start_matches("0x"), 16), l.parse::<u32>()) {
                // RL_WATCH_MOD=<stride>:<offset> (decimal) only logs writes at that field of each stride-sized record.
                let modf: Option<(u64, u64)> = std::env::var("RL_WATCH_MOD").ok().and_then(|m| m.split_once(':').and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?))));
                uc.add_mem_hook(HookType::MEM_WRITE, w as u64, (w + len.max(1) - 1) as u64, move |uc, _, addr, size, val| {
                    if modf.is_some_and(|(st, off)| (addr - w as u64) % st != off) {
                        return true;
                    }
                    let esp = reg(uc, RegisterX86::ESP);
                    let stack: Vec<String> = (0..64).map(|i| rd32(uc, esp + i * 4)).filter(|v| (0x401000..0x494000).contains(v)).take(6).map(|v| format!("{v:#x}")).collect();
                    eprintln!("[watch] write {size} at {addr:#x} = {val:#x} from eip={:#x} stack: {}", reg(uc, RegisterX86::EIP), stack.join(" "));
                    true
                })
                .map_err(|e| format!("watch: {e:?}"))?;
            }
        }
        Ok(Machine { uc, pc: img.entry })
    }

    /// Runs the guest for up to `slice_us` microseconds of host time.
    /// Returns `Some(exit code)` once the guest has exited.
    pub fn step(&mut self, slice_us: u64) -> Result<Option<u32>, Box<dyn Error>> {
        let pc = self.pc;
        if let Err(e) = self.uc.emu_start(pc as u64, 0, slice_us, 0) {
            let eip = reg(&self.uc, RegisterX86::EIP);
            let regs: Vec<String> = [
                ("eax", RegisterX86::EAX),
                ("ebx", RegisterX86::EBX),
                ("ecx", RegisterX86::ECX),
                ("edx", RegisterX86::EDX),
                ("esi", RegisterX86::ESI),
                ("edi", RegisterX86::EDI),
                ("ebp", RegisterX86::EBP),
                ("esp", RegisterX86::ESP),
            ]
            .iter()
            .map(|(n, r)| format!("{n}={:#x}", reg(&self.uc, *r)))
            .collect();
            let esp = reg(&self.uc, RegisterX86::ESP);
            let stack: Vec<String> = (0..24).map(|i| rd32(&self.uc, esp + i * 4)).filter(|v| (0x401000..0x494000).contains(v)).map(|v| format!("{v:#x}")).collect();
            // RL_DUMP="4e3aec:16,5080a8:4" prints guest memory (hex) when the guest faults.
            if let Ok(spec) = std::env::var("RL_DUMP") {
                for part in spec.split(',') {
                    let (a, l) = part.split_once(':').unwrap_or((part, "16"));
                    // A leading '*' follows the pointer stored at the address first.
                    let (deref, a) = a.strip_prefix('*').map_or((false, a), |r| (true, r));
                    if let (Ok(mut a), Ok(l)) = (u32::from_str_radix(a.trim_start_matches("0x"), 16), l.parse::<usize>()) {
                        if deref {
                            a = rd32(&self.uc, a);
                        }
                        let mut b = vec![0u8; l];
                        let ok = self.uc.mem_read(a as u64, &mut b).is_ok();
                        eprintln!("  dump {a:#x}+{l}: {}{}", if ok { "" } else { "(unreadable) " }, b.iter().map(|x| format!("{x:02x}")).collect::<Vec<_>>().join(" "));
                    }
                }
            }
            if std::env::var_os("RL_STACKDUMP").is_some() {
                for i in 0..48 {
                    eprintln!("  [esp+{:#x}] = {:#x}", i * 4, rd32(&self.uc, esp + i * 4));
                }
            }
            return Err(format!("guest fault {e:?} at eip={eip:#x} {} | code addrs on stack: {}", regs.join(" "), stack.join(" ")).into());
        }
        match self.uc.get_data_mut().pending.take() {
            Some(Action::Exit(code)) => return Ok(Some(code)),
            Some(Action::Resume { pc: p, set_eax }) => {
                if let Some(v) = set_eax {
                    set_reg(&mut self.uc, RegisterX86::EAX, v);
                }
                self.uc.get_data_mut().skip_once = Some(p);
                self.pc = p;
            }
            Some(Action::Call { target, args, resume, set_eax, rest, temps }) => {
                let mut esp = reg(&self.uc, RegisterX86::ESP);
                for a in args.iter().rev() {
                    esp -= 4;
                    wr32(&mut self.uc, esp, *a);
                }
                esp -= 4;
                wr32(&mut self.uc, esp, TRAMP);
                set_reg(&mut self.uc, RegisterX86::ESP, esp);
                self.uc.get_data_mut().frames.push(Frame { resume, set_eax, rest, temps });
                self.pc = target;
            }
            // Timed out or stopped without an action: continue where the guest is.
            None => {
                let eip = reg(&self.uc, RegisterX86::EIP);
                if self.uc.get_data().last_hooked == Some(eip) {
                    // Stopped between the hook and the stub's `ret`: don't run the handler twice.
                    self.uc.get_data_mut().skip_once = Some(eip);
                }
                self.pc = eip;
            }
        }
        Ok(None)
    }
}

fn on_stub(uc: &mut Uc, addr: u32) {
    if uc.get_data().skip_once == Some(addr) {
        uc.get_data_mut().skip_once = None;
        return;
    }
    if addr == TRAMP {
        let mut fr = uc.get_data_mut().frames.pop().expect("callback return without a frame");
        if !fr.rest.is_empty() {
            let (target, args) = fr.rest.remove(0);
            uc.get_data_mut().pending = Some(Action::Call { target, args, resume: fr.resume, set_eax: fr.set_eax, rest: fr.rest, temps: fr.temps });
        } else {
            for t in fr.temps {
                uc.get_data_mut().free(t);
            }
            uc.get_data_mut().pending = Some(Action::Resume { pc: fr.resume, set_eax: fr.set_eax });
        }
        let _ = uc.emu_stop();
        return;
    }
    uc.get_data_mut().last_hooked = Some(addr);
    let esp = reg(uc, RegisterX86::ESP);
    let mut args = [0u32; 14];
    for (i, a) in args.iter_mut().enumerate() {
        let mut b = [0u8; 4];
        if uc.mem_read((esp + 4 + 4 * i as u32) as u64, &mut b).is_ok() {
            *a = u32::from_le_bytes(b);
        }
    }
    let idx = ((addr - STUB_BASE) / 16) as usize;
    let eax = if addr >= STUB_BASE + 16 && idx >= 1 && idx - 1 < uc.get_data().stubs.len() {
        win32::call_import(uc, idx - 1, addr, &args)
    } else {
        com::call_method(uc, addr, &args)
    };
    if let Some(v) = eax {
        set_reg(uc, RegisterX86::EAX, v);
    }
}

pub fn stop_with(uc: &mut Uc, a: Action) {
    uc.get_data_mut().pending = Some(a);
    let _ = uc.emu_stop();
}

#[allow(dead_code)]
pub fn err_name(e: uc_error) -> String {
    format!("{e:?}")
}

#[cfg(test)]
pub mod testing {
    use super::*;

    /// A machine running a tiny generated executable (see [`crate::pe::testing::exe`]).
    pub fn machine(code: &[u8], imports: &[&str]) -> Machine {
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!("rl-machine-{}-{}", std::process::id(), NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("TEST.EXE");
        std::fs::write(&exe, crate::pe::testing::exe(code, imports)).unwrap();
        Machine::new(&exe, &dir, &dir, &dir).unwrap()
    }

    /// Runs until the guest exits, with a bound on the number of time slices.
    pub fn run_to_exit(m: &mut Machine) -> Result<u32, String> {
        for _ in 0..1000 {
            match m.step(200_000) {
                Ok(Some(code)) => return Ok(code),
                Ok(None) => {}
                Err(e) => return Err(e.to_string()),
            }
        }
        Err("did not exit".into())
    }

    /// A bare guest: 2 MiB at the PE base (code/data), a stack, and a user dir that does not exist.
    pub fn bare_uc() -> Uc<'static> {
        let dir = std::env::temp_dir().join("rl-emu-bare");
        uc_in(&dir, &dir, &dir)
    }

    /// Like `bare_uc`, with the guest's `C:` (install), `D:` (disc) and per-user overlay on real directories.
    pub fn uc_in(install: &Path, disc: &Path, user: &Path) -> Uc<'static> {
        let mut uc = Unicorn::new_with_data(Arch::X86, Mode::MODE_32, State::new(install, disc, user)).unwrap();
        uc.mem_map(0x40_0000, 0x20_0000, Prot::ALL).unwrap();
        uc.mem_map((STACK_TOP - STACK_SIZE) as u64, STACK_SIZE as u64, Prot::ALL).unwrap();
        set_reg(&mut uc, RegisterX86::ESP, STACK_TOP - 0x1000);
        uc
    }

    /// `bare_uc` plus a guest heap and room for COM method stubs.
    pub fn com_uc() -> Uc<'static> {
        with_heap(bare_uc())
    }

    pub fn with_heap(mut uc: Uc<'static>) -> Uc<'static> {
        uc.mem_map(HEAP_BASE as u64, 0x80_0000, Prot::ALL).unwrap();
        uc.mem_map(STUB_BASE as u64, 0x1_0000, Prot::ALL).unwrap();
        uc.get_data_mut().com.next_stub = STUB_BASE + 0x100;
        uc
    }
}

#[cfg(test)]
mod tests {
    use super::testing::bare_uc;
    use super::*;

    fn state() -> State {
        let d = std::env::temp_dir();
        State::new(&d, &d, &d)
    }

    #[test]
    fn alloc_rounds_to_16_and_never_overlaps() {
        let mut s = state();
        let a = s.alloc(1);
        let b = s.alloc(17);
        let c = s.alloc(0);
        assert_eq!(a, HEAP_BASE);
        assert_eq!(b, a + 16);
        assert_eq!(c, b + 32);
        assert_eq!(s.heap_sizes[&c], 16);
    }

    #[test]
    fn freed_blocks_are_quarantined_then_reused_and_coalesced() {
        let mut s = state();
        let (a, b, c) = (s.alloc(32), s.alloc(32), s.alloc(32));
        s.free(a);
        s.free(b);
        // Quarantined: still not reusable.
        assert_ne!(s.alloc(32), a);
        // Releasing out of order merges neighbours into one block.
        s.release(b, 32);
        s.release(a, 32);
        assert_eq!(s.heap_free.get(&a), Some(&64));
        assert_eq!(s.alloc(64), a);
        assert!(s.heap_free.is_empty());
        s.free(c);
        s.free(c); // double free is ignored
        assert_eq!(s.heap_quarantine.len(), 3);
    }

    #[test]
    fn quarantine_releases_the_oldest_past_64_mib() {
        let mut s = state();
        let big: Vec<u32> = (0..3).map(|_| s.alloc(30 << 20)).collect();
        for &a in &big {
            s.free(a);
        }
        assert_eq!(s.heap_quarantine.len(), 2);
        assert_eq!(s.heap_free.get(&big[0]), Some(&(30 << 20)));
        assert_eq!(s.alloc(30 << 20), big[0]);
    }

    #[test]
    fn handles_increase_by_four() {
        let mut s = state();
        let (a, b) = (s.new_handle(), s.new_handle());
        assert_eq!(b, a + 4);
    }

    #[test]
    fn memory_helpers_round_trip() {
        let mut uc = bare_uc();
        wr32(&mut uc, 0x40_1000, 0xdead_beef);
        assert_eq!(rd32(&uc, 0x40_1000), 0xdead_beef);
        wr16(&mut uc, 0x40_1010, 0x1234);
        assert_eq!(rd_bytes(&uc, 0x40_1010, 2), [0x34, 0x12]);
        assert_eq!(wr_cstr(&mut uc, 0x40_1020, "Armaz\u{e9}m", 64), 7);
        assert_eq!(rd_bytes(&uc, 0x40_1020, 8), b"Armaz\xe9m\0");
        assert_eq!(rd_cstr(&uc, 0x40_1020), "Armaz\u{e9}m");
        assert_eq!(rd_cstr(&uc, 0), "");
        // Truncated to the buffer, still NUL-terminated.
        assert_eq!(wr_cstr(&mut uc, 0x40_1040, "abcdef", 4), 3);
        assert_eq!(rd_cstr(&uc, 0x40_1040), "abc");
        set_reg(&mut uc, RegisterX86::EBX, 77);
        assert_eq!(reg(&uc, RegisterX86::EBX), 77);
    }

    #[test]
    fn zeroed_alloc_clears_dirty_memory() {
        let mut uc = bare_uc();
        uc.mem_map(HEAP_BASE as u64, 0x10000, Prot::ALL).unwrap();
        wr_bytes(&mut uc, HEAP_BASE, &[0xaa; 64]);
        let a = zeroed_alloc(&mut uc, 40);
        assert_eq!(a, HEAP_BASE);
        assert_eq!(rd_bytes(&uc, a, 48), vec![0; 48]);
        assert_eq!(rd_bytes(&uc, a + 48, 1), [0xaa]);
    }

    #[test]
    fn building_accessor_patch_applies_only_to_the_original_bytes() {
        let mut uc = bare_uc();
        apply_patches(&mut uc);
        assert_eq!(rd_bytes(&uc, 0x41a4f4, 1), [0], "unknown code is left alone");
        wr_bytes(&mut uc, 0x41a4f4, &[0x52, 0x31, 0xd2, 0x66, 0x89, 0xc2, 0x89, 0xd0]);
        apply_patches(&mut uc);
        assert_eq!(rd_bytes(&uc, 0x41a4f4, 3), [0x0f, 0xb7, 0xc0]);
    }

    #[test]
    fn err_name_is_debug_text() {
        assert_eq!(err_name(uc_error::NOMEM), format!("{:?}", uc_error::NOMEM));
    }
}

#[cfg(test)]
mod machine_tests {
    use super::testing::{machine, run_to_exit};
    use super::*;
    use crate::pe::testing::slot;

    fn call_slot(i: usize) -> Vec<u8> {
        let mut c = vec![0xff, 0x15];
        c.extend_from_slice(&slot(i).to_le_bytes());
        c
    }

    fn push(v: u32) -> Vec<u8> {
        let mut c = vec![0x68];
        c.extend_from_slice(&v.to_le_bytes());
        c
    }

    fn store_eax(addr: u32) -> Vec<u8> {
        let mut c = vec![0xa3];
        c.extend_from_slice(&addr.to_le_bytes());
        c
    }

    #[test]
    fn exit_code_comes_from_exit_process() {
        let code = [push(7), call_slot(0)].concat();
        let mut m = machine(&code, &["ExitProcess"]);
        assert_eq!(run_to_exit(&mut m), Ok(7));
    }

    #[test]
    fn imports_get_stubs_and_return_values_reach_eax() {
        // GetVersion -> [0x403000]; GetCurrentProcessId -> [0x403004]; ExitProcess(0).
        let code = [call_slot(0), store_eax(0x40_3000), call_slot(1), store_eax(0x40_3004), push(0), call_slot(2)].concat();
        let mut m = machine(&code, &["GetVersion", "GetCurrentProcessId", "ExitProcess"]);
        assert_eq!(run_to_exit(&mut m), Ok(0));
        assert_eq!(rd32(&m.uc, 0x40_3000), 0xc000_0a04);
        assert_eq!(rd32(&m.uc, 0x40_3004), 100);
        assert_eq!(m.uc.get_data().stubs.len(), 3);
        assert_eq!(m.uc.get_data().api_counts["GetVersion"], 1);
        assert_eq!(rd32(&m.uc, slot(0)), STUB_BASE + 16);
    }

    #[test]
    fn host_can_call_back_into_the_guest_and_resume() {
        // DirectPlayEnumerate(cb, ctx): the host calls cb(guid, name, 6, 0, ctx) in the guest and then returns.
        // cb stores ctx at [0x403004] and returns (stdcall, 5 args).
        let cb = [vec![0x8b, 0x44, 0x24, 0x14], store_eax(0x40_3004), vec![0x31, 0xc0, 0xc2, 0x14, 0x00]].concat();
        let main = [push(0x55), push(0x40_1100), call_slot(0), store_eax(0x40_3008), push(0), call_slot(1)].concat();
        let mut code = main;
        code.resize(0x100, 0x90);
        code.extend(cb);
        let mut m = machine(&code, &["#2", "ExitProcess"]);
        assert_eq!(run_to_exit(&mut m), Ok(0));
        assert_eq!(rd32(&m.uc, 0x40_3004), 0x55, "the callback ran with its context argument");
        assert_eq!(rd32(&m.uc, 0x40_3008), 0, "DP_OK is returned to the caller afterwards");
        assert!(m.uc.get_data().frames.is_empty());
    }

    #[test]
    fn busy_guest_is_time_sliced_and_faults_are_reported() {
        let mut m = machine(&[0xeb, 0xfe], &[]); // jmp $
        for _ in 0..3 {
            assert_eq!(m.step(2_000).unwrap(), None);
        }
        let mut m = machine(&[0xa1, 0x78, 0x56, 0x34, 0x12], &[]); // mov eax, [0x12345678]
        let e = run_to_exit(&mut m).unwrap_err();
        assert!(e.starts_with("guest fault"), "{e}");
        assert!(e.contains("READ_UNMAPPED") && e.contains("eip=0x4010"), "{e}");
    }

    #[test]
    fn missing_or_invalid_executables_are_errors() {
        let dir = std::env::temp_dir();
        assert!(Machine::new(&dir.join("no-such.exe"), &dir, &dir, &dir).is_err());
        let bad = dir.join(format!("rl-bad-{}.exe", std::process::id()));
        std::fs::write(&bad, b"not an exe").unwrap();
        assert!(Machine::new(&bad, &dir, &dir, &dir).is_err());
        let _ = std::fs::remove_file(bad);
    }
}
