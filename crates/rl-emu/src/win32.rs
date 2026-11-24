//! Host implementations of the Win32 imports used by RISING.EXE.

use crate::{
    com,
    machine::*,
};
use std::io::{Read, Seek, SeekFrom, Write};
use unicorn_engine::RegisterX86;

/// stdcall argument bytes (or cdecl) for an import.
pub fn arg_bytes(_dll: &str, name: &str) -> (u16, bool) {
    let n: u16 = match name {
        "GetTickCount" | "GetCommandLineA" | "GetCurrentProcessId" | "GetCurrentThreadId" | "GetCurrentThread"
        | "GetEnvironmentStrings" | "GetLastError" | "GetVersion" | "TlsAlloc" | "WaitMessage" | "GetActiveWindow"
        | "timeGetTime" => 0,
        "FindClose" | "FreeLibrary" | "GetDriveTypeA" | "GlobalCompact" | "GlobalFree" | "GlobalHandle" | "GlobalLock"
        | "GlobalMemoryStatus" | "GlobalUnlock" | "LoadLibraryA" | "SetErrorMode" | "_lclose" | "CloseHandle"
        | "DeleteFileA" | "ExitProcess" | "ExitThread" | "GetFileType" | "GetModuleHandleA" | "GetStdHandle"
        | "ReleaseMutex" | "SetCurrentDirectoryA" | "SetEvent" | "TlsFree" | "TlsGetValue" | "DispatchMessageA"
        | "GetAsyncKeyState" | "GetCursorPos" | "GetSystemMetrics" | "MessageBeep" | "PostQuitMessage" | "RegisterClassA"
        | "SetFocus" | "ShowCursor" | "TranslateMessage" | "UpdateWindow" | "GetStockObject" | "GetOpenFileNameA" => 1,
        "FindFirstFileA" | "FindNextFileA" | "GetComputerNameA" | "GetProcAddress" | "_lcreat" | "_lopen"
        | "GetConsoleMode" | "GetCurrentDirectoryA" | "SetConsoleCtrlHandler" | "SetConsoleMode" | "SetStdHandle"
        | "TlsSetValue" | "WaitForSingleObject" | "BeginPaint" | "EndPaint" | "FindWindowA" | "GetWindowRect"
        | "LoadCursorA" | "LoadIconA" | "SetCursorPos" | "ShowWindow" | "UnregisterClassA" | "GetUserNameA" => 2,
        "GlobalAlloc_" => 2,
        "OpenFile" | "_hread" | "_hwrite" | "_llseek" | "CreateMutexA" | "GetModuleFileNameA" | "VirtualFree"
        | "DirectDrawCreate" | "DirectSoundCreate" => 3,
        "GlobalAlloc" => 2,
        "GetPrivateProfileIntA" | "CreateEventA" | "SetFilePointer" | "VirtualAlloc" | "ReadConsoleInputA"
        | "DefWindowProcA" | "GetMessageA" | "MessageBoxA" | "SendMessageA" | "mciSendCommandA" | "mciSendStringA" => 4,
        "ReadFile" | "WriteFile" | "WriteConsoleA" | "PeekMessageA" => 5,
        "GetPrivateProfileStringA" | "CreateThread" | "MoveWindow" => 6,
        "CreateFileA" => 7,
        "GetVolumeInformationA" => 8,
        "CreateWindowExA" => 12,
        "#1" => 3,
        "#2" => 2,
        _ => 0,
    };
    // wsprintfA and MCIWndCreateA are cdecl.
    let cdecl = matches!(name, "wsprintfA" | "MCIWndCreateA");
    (n * 4, cdecl)
}

fn log_unknown(uc: &Uc, name: &str, a: &[u32]) {
    eprintln!("[win32] UNIMPLEMENTED {name}({:#x}, {:#x}, {:#x}, {:#x})  ret={:#x}", a[0], a[1], a[2], a[3], rd32(uc, reg(uc, RegisterX86::ESP)));
}

fn set_err(uc: &mut Uc, e: u32) {
    uc.get_data_mut().last_error = e;
}

fn ensure_string(uc: &mut Uc, key: &'static str, s: &str) -> u32 {
    if let Some(&a) = uc.get_data().strings.get(key) {
        return a;
    }
    let a = uc.get_data_mut().alloc(s.len() as u32 + 1);
    wr_cstr(uc, a, s, s.len() + 1);
    uc.get_data_mut().strings.insert(key, a);
    a
}

/// Formats `wsprintfA`-style output using a cdecl argument list on the guest stack.
fn format_printf(uc: &Uc, fmt: &str, mut argp: u32) -> String {
    let mut out = String::new();
    let mut it = fmt.chars().peekable();
    while let Some(c) = it.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        let mut spec = String::new();
        while let Some(&n) = it.peek() {
            if n.is_ascii_digit() || matches!(n, '-' | '+' | ' ' | '#' | 'l' | 'h' | '.') {
                spec.push(n);
                it.next();
            } else {
                break;
            }
        }
        let conv = it.next().unwrap_or('%');
        let left = spec.contains('-');
        let zero = spec.starts_with('0') || spec.contains("-0");
        let width: usize = spec.trim_start_matches(['-', '+', ' ', '#', '0']).split('.').next().unwrap_or("").trim_matches(|c: char| !c.is_ascii_digit()).parse().unwrap_or(0);
        let body = match conv {
            '%' => "%".to_string(),
            'd' | 'i' => {
                let v = rd32(uc, argp) as i32;
                argp += 4;
                v.to_string()
            }
            'u' => {
                let v = rd32(uc, argp);
                argp += 4;
                v.to_string()
            }
            'x' => {
                let v = rd32(uc, argp);
                argp += 4;
                format!("{v:x}")
            }
            'X' => {
                let v = rd32(uc, argp);
                argp += 4;
                format!("{v:X}")
            }
            'c' => {
                let v = rd32(uc, argp);
                argp += 4;
                (v as u8 as char).to_string()
            }
            's' => {
                let p = rd32(uc, argp);
                argp += 4;
                rd_cstr(uc, p)
            }
            _ => String::new(),
        };
        let pad = width.saturating_sub(body.chars().count());
        if left {
            out.push_str(&body);
            out.extend(std::iter::repeat_n(' ', pad));
        } else if zero && conv != 's' && conv != 'c' {
            let (sign, digits) = body.strip_prefix('-').map_or(("", &body[..]), |d| ("-", d));
            out.push_str(sign);
            out.extend(std::iter::repeat_n('0', pad));
            out.push_str(digits);
        } else {
            out.extend(std::iter::repeat_n(' ', pad));
            out.push_str(&body);
        }
    }
    out
}

/// Runs an imported function. Returns the value for EAX, if any.
pub fn call_import(uc: &mut Uc, idx: usize, addr: u32, a: &[u32; 14]) -> Option<u32> {
    let t0 = std::time::Instant::now();
    let r = call_import_inner(uc, idx, addr, a);
    if t0.elapsed().as_millis() >= 5 && std::env::var_os("RL_LOG_SLOW").is_some() {
        eprintln!("[slow] {} took {:?} (a0={:#x})", uc.get_data().stubs[idx].name, t0.elapsed(), a[0]);
    }
    // The message pump is where the host gets control back (events, frame pacing). Yield there
    // deterministically instead of relying on CPU timeouts, which could stop the CPU between a
    // stub's hook and its `ret` and make the handler's result get lost.
    if let Some(v) = r {
        let n = &uc.get_data().stubs[idx].name;
        // The game waits for Return to be released by polling GetAsyncKeyState without ever
            // touching the message queue, so that poll must let the host process input too.
            if n == "PeekMessageA" || n == "GetMessageA" || (n == "GetAsyncKeyState" && v != 0) {
            stop_with(uc, Action::Resume { pc: addr, set_eax: Some(v) });
            return None;
        }
    }
    r
}

fn call_import_inner(uc: &mut Uc, idx: usize, addr: u32, a: &[u32; 14]) -> Option<u32> {
    let name = uc.get_data().stubs[idx].name.clone();
    if uc.get_data().log_calls {
        eprintln!("[win32] {name}({:#x}, {:#x}, {:#x}, {:#x})", a[0], a[1], a[2], a[3]);
    }
    *uc.get_data_mut().api_counts.entry(name.clone()).or_insert(0) += 1;
    Some(match name.as_str() {
        // ---- process / environment ----
        "GetVersion" => 0xc000_0a04,
        "GetCommandLineA" => ensure_string(uc, "cmdline", "C:\\RISING\\RISING.EXE"),
        "GetEnvironmentStrings" => {
            let p = uc.get_data_mut().alloc(8);
            wr_bytes(uc, p, &[0; 8]);
            p
        }
        "GetModuleFileNameA" => wr_cstr(uc, a[1], "C:\\RISING\\RISING.EXE", a[2] as usize) as u32,
        "GetModuleHandleA" => 0x0040_0000,
        "GetCurrentProcessId" => 100,
        "GetCurrentThreadId" => 1,
        "GetCurrentThread" => 0xfffffffe,
        "ExitProcess" | "ExitThread" => {
            stop_with(uc, Action::Exit(a[0]));
            0
        }
        "GetLastError" => uc.get_data().last_error,
        "SetErrorMode" => 0,
        "GetUserNameA" => {
            wr_cstr(uc, a[0], "player", 64);
            wr32(uc, a[1], 7);
            1
        }
        "GetComputerNameA" => {
            wr_cstr(uc, a[0], "RISING", 64);
            wr32(uc, a[1], 6);
            1
        }
        "GetProcAddress" | "LoadLibraryA" | "FreeLibrary" => 0,
        "GetTickCount" | "timeGetTime" => {
            if std::env::var_os("RL_LOG_TICK").is_some() {
                let esp = uc.reg_read(unicorn_engine::RegisterX86::ESP).unwrap_or(0) as u32;
                eprintln!("[tick] from {:#x}", rd32(uc, esp));
            }
            uc.get_data().start.elapsed().as_millis() as u32
        }
        "GlobalMemoryStatus" => {
            // MEMORYSTATUS: pretend 128 MB.
            for (i, v) in [32u32, 50, 128 << 20, 100 << 20, 256 << 20, 200 << 20, 512 << 20, 400 << 20].iter().enumerate() {
                wr32(uc, a[0] + i as u32 * 4, *v);
            }
            0
        }
        // ---- console (Watcom CRT start-up) ----
        "GetStdHandle" => a[0],
        "GetFileType" => 2,
        "GetConsoleMode" | "SetConsoleMode" | "SetConsoleCtrlHandler" | "SetStdHandle" => 1,
        "WriteConsoleA" | "ReadConsoleInputA" => 1,
        // ---- TLS ----
        "TlsAlloc" => {
            let s = uc.get_data_mut();
            let i = s.tls.iter().position(|&v| v == 0).unwrap_or(1).max(1);
            s.tls[i] = 0;
            i as u32
        }
        "TlsFree" => 1,
        "TlsGetValue" => uc.get_data().tls.get(a[0] as usize + 32).copied().unwrap_or(0),
        "TlsSetValue" => {
            if let Some(v) = uc.get_data_mut().tls.get_mut(a[0] as usize + 32) {
                *v = a[1];
            }
            1
        }
        // ---- memory ----
        "GlobalAlloc" | "VirtualAlloc" => zeroed_alloc(uc, a[1]),
        "GlobalFree" => {
            uc.get_data_mut().free(a[0]);
            0
        }
        "VirtualFree" => {
            uc.get_data_mut().free(a[0]);
            1
        }
        "GlobalLock" | "GlobalHandle" => a[0],
        "GlobalUnlock" => 1,
        "GlobalCompact" => 0x1000_0000,
        // ---- threads and sync (single threaded) ----
        "CreateMutexA" | "CreateEventA" => uc.get_data_mut().new_handle(),
        "WaitForSingleObject" => 0,
        "ReleaseMutex" | "SetEvent" => 1,
        "CreateThread" => {
            eprintln!("[win32] CreateThread({:#x}) is not supported yet", a[2]);
            0
        }
        // ---- profile strings (RISING.INI holds all unit and building stats) ----
        "GetPrivateProfileIntA" => {
            let (app, key, file) = (rd_cstr(uc, a[0]), rd_cstr(uc, a[1]), rd_cstr(uc, a[3]));
            if std::env::var("RL_LOG_INI").is_ok_and(|p| app.contains(&p)) {
                let esp = uc.reg_read(unicorn_engine::RegisterX86::ESP).unwrap_or(0) as u32;
                eprintln!("[ini] {app}.{key} = {:?} (from {:#x})", profile_value(uc, &file, &app, &key), rd32(uc, esp));
            }
            match profile_value(uc, &file, &app, &key) {
                Some(v) => parse_int_prefix(&v).unwrap_or(a[2] as i32) as u32,
                None => a[2],
            }
        }
        "GetPrivateProfileStringA" => {
            let (app, key, file) = (rd_cstr(uc, a[0]), rd_cstr(uc, a[1]), rd_cstr(uc, a[5]));
            let v = profile_value(uc, &file, &app, &key).unwrap_or_else(|| rd_cstr(uc, a[2]));
            wr_cstr(uc, a[3], &v, a[4] as usize) as u32
        }
        // ---- files ----
        "GetDriveTypeA" => {
            let p = rd_cstr(uc, a[0]);
            match p.chars().next().map(|c| c.to_ascii_uppercase()) {
                Some('C') => 3,
                Some('D') => 5,
                _ => 1,
            }
        }
        "GetVolumeInformationA" => {
            let root = rd_cstr(uc, a[0]);
            if !root.to_ascii_uppercase().starts_with('D') {
                return Some(0);
            }
            if a[1] != 0 {
                wr_cstr(uc, a[1], "RISING_LANDS_BR_", a[2] as usize);
            }
            for p in [a[3], a[4], a[5]] {
                if p != 0 {
                    wr32(uc, p, if p == a[4] { 255 } else { 0x1234_5678 });
                }
            }
            if a[6] != 0 {
                wr_cstr(uc, a[6], "CDFS", a[7] as usize);
            }
            1
        }
        "GetCurrentDirectoryA" => {
            let cwd = uc.get_data().vfs.cwd.clone();
            wr_cstr(uc, a[1], &cwd, a[0] as usize) as u32
        }
        "SetCurrentDirectoryA" => {
            let p = rd_cstr(uc, a[0]);
            if uc.get_data().vfs.resolve(&p).is_some() {
                uc.get_data_mut().vfs.cwd = p;
                1
            } else {
                0
            }
        }
        "OpenFile" => {
            // OFSTRUCT is ignored; flags: OF_READ=0, OF_WRITE=1, OF_READWRITE=2, OF_CREATE=0x1000.
            let p = rd_cstr(uc, a[0]);
            let flags = a[2];
            let h = uc.get_data_mut().vfs.open(&p, flags & 3 != 0, flags & 0x1000 != 0);
            h.unwrap_or(u32::MAX)
        }
        "_lopen" => {
            let p = rd_cstr(uc, a[0]);
            uc.get_data_mut().vfs.open(&p, a[1] & 3 != 0, false).unwrap_or(u32::MAX)
        }
        "_lcreat" => {
            let p = rd_cstr(uc, a[0]);
            uc.get_data_mut().vfs.open(&p, true, true).unwrap_or(u32::MAX)
        }
        "CreateFileA" => {
            let p = rd_cstr(uc, a[0]);
            let write = a[1] & 0x4000_0000 != 0;
            let create = matches!(a[4], 1 | 2 | 4);
            if uc.get_data().log_calls {
                eprintln!("[win32]   path={p} write={write} create={create}");
            }
            uc.get_data_mut().vfs.open(&p, write, create).unwrap_or(u32::MAX)
        }
        "_hread" | "ReadFile" => {
            let (buf, n) = (a[1], a[2] as usize);
            let mut data = vec![0u8; n];
            let got = match uc.get_data_mut().vfs.file(a[0]) {
                Some(f) => f.read(&mut data).unwrap_or(0),
                None => return Some(if name == "_hread" { u32::MAX } else { 0 }),
            };
            if uc.get_data().log_calls {
                eprintln!("[win32]   -> read {got} of {n}");
            }
            wr_bytes(uc, buf, &data[..got]);
            if name == "ReadFile" {
                if a[3] != 0 {
                    wr32(uc, a[3], got as u32);
                }
                1
            } else {
                got as u32
            }
        }
        "_hwrite" | "WriteFile" => {
            let data = rd_bytes(uc, a[1], a[2] as usize);
            let n = match uc.get_data_mut().vfs.file(a[0]) {
                Some(f) => f.write(&data).unwrap_or(0),
                None => 0,
            };
            if name == "WriteFile" {
                if a[3] != 0 {
                    wr32(uc, a[3], n as u32);
                }
                1
            } else {
                n as u32
            }
        }
        "_llseek" | "SetFilePointer" => {
            let from = if name == "_llseek" { a[2] } else { a[3] };
            let off = a[1] as i32 as i64;
            let pos = match uc.get_data_mut().vfs.file(a[0]) {
                Some(f) => {
                    let sf = match from {
                        0 => SeekFrom::Start(off.max(0) as u64),
                        1 => SeekFrom::Current(off),
                        _ => SeekFrom::End(off),
                    };
                    f.seek(sf).unwrap_or(u64::MAX)
                }
                None => u64::MAX,
            };
            if pos == u64::MAX {
                u32::MAX
            } else {
                pos as u32
            }
        }
        "_lclose" | "CloseHandle" => {
            uc.get_data_mut().vfs.close(a[0]);
            1
        }
        "DeleteFileA" => {
            let p = rd_cstr(uc, a[0]);
            // Only files the game itself created (in the overlay) can be deleted.
            match uc.get_data().vfs.resolve(&p) {
                Some(h) if h.starts_with(&uc.get_data().vfs.overlay) => std::fs::remove_file(h).is_ok() as u32,
                _ => 0,
            }
        }
        "FindFirstFileA" => {
            let pat = rd_cstr(uc, a[0]);
            let mut find = uc.get_data().vfs.find(&pat);
            if find.entries.is_empty() {
                set_err(uc, 2);
                return Some(u32::MAX);
            }
            let h = uc.get_data_mut().new_handle();
            write_find_data(uc, a[1], &find.entries[0]);
            find.next = 1;
            uc.get_data_mut().finds.insert(h, find);
            h
        }
        "FindNextFileA" => {
            let next = uc.get_data_mut().finds.get_mut(&a[0]).and_then(|f| {
                let e = f.entries.get(f.next).cloned();
                f.next += 1;
                e
            });
            match next {
                Some(p) => {
                    write_find_data(uc, a[1], &p);
                    1
                }
                None => {
                    set_err(uc, 18);
                    0
                }
            }
        }
        "FindClose" => {
            uc.get_data_mut().finds.remove(&a[0]);
            1
        }
        // ---- user32: windows and messages ----
        "RegisterClassA" => {
            let proc_ = rd32(uc, a[0] + 4);
            let cls = rd_cstr(uc, rd32(uc, a[0] + 36));
            uc.get_data_mut().classes.insert(cls.to_ascii_lowercase(), proc_);
            0xc001
        }
        "UnregisterClassA" => 1,
        "CreateWindowExA" => {
            let cls = rd_cstr(uc, a[1]).to_ascii_lowercase();
            let proc_ = uc.get_data().classes.get(&cls).copied().unwrap_or(0);
            let hwnd = 0x1000 + uc.get_data().windows.len() as u32 * 4;
            uc.get_data_mut().windows.push(Window { proc_ });
            if proc_ != 0 {
                // CREATESTRUCT
                let cs = uc.get_data_mut().alloc(48);
                let fields = [a[11], a[10], a[9], a[8], a[7], a[6], a[5], a[4], a[3], a[2], a[1], a[0]];
                for (i, v) in fields.iter().enumerate() {
                    wr32(uc, cs + i as u32 * 4, *v);
                }
                stop_with(uc, Action::call(proc_, vec![hwnd, 1, 0, cs], addr, Some(hwnd)));
            }
            hwnd
        }
        "DefWindowProcA" => 0,
        "SetCursorPos" => {
            // The game re-sets the position it just read every tick; that must not overwrite the real
            // pointer with the clamped one.
            if (a[0] as i32, a[1] as i32) != uc.get_data().reported_cursor {
                uc.get_data_mut().cursor = (a[0] as i32, a[1] as i32);
            }
            1
        }
        "ShowWindow" | "UpdateWindow" | "MoveWindow" | "SetFocus" | "MessageBeep" => 1,
        // The game shows the "system" pointer when the cursor leaves the inner map area and hides it
        // when it comes back; its own sprite is only drawn inside. The frontend draws the arrow.
        "ShowCursor" => {
            uc.get_data_mut().sys_cursor = a[0] != 0;
            0
        }
        "GetActiveWindow" => 0x1000,
        "FindWindowA" => 0,
        "LoadCursorA" | "LoadIconA" | "GetStockObject" => 1,
        "GetWindowRect" => {
            for (i, v) in [0u32, 0, 800, 600].iter().enumerate() {
                wr32(uc, a[1] + i as u32 * 4, *v);
            }
            1
        }
        "GetSystemMetrics" => match a[0] {
            0 => 800,
            1 => 600,
            _ => 0,
        },
        "GetCursorPos" => {
            // The game's scroll zones (hit-rect table at 0x5099b4, regions 1-8) are x 0-2 (left), 798-799,
            // y 0-1 and 598-599.
            const EDGE_BAND: i32 = 2;
            // Always inside the 800x600 screen: the original indexed tables with the raw cursor
            // position and crashed when it came back from outside the window (archer bug).
            let (x, y) = uc.get_data().cursor;
            let (x, y) = (x.clamp(0, 799), y.clamp(0, 599));
            // With edge scrolling off, the game never sees the pointer in the band it scrolls from.
            let (x, y) = if uc.get_data().block_edges { (x.clamp(EDGE_BAND + 1, 799 - EDGE_BAND), y.clamp(EDGE_BAND, 599 - EDGE_BAND)) } else { (x, y) };
            uc.get_data_mut().reported_cursor = (x, y);
            wr32(uc, a[0], x as u32);
            wr32(uc, a[0] + 4, y as u32);
            1
        }
        "GetAsyncKeyState" => {
            if uc.get_data().keys.get(a[0] as usize).copied().unwrap_or(false) {
                0x8001
            } else {
                0
            }
        }
        "BeginPaint" | "EndPaint" => 1,
        "PostQuitMessage" => {
            uc.get_data_mut().quit = true;
            0
        }
        "SendMessageA" => {
            let proc_ = uc.get_data().windows.get(((a[0].wrapping_sub(0x1000)) / 4) as usize).map(|w| w.proc_).unwrap_or(0);
            if proc_ != 0 {
                stop_with(uc, Action::call(proc_, vec![a[0], a[1], a[2], a[3]], addr, None));
            }
            0
        }
        "PeekMessageA" | "GetMessageA" => {
            let blocking = name == "GetMessageA";
            let remove = name == "GetMessageA" || a[4] & 1 != 0;
            let msg = if remove { uc.get_data_mut().msgs.pop_front() } else { uc.get_data().msgs.front().map(|m| Msg { hwnd: m.hwnd, message: m.message, wparam: m.wparam, lparam: m.lparam }) };
            match msg {
                Some(m) => {
                    for (i, v) in [m.hwnd, m.message, m.wparam, m.lparam, 0, 0, 0].iter().enumerate() {
                        wr32(uc, a[0] + i as u32 * 4, *v);
                    }
                    1
                }
                None if uc.get_data().quit => {
                    if blocking {
                        // WM_QUIT
                        for (i, v) in [0u32, 0x12, 0, 0, 0, 0, 0].iter().enumerate() {
                            wr32(uc, a[0] + i as u32 * 4, *v);
                        }
                        0
                    } else {
                        for (i, v) in [0u32, 0x12, 0, 0, 0, 0, 0].iter().enumerate() {
                            wr32(uc, a[0] + i as u32 * 4, *v);
                        }
                        1
                    }
                }
                None => {
                    if blocking {
                        // Nothing to wait for in a headless run: idle message.
                        for (i, v) in [0x1000u32, 0, 0, 0, 0, 0, 0].iter().enumerate() {
                            wr32(uc, a[0] + i as u32 * 4, *v);
                        }
                        1
                    } else {
                        pace_frame(uc);
                        0
                    }
                }
            }
        }
        "TranslateMessage" => 0,
        "WaitMessage" => 1,
        "DispatchMessageA" => {
            let hwnd = rd32(uc, a[0]);
            let proc_ = uc.get_data().windows.get(((hwnd.wrapping_sub(0x1000)) / 4) as usize).map(|w| w.proc_).unwrap_or(0);
            if proc_ != 0 {
                let (m, w, l) = (rd32(uc, a[0] + 4), rd32(uc, a[0] + 8), rd32(uc, a[0] + 12));
                stop_with(uc, Action::call(proc_, vec![hwnd, m, w, l], addr, None));
            }
            0
        }
        "MessageBoxA" => {
            eprintln!("[win32] MessageBox: {} / {}", rd_cstr(uc, a[2]), rd_cstr(uc, a[1]));
            1
        }
        "GetOpenFileNameA" => 0,
        "wsprintfA" => {
            let fmt = rd_cstr(uc, a[1]);
            let s = format_printf(uc, &fmt, reg(uc, RegisterX86::ESP) + 12);
            wr_cstr(uc, a[0], &s, 4096) as u32
        }
        // ---- DirectX and multimedia ----
        "DirectDrawCreate" => com::direct_draw_create(uc, a[1]),
        "DirectSoundCreate" => com::direct_sound_create(uc, a[1]),
        "#1" => crate::dplay::create(uc, a[1]),
        "#2" => return crate::dplay::enumerate(uc, a[0], a[1], addr),
        "mciSendStringA" => {
            let cmd = rd_cstr(uc, a[0]);
            mci_string(uc, &cmd, a[1], a[2])
        }
        "mciSendCommandA" => mci_command(uc, a[0], a[1], a[2], a[3]),
        "MCIWndCreateA" => 0,
        _ => {
            log_unknown(uc, &name, a);
            0
        }
    })
}

fn write_find_data(uc: &mut Uc, at: u32, p: &std::path::Path) {
    let md = std::fs::metadata(p);
    let (attr, size) = match &md {
        Ok(m) if m.is_dir() => (0x10u32, 0u32),
        Ok(m) => (0x80, m.len() as u32),
        Err(_) => (0x80, 0),
    };
    wr_bytes(uc, at, &[0u8; 318]);
    wr32(uc, at, attr);
    wr32(uc, at + 32, size);
    let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    wr_cstr(uc, at + 44, &name, 260);
}

fn parse_int_prefix(v: &str) -> Option<i32> {
    let t = v.trim_start();
    let (neg, t) = t.strip_prefix('-').map_or((false, t), |r| (true, r));
    let digits: String = t.chars().take_while(|c| c.is_ascii_digit()).collect();
    let n: i64 = digits.parse().ok()?;
    Some(if neg { -n } else { n } as i32)
}

/// Looks a key up in an INI file on the guest's file system. Parsed files are cached.
fn profile_value(uc: &mut Uc, file: &str, app: &str, key: &str) -> Option<String> {
    let host = uc.get_data().vfs.resolve(file)?;
    let host = uc.get_data().vfs.read_path(host);
    let cache_key = host.to_string_lossy().into_owned();
    if !uc.get_data().ini.contains_key(&cache_key) {
        let text: String = std::fs::read(&host).ok()?.iter().map(|&b| b as char).collect();
        let mut section = String::new();
        let mut out = String::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with(';') {
                continue;
            }
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.split(']').next()) {
                section = name.trim().to_ascii_lowercase();
            } else if let Some((k, v)) = line.split_once('=') {
                // One entry per line: "section\u{1}key\u{1}value\u{2}".
                out.push_str(&format!("{section}\u{1}{}\u{1}{}\u{2}", k.trim().to_ascii_lowercase(), v.trim()));
            }
        }
        uc.get_data_mut().ini.insert(cache_key.clone(), out);
    }
    let want = format!("{}\u{1}{}\u{1}", app.to_ascii_lowercase(), key.to_ascii_lowercase());
    let blob = &uc.get_data().ini[&cache_key];
    let start = blob.find(&format!("\u{2}{want}")).map(|i| i + 1).or_else(|| blob.starts_with(&want).then_some(0))?;
    let rest = &blob[start + want.len()..];
    Some(rest[..rest.find('\u{2}')?].to_string())
}

/// Mission text / info screens (ids 0x11, 0x17): the scroll arrows act only on frames where the
/// frame counter (0x508f7c) is a multiple of 10 while the click flag (0x5099ae) is set, so a short
/// click scrolled only some of the time. When the flag first appears, line the counter up so the
/// next frame scrolls once (holding still repeats every 10 frames).
fn scroll_click_assist(uc: &mut Uc) {
    let down = rd_bytes(uc, 0x5099ae, 1)[0] != 0;
    let armed = uc.get_data().click_aligned;
    if !down {
        uc.get_data_mut().click_aligned = false;
    } else if !armed {
        uc.get_data_mut().click_aligned = true;
        if matches!(rd_bytes(uc, 0x4e2f78, 1)[0], 0x11 | 0x17) {
            let n = rd32(uc, 0x508f7c);
            wr32(uc, 0x508f7c, n - n % 10 + 9);
        }
    }
}

/// The game loop polls for messages once per pass; hold it to the frame budget so the
/// game runs at the intended speed instead of as fast as the host allows.
fn pace_frame(uc: &mut Uc) {
    scroll_click_assist(uc);
    crate::debug::tick(uc);
    let now = std::time::Instant::now();
    let (next, interval) = (uc.get_data().next_frame, uc.get_data().frame_interval.div_f64(uc.get_data().speed));
    if next > now {
        std::thread::sleep(next - now);
        uc.get_data_mut().next_frame = next + interval;
    } else {
        // Fell behind (or first call): don't try to catch up with a burst of frames.
        if std::env::var_os("RL_LOG_SLOW").is_some() && now - next > std::time::Duration::from_millis(60) {
            eprintln!("[late] frame {:?} over budget", now - next);
        }
        uc.get_data_mut().next_frame = now + interval;
    }
}

fn cd_track(uc: &mut Uc, n: u32) -> Option<std::sync::Arc<Vec<u8>>> {
    if let Some(t) = uc.get_data().cd_tracks.get(&n) {
        return Some(t.clone());
    }
    let p = uc.get_data().vfs.resolve(&format!("D:\\track{n:02}.raw"))?;
    let t = std::sync::Arc::new(std::fs::read(p).ok()?);
    uc.get_data_mut().cd_tracks.insert(n, t.clone());
    Some(t)
}

/// The subset of MCI string commands the game uses for CD audio. Tracks 2 and 3 of the
/// disc are CD audio; the extracted files are raw 44.1 kHz stereo 16-bit.
fn mci_string(uc: &mut Uc, cmd: &str, ret: u32, ret_len: u32) -> u32 {
    let words: Vec<&str> = cmd.split_whitespace().collect();
    let reply = |uc: &mut Uc, s: &str| {
        if ret != 0 && ret_len > 0 {
            wr_cstr(uc, ret, s, ret_len as usize);
        }
    };
    if uc.get_data().log_calls {
        eprintln!("[mci] {cmd}");
    }
    match words.as_slice() {
        ["open", _, "type", "cdaudio", ..] => 0,
        ["close", ..] => 0,
        ["stop", ..] => {
            uc.get_data().mixer.lock().unwrap().cd = None;
            0
        }
        ["set", _, "time", "format", fmt, ..] => {
            uc.get_data_mut().cd_format_tmsf = *fmt == "tmsf";
            0
        }
        ["status", _, "number", "of", "tracks", ..] => {
            reply(uc, "3");
            0
        }
        ["status", _, "length", "track", n, ..] => {
            let n: u32 = n.parse().unwrap_or(0);
            match cd_track(uc, n) {
                Some(t) => {
                    let ms = t.len() as u64 * 1000 / (44100 * 4);
                    if uc.get_data().cd_format_tmsf {
                        // Minutes, seconds, frames (75 per second) packed as "mm:ss:ff".
                        let (m, s, f) = (ms / 60000, ms / 1000 % 60, ms % 1000 * 75 / 1000);
                        reply(uc, &format!("{m:02}:{s:02}:{f:02}"));
                    } else {
                        reply(uc, &ms.to_string());
                    }
                    0
                }
                None => 263,
            }
        }
        ["play", _, "from", from, ..] => {
            // With the tmsf format the low byte of the position is the track number.
            let v: u32 = from.parse().unwrap_or(0);
            let track = if uc.get_data().cd_format_tmsf { v & 0xff } else { v };
            match cd_track(uc, track) {
                Some(t) => {
                    uc.get_data().mixer.lock().unwrap().cd = Some(crate::audio::Cd { data: t, pos: 0 });
                    0
                }
                None => 263,
            }
        }
        _ => {
            eprintln!("[mci] unhandled: {cmd}");
            263
        }
    }
}

/// MCI commands for AVI playback (the intro, victory and defeat movies).
fn mci_command(uc: &mut Uc, device: u32, msg: u32, flags: u32, params: u32) -> u32 {
    if uc.get_data().log_calls {
        eprintln!("[mci] command dev={device} msg={msg:#x} flags={flags:#x} params={params:#x}");
    }
    match msg {
        0x803 => {
            // MCI_OPEN: dwStyle/hWnd are ignored; the element name is a path on the guest file system.
            let name = rd_cstr(uc, rd32(uc, params + 12));
            if uc.get_data().log_calls {
                eprintln!("[mci] open movie {name:?}");
            }
            let Some(host) = uc.get_data().vfs.resolve(&name).filter(|_| !uc.get_data().skip_movies) else {
                eprintln!("[mci] movie not found: {name:?}");
                wr32(uc, params + 4, 1);
                return 0; // let the game carry on without the movie
            };
            match std::fs::read(&host).map_err(|e| e.to_string()).and_then(|b| crate::video::Avi::parse(&b)) {
                Ok(avi) => uc.get_data_mut().video_open = Some(avi),
                Err(e) => eprintln!("[mci] cannot open {name:?}: {e}"),
            }
            wr32(uc, params + 4, 1);
            0
        }
        0x806 => {
            // MCI_PLAY: start the movie and its sound track.
            if let Some(avi) = uc.get_data_mut().video_open.take() {
                let mut voice = crate::audio::Voice::new(0);
                voice.data = avi.audio.iter().flat_map(|s| s.to_le_bytes()).collect();
                voice.channels = avi.channels.max(1);
                voice.rate = avi.sample_rate;
                voice.bits = 16;
                voice.playing = !voice.data.is_empty();
                uc.get_data().mixer.lock().unwrap().voices.insert(VIDEO_VOICE, voice);
                uc.get_data_mut().video = Some(crate::video::Player::new(avi));
            }
            0
        }
        0x814 => {
            // MCI_STATUS (item = mode): 0x20e playing, 0x20d stopped.
            let playing = uc.get_data().video.as_ref().is_some_and(|v| !v.finished);
            wr32(uc, params + 4, if playing { 0x20e } else { 0x20d });
            0
        }
        0x804 => {
            // MCI_CLOSE
            uc.get_data_mut().video = None;
            uc.get_data_mut().video_open = None;
            uc.get_data().mixer.lock().unwrap().voices.remove(&VIDEO_VOICE);
            0
        }
        0x841 | 0x842 => 0,
        _ => 0,
    }
}

const VIDEO_VOICE: u32 = 0xfff0;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::testing::{uc_in, with_heap};
    use std::path::PathBuf;

    struct Env {
        root: PathBuf,
        uc: Uc<'static>,
    }

    impl Drop for Env {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    /// Guest with real install/disc/user directories (install holds `RISING.INI`, `Maps/a.txt`).
    fn env(name: &str) -> Env {
        let root = std::env::temp_dir().join(format!("rl-win32-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for d in ["install/Maps", "disc", "user"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        std::fs::write(root.join("install/RISING.INI"), "; c\n[SOLDAT]\nRAYON=7 ; x\nName = Soldado\nBAD=abc\nNEG=-3\n[Other]\nRAYON=1\n").unwrap();
        std::fs::write(root.join("install/Maps/a.txt"), b"hello world").unwrap();
        std::fs::write(root.join("install/Maps/b.txt"), b"x").unwrap();
        let uc = with_heap(uc_in(&root.join("install"), &root.join("disc"), &root.join("user")));
        Env { root, uc }
    }

    fn imp(uc: &mut Uc, name: &str, a: &[u32]) -> Option<u32> {
        let idx = uc.get_data().stubs.len();
        uc.get_data_mut().stubs.push(Stub { name: name.into() });
        let mut args = [0u32; 14];
        args[..a.len()].copy_from_slice(a);
        call_import_inner(uc, idx, 0x7f00_0200, &args)
    }

    fn call(uc: &mut Uc, name: &str, a: &[u32]) -> u32 {
        imp(uc, name, a).unwrap_or_else(|| panic!("{name} yielded"))
    }

    fn cstr(uc: &mut Uc, s: &str) -> u32 {
        let p = zeroed_alloc(uc, s.len() as u32 + 1);
        wr_cstr(uc, p, s, s.len() + 1);
        p
    }

    fn buf(uc: &mut Uc, n: u32) -> u32 {
        zeroed_alloc(uc, n)
    }

    #[test]
    fn argument_byte_counts_follow_the_calling_convention() {
        assert_eq!(arg_bytes("k32", "GetTickCount"), (0, false));
        assert_eq!(arg_bytes("k32", "CloseHandle"), (4, false));
        assert_eq!(arg_bytes("k32", "FindFirstFileA"), (8, false));
        assert_eq!(arg_bytes("k32", "OpenFile"), (12, false));
        assert_eq!(arg_bytes("k32", "ReadFile"), (20, false));
        assert_eq!(arg_bytes("k32", "CreateFileA"), (28, false));
        assert_eq!(arg_bytes("u32", "CreateWindowExA"), (48, false));
        assert_eq!(arg_bytes("dp", "#1"), (12, false));
        assert_eq!(arg_bytes("u32", "wsprintfA"), (0, true), "cdecl: the caller cleans up");
        assert_eq!(arg_bytes("x", "NoSuchFunction"), (0, false));
    }

    #[test]
    fn printf_formats_widths_flags_and_strings() {
        let mut e = env("printf");
        let uc = &mut e.uc;
        let s = cstr(uc, "str");
        let args = buf(uc, 64);
        for (i, v) in [42u32, (-7i32) as u32, 255, s, b'Z' as u32, 7].iter().enumerate() {
            wr32(uc, args + i as u32 * 4, *v);
        }
        assert_eq!(format_printf(uc, "n=%d|%5d|%-4x|", args), "n=42|   -7|ff  |");
        assert_eq!(format_printf(uc, "%s %c %03u %%", args + 12), "str Z 007 %");
        assert_eq!(format_printf(uc, "%05d %X", args + 4), "-0007 FF");
        assert_eq!(format_printf(uc, "%-6s|", args + 12), "str   |");
        assert_eq!(format_printf(uc, "%6s|", args + 12), "   str|");
        assert_eq!(format_printf(uc, "plain", args), "plain");
        assert_eq!(format_printf(uc, "%q!", args), "!", "unknown conversions print nothing");
    }

    #[test]
    fn int_prefix_parsing() {
        assert_eq!(parse_int_prefix("  12abc"), Some(12));
        assert_eq!(parse_int_prefix("-3"), Some(-3));
        assert_eq!(parse_int_prefix("abc"), None);
        assert_eq!(parse_int_prefix(""), None);
    }

    #[test]
    fn profile_strings_and_ints_come_from_the_ini() {
        let mut e = env("ini");
        let uc = &mut e.uc;
        let (soldat, other, file) = (cstr(uc, "SOLDAT"), cstr(uc, "other"), cstr(uc, "C:\\RISING\\rising.ini"));
        let (rayon, name, bad, neg, none) = (cstr(uc, "rayon"), cstr(uc, "NAME"), cstr(uc, "BAD"), cstr(uc, "NEG"), cstr(uc, "nokey"));
        assert_eq!(call(uc, "GetPrivateProfileIntA", &[soldat, rayon, 99, file]), 7);
        assert_eq!(call(uc, "GetPrivateProfileIntA", &[other, rayon, 99, file]), 1);
        assert_eq!(call(uc, "GetPrivateProfileIntA", &[soldat, bad, 99, file]), 99, "unparsable values fall back to the default");
        assert_eq!(call(uc, "GetPrivateProfileIntA", &[soldat, neg, 99, file]) as i32, -3);
        assert_eq!(call(uc, "GetPrivateProfileIntA", &[soldat, none, 55, file]), 55);
        let missing = cstr(uc, "C:\\nofile.ini");
        assert_eq!(call(uc, "GetPrivateProfileIntA", &[soldat, rayon, 4, missing]), 4);
        let (out, def) = (buf(uc, 32), cstr(uc, "dflt"));
        assert_eq!(call(uc, "GetPrivateProfileStringA", &[soldat, name, def, out, 32, file]), 7);
        assert_eq!(rd_cstr(uc, out), "Soldado");
        assert_eq!(call(uc, "GetPrivateProfileStringA", &[soldat, none, def, out, 32, file]), 4);
        assert_eq!(rd_cstr(uc, out), "dflt");
        assert_eq!(call(uc, "GetPrivateProfileStringA", &[soldat, name, def, out, 4, file]), 3, "truncated to the buffer");
    }

    #[test]
    fn process_environment_and_identity() {
        let mut e = env("proc");
        let uc = &mut e.uc;
        assert_eq!(call(uc, "GetVersion", &[]), 0xc000_0a04);
        assert_eq!(call(uc, "GetModuleHandleA", &[0]), 0x40_0000);
        let cmd = call(uc, "GetCommandLineA", &[]);
        assert_eq!(rd_cstr(uc, cmd), "C:\\RISING\\RISING.EXE");
        assert_eq!(call(uc, "GetCommandLineA", &[]), cmd, "the string is allocated once");
        let (name, size) = (buf(uc, 64), buf(uc, 4));
        assert_eq!(call(uc, "GetModuleFileNameA", &[0, name, 64]), 20);
        call(uc, "GetUserNameA", &[name, size]);
        assert_eq!((rd_cstr(uc, name), rd32(uc, size)), ("player".into(), 7));
        call(uc, "GetComputerNameA", &[name, size]);
        assert_eq!(rd_cstr(uc, name), "RISING");
        let ms = buf(uc, 32);
        call(uc, "GlobalMemoryStatus", &[ms]);
        assert_eq!(rd32(uc, ms + 8), 128 << 20);
        assert_eq!(call(uc, "GetStdHandle", &[7]), 7);
        assert_eq!(call(uc, "GetProcAddress", &[1, 2]), 0);
        assert!(call(uc, "GetTickCount", &[]) < 5000);
        assert_eq!(call(uc, "GetSystemMetrics", &[0]), 800);
        assert_eq!(call(uc, "GetSystemMetrics", &[1]), 600);
        assert_eq!(call(uc, "GetSystemMetrics", &[2]), 0);
        let rect = buf(uc, 16);
        call(uc, "GetWindowRect", &[0, rect]);
        assert_eq!((rd32(uc, rect + 8), rd32(uc, rect + 12)), (800, 600));
        assert_eq!(imp(uc, "ExitProcess", &[3]), Some(0));
        assert!(matches!(uc.get_data_mut().pending.take(), Some(Action::Exit(3))));
        uc.get_data_mut().last_error = 0;
        assert_eq!(call(uc, "GetLastError", &[]), 0);
        assert_eq!(call(uc, "NoSuchApi", &[1]), 0, "unknown imports return 0");
    }

    #[test]
    fn tls_slots_and_heap_calls() {
        let mut e = env("tls");
        let uc = &mut e.uc;
        let slot = call(uc, "TlsAlloc", &[]);
        assert!(slot >= 1);
        // The slot table is offset by 32 on both sides, so a Set/Get pair round-trips for small indices.
        assert_eq!(call(uc, "TlsSetValue", &[slot, 0x1234]), 1);
        assert_eq!(call(uc, "TlsGetValue", &[slot]), 0x1234);
        assert_eq!(call(uc, "TlsGetValue", &[100_000]), 0, "out of range reads as empty");
        assert_eq!(call(uc, "TlsSetValue", &[100_000, 1]), 1, "and writes are ignored");
        let m = call(uc, "GlobalAlloc", &[0, 100]);
        assert_eq!(rd_bytes(uc, m, 100), vec![0; 100]);
        assert_eq!(call(uc, "GlobalLock", &[m]), m);
        assert_eq!(call(uc, "GlobalFree", &[m]), 0);
        let v = call(uc, "VirtualAlloc", &[0, 4096]);
        assert_ne!(v, 0);
        assert_eq!(call(uc, "VirtualFree", &[v]), 1);
        let (a, b) = (call(uc, "CreateEventA", &[]), call(uc, "CreateMutexA", &[]));
        assert_ne!(a, b);
        assert_eq!(call(uc, "WaitForSingleObject", &[a, 0]), 0);
        assert_eq!(call(uc, "CreateThread", &[0, 0, 0x40_1000]), 0);
    }

    #[test]
    fn files_read_write_seek_and_delete() {
        let mut e = env("files");
        let uc = &mut e.uc;
        let p = cstr(uc, "C:\\RISING\\Maps\\a.txt");
        let h = call(uc, "CreateFileA", &[p, 0x8000_0000, 0, 0, 3]);
        assert_ne!(h, u32::MAX);
        let (data, got) = (buf(uc, 32), buf(uc, 4));
        assert_eq!(call(uc, "ReadFile", &[h, data, 5, got]), 1);
        assert_eq!((rd_bytes(uc, data, 5), rd32(uc, got)), (b"hello".to_vec(), 5));
        assert_eq!(call(uc, "SetFilePointer", &[h, 6, 0, 0]), 6);
        assert_eq!(call(uc, "_hread", &[h, data, 32]), 5);
        assert_eq!(rd_bytes(uc, data, 5), b"world");
        assert_eq!(call(uc, "_llseek", &[h, (-5i32) as u32, 2]), 6);
        assert_eq!(call(uc, "_llseek", &[h, (-2i32) as u32, 1]), 4);
        assert_eq!(call(uc, "CloseHandle", &[h]), 1);
        assert_eq!(call(uc, "ReadFile", &[h, data, 5, got]), 0, "closed handle");
        assert_eq!(call(uc, "_hread", &[h, data, 5]), u32::MAX);
        assert_eq!(call(uc, "SetFilePointer", &[h, 0, 0, 0]), u32::MAX);
        let missing = cstr(uc, "C:\\RISING\\nope.bin");
        assert_eq!(call(uc, "CreateFileA", &[missing, 0x8000_0000, 0, 0, 3]), u32::MAX);
        // Writing creates the file in the user overlay, and only that copy can be deleted.
        let sav = cstr(uc, "C:\\RISING\\SAVE\\x.sav");
        let h = call(uc, "CreateFileA", &[sav, 0x4000_0000, 0, 0, 2]);
        let src = cstr(uc, "SAVED");
        assert_eq!(call(uc, "WriteFile", &[h, src, 5, got]), 1);
        assert_eq!(rd32(uc, got), 5);
        assert_eq!(call(uc, "_hwrite", &[h, src, 3]), 3);
        call(uc, "_lclose", &[h]);
        assert_eq!(std::fs::read(e.root.join("user/SAVE/x.sav")).unwrap(), b"SAVEDSAV");
        let uc = &mut e.uc;
        assert_eq!(call(uc, "DeleteFileA", &[sav]), 1);
        assert_eq!(call(uc, "DeleteFileA", &[p]), 0, "installed files are not deletable");
        assert!(e.root.join("install/Maps/a.txt").exists());
        let h = call(uc, "OpenFile", &[p, 0, 0x1000]);
        assert_ne!(h, u32::MAX, "OF_CREATE opens for writing in the overlay");
        let newp = cstr(uc, "C:\\RISING\\new.dat");
        let (ro, rw, cr) = (call(uc, "_lopen", &[p, 0]), call(uc, "_lopen", &[p, 2]), call(uc, "_lcreat", &[newp, 0]));
        assert!(ro != u32::MAX && rw != u32::MAX && cr != u32::MAX);
    }

    #[test]
    fn directories_drives_and_find() {
        let mut e = env("dirs");
        let uc = &mut e.uc;
        let (c, d, z) = (cstr(uc, "C:\\"), cstr(uc, "d:\\"), cstr(uc, "Z:\\"));
        assert_eq!((call(uc, "GetDriveTypeA", &[c]), call(uc, "GetDriveTypeA", &[d]), call(uc, "GetDriveTypeA", &[z])), (3, 5, 1));
        let (label, serial, maxlen, flags, fs_name) = (buf(uc, 32), buf(uc, 4), buf(uc, 4), buf(uc, 4), buf(uc, 16));
        assert_eq!(call(uc, "GetVolumeInformationA", &[c, label, 32, serial, maxlen, flags, fs_name, 16]), 0);
        assert_eq!(call(uc, "GetVolumeInformationA", &[d, label, 32, serial, maxlen, flags, fs_name, 16]), 1);
        assert_eq!((rd_cstr(uc, label), rd_cstr(uc, fs_name), rd32(uc, maxlen)), ("RISING_LANDS_BR_".into(), "CDFS".into(), 255));
        let cwd = buf(uc, 64);
        assert_eq!(call(uc, "GetCurrentDirectoryA", &[64, cwd]), 9);
        assert_eq!(rd_cstr(uc, cwd), "C:\\RISING");
        let maps = cstr(uc, "C:\\RISING\\Maps");
        assert_eq!(call(uc, "SetCurrentDirectoryA", &[maps]), 1);
        let nowhere = cstr(uc, "C:\\nowhere");
        assert_eq!(call(uc, "SetCurrentDirectoryA", &[nowhere]), 0);
        // Find with a relative pattern, now that the cwd is Maps.
        let (pat, fd) = (cstr(uc, "*.txt"), buf(uc, 320));
        let h = call(uc, "FindFirstFileA", &[pat, fd]);
        assert_ne!(h, u32::MAX);
        assert_eq!((rd_cstr(uc, fd + 44), rd32(uc, fd), rd32(uc, fd + 32)), ("a.txt".into(), 0x80, 11));
        assert_eq!(call(uc, "FindNextFileA", &[h, fd]), 1);
        assert_eq!(rd_cstr(uc, fd + 44), "b.txt");
        assert_eq!(call(uc, "FindNextFileA", &[h, fd]), 0);
        assert_eq!(call(uc, "GetLastError", &[]), 18);
        assert_eq!(call(uc, "FindClose", &[h]), 1);
        assert_eq!(call(uc, "FindNextFileA", &[h, fd]), 0, "closed find handle");
        let zzz = cstr(uc, "*.zzz");
        assert_eq!(call(uc, "FindFirstFileA", &[zzz, fd]), u32::MAX);
        assert_eq!(call(uc, "GetLastError", &[]), 2);
        // A directory reports the directory attribute.
        let (rising, maps_dir) = (cstr(uc, "C:\\RISING"), cstr(uc, "Maps"));
        call(uc, "SetCurrentDirectoryA", &[rising]);
        call(uc, "FindFirstFileA", &[maps_dir, fd]);
        assert_eq!(rd32(uc, fd), 0x10);
    }

    #[test]
    fn window_classes_creation_and_messages() {
        let mut e = env("windows");
        let uc = &mut e.uc;
        let (cls, wc) = (cstr(uc, "RisingClass"), buf(uc, 48));
        wr32(uc, wc + 4, 0x40_2000);
        wr32(uc, wc + 36, cls);
        assert_eq!(call(uc, "RegisterClassA", &[wc]), 0xc001);
        // CreateWindowExA: arguments a[0..12]; class name in a[1]. The window procedure gets WM_CREATE.
        let mut args = [0u32; 12];
        args[1] = cstr(uc, "risingclass");
        let hwnd = call(uc, "CreateWindowExA", &args);
        assert_eq!(hwnd, 0x1000);
        match uc.get_data_mut().pending.take() {
            Some(Action::Call { target, args, set_eax, .. }) => assert_eq!((target, args[0], args[1], set_eax), (0x40_2000, 0x1000, 1, Some(0x1000))),
            _ => panic!("WM_CREATE not queued"),
        }
        assert_eq!(uc.get_data().windows.len(), 1);
        // SendMessage / DispatchMessage run the window procedure; unknown windows do nothing.
        assert_eq!(call(uc, "SendMessageA", &[hwnd, 0x111, 5, 6]), 0);
        assert!(matches!(uc.get_data_mut().pending.take(), Some(Action::Call { target: 0x40_2000, .. })));
        let msg = buf(uc, 28);
        for (i, v) in [hwnd, 0x100, 7, 8].iter().enumerate() {
            wr32(uc, msg + i as u32 * 4, *v);
        }
        call(uc, "DispatchMessageA", &[msg]);
        match uc.get_data_mut().pending.take() {
            Some(Action::Call { args, .. }) => assert_eq!(args, [hwnd, 0x100, 7, 8]),
            _ => panic!(),
        }
        call(uc, "SendMessageA", &[0x9999, 1, 2, 3]);
        assert!(uc.get_data().pending.is_none());
        // Message queue: Peek without remove leaves it; Get removes.
        uc.get_data_mut().msgs.push_back(Msg { hwnd, message: 0x201, wparam: 1, lparam: 2 });
        assert_eq!(call(uc, "PeekMessageA", &[msg, 0, 0, 0, 0]), 1);
        assert_eq!(rd32(uc, msg + 4), 0x201);
        assert_eq!(uc.get_data().msgs.len(), 1);
        assert_eq!(call(uc, "PeekMessageA", &[msg, 0, 0, 0, 1]), 1);
        assert!(uc.get_data().msgs.is_empty());
        // Empty queue: Get returns an idle message, Peek returns 0 after pacing the frame.
        assert_eq!(call(uc, "GetMessageA", &[msg, 0, 0, 0]), 1);
        assert_eq!((rd32(uc, msg), rd32(uc, msg + 4)), (0x1000, 0));
        uc.get_data_mut().frame_interval = std::time::Duration::from_millis(1);
        assert_eq!(call(uc, "PeekMessageA", &[msg, 0, 0, 0, 1]), 0);
        // After PostQuitMessage: Get answers WM_QUIT with 0, Peek with a WM_QUIT message.
        call(uc, "PostQuitMessage", &[0]);
        assert_eq!(call(uc, "GetMessageA", &[msg, 0, 0, 0]), 0);
        assert_eq!(rd32(uc, msg + 4), 0x12);
        assert_eq!(call(uc, "PeekMessageA", &[msg, 0, 0, 0, 1]), 1);
    }

    #[test]
    fn input_cursor_and_keys() {
        let mut e = env("input");
        let uc = &mut e.uc;
        let pt = buf(uc, 8);
        uc.get_data_mut().cursor = (-50, 9999);
        call(uc, "GetCursorPos", &[pt]);
        assert_eq!((rd32(uc, pt), rd32(uc, pt + 4)), (0, 599), "clamped to the 800x600 screen");
        call(uc, "SetCursorPos", &[300, 200]);
        call(uc, "GetCursorPos", &[pt]);
        assert_eq!((rd32(uc, pt), rd32(uc, pt + 4)), (300, 200));
        uc.get_data_mut().block_edges = true;
        uc.get_data_mut().cursor = (0, 599);
        call(uc, "GetCursorPos", &[pt]);
        assert_eq!((rd32(uc, pt), rd32(uc, pt + 4)), (3, 597), "kept off the edges when scrolling is off");
        uc.get_data_mut().cursor = (0, 100);
        call(uc, "GetCursorPos", &[pt]);
        assert_eq!((rd32(uc, pt), rd32(uc, pt + 4)), (3, 100), "clamped");
        call(uc, "SetCursorPos", &[3, 100]);
        assert_eq!(uc.get_data().cursor, (0, 100), "echoing the clamped position back keeps the real one");
        call(uc, "ShowCursor", &[1]);
        assert!(uc.get_data().sys_cursor);
        call(uc, "ShowCursor", &[0]);
        assert!(!uc.get_data().sys_cursor);
        uc.get_data_mut().block_edges = false;
        uc.get_data_mut().keys[0x41] = true;
        assert_eq!(call(uc, "GetAsyncKeyState", &[0x41]), 0x8001);
        assert_eq!(call(uc, "GetAsyncKeyState", &[0x42]), 0);
        assert_eq!(call(uc, "GetAsyncKeyState", &[0x1000]), 0, "out-of-range key codes are not pressed");
    }

    #[test]
    fn import_dispatch_yields_to_the_host_in_the_message_pump() {
        let mut e = env("yield");
        let uc = &mut e.uc;
        uc.get_data_mut().stubs.push(Stub { name: "GetMessageA".into() });
        uc.get_data_mut().stubs.push(Stub { name: "GetAsyncKeyState".into() });
        uc.get_data_mut().stubs.push(Stub { name: "GetVersion".into() });
        let msg = buf(uc, 28);
        let mut a = [0u32; 14];
        a[0] = msg;
        assert_eq!(call_import(uc, 0, 0x7f00_0300, &a), None);
        assert!(matches!(uc.get_data_mut().pending.take(), Some(Action::Resume { pc: 0x7f00_0300, set_eax: Some(1) })));
        // A released key does not yield; a pressed one does.
        a[0] = 0x41;
        assert_eq!(call_import(uc, 1, 0x7f00_0310, &a), Some(0));
        uc.get_data_mut().keys[0x41] = true;
        assert_eq!(call_import(uc, 1, 0x7f00_0310, &a), None);
        uc.get_data_mut().pending = None;
        assert_eq!(call_import(uc, 2, 0, &a), Some(0xc000_0a04));
        assert_eq!(uc.get_data().api_counts["GetVersion"], 1);
    }

    #[test]
    fn wsprintf_reads_cdecl_arguments_from_the_stack() {
        let mut e = env("wsprintf");
        let uc = &mut e.uc;
        let (out, fmt) = (buf(uc, 64), cstr(uc, "%s=%d"));
        let (key, esp) = (cstr(uc, "hp"), reg(uc, RegisterX86::ESP));
        // [esp] return address, [esp+4] buffer, [esp+8] format, [esp+12...] varargs.
        wr32(uc, esp + 12, key);
        wr32(uc, esp + 16, 90);
        assert_eq!(call(uc, "wsprintfA", &[out, fmt]), 5);
        assert_eq!(rd_cstr(uc, out), "hp=90");
    }

    #[test]
    fn scroll_click_assist_aligns_the_frame_counter_once_per_click() {
        let mut e = env("scroll");
        let uc = &mut e.uc;
        wr_bytes(uc, 0x4e2f78, &[0x11]);
        wr32(uc, 0x508f7c, 123);
        scroll_click_assist(uc);
        assert_eq!(rd32(uc, 0x508f7c), 123, "no click: untouched");
        wr_bytes(uc, 0x5099ae, &[1]);
        scroll_click_assist(uc);
        assert_eq!(rd32(uc, 0x508f7c), 129);
        wr32(uc, 0x508f7c, 140);
        scroll_click_assist(uc);
        assert_eq!(rd32(uc, 0x508f7c), 140, "same click: not realigned");
        wr_bytes(uc, 0x5099ae, &[0]);
        scroll_click_assist(uc);
        wr_bytes(uc, 0x4e2f78, &[0x05]);
        wr_bytes(uc, 0x5099ae, &[1]);
        scroll_click_assist(uc);
        assert_eq!(rd32(uc, 0x508f7c), 140, "only on the mission text screens");
    }

    #[test]
    fn frame_pacing_sleeps_up_to_the_frame_budget() {
        let mut e = env("pace");
        let uc = &mut e.uc;
        uc.get_data_mut().frame_interval = std::time::Duration::from_millis(40);
        pace_frame(uc); // first call only sets the deadline
        let t0 = std::time::Instant::now();
        pace_frame(uc);
        assert!(t0.elapsed() >= std::time::Duration::from_millis(25), "{:?}", t0.elapsed());
        // At 4x speed the budget shrinks to a quarter.
        uc.get_data_mut().speed = 4.0;
        pace_frame(uc);
        let t0 = std::time::Instant::now();
        pace_frame(uc);
        assert!(t0.elapsed() < std::time::Duration::from_millis(30));
    }

    #[test]
    fn cd_audio_through_mci_strings() {
        let mut e = env("mci");
        // 2 s of stereo 16-bit silence for track 2.
        std::fs::write(e.root.join("disc/track02.raw"), vec![0u8; 44100 * 4 * 2]).unwrap();
        let uc = &mut e.uc;
        let (ret, mci) = (buf(uc, 32), |uc: &mut Uc, ret: u32, c: &str| {
            let p = cstr(uc, c);
            call(uc, "mciSendStringA", &[p, ret, 32, 0])
        });
        assert_eq!(mci(uc, ret, "open cd type cdaudio alias cd"), 0);
        assert_eq!(mci(uc, ret, "status cd number of tracks"), 0);
        assert_eq!(rd_cstr(uc, ret), "3");
        assert_eq!(mci(uc, ret, "status cd length track 2"), 0);
        assert_eq!(rd_cstr(uc, ret), "2000");
        assert_eq!(mci(uc, ret, "set cd time format tmsf"), 0);
        assert_eq!(mci(uc, ret, "status cd length track 2"), 0);
        assert_eq!(rd_cstr(uc, ret), "00:02:00");
        assert_eq!(mci(uc, ret, "status cd length track 9"), 263, "missing track");
        assert_eq!(mci(uc, ret, "play cd from 258"), 0, "tmsf: the low byte is the track");
        assert!(uc.get_data().mixer.lock().unwrap().cd.is_some());
        assert_eq!(mci(uc, ret, "stop cd"), 0);
        assert!(uc.get_data().mixer.lock().unwrap().cd.is_none());
        assert_eq!(mci(uc, ret, "play cd from 7"), 263);
        assert_eq!(mci(uc, ret, "close cd"), 0);
        assert_eq!(mci(uc, ret, "frobnicate"), 263);
        assert_eq!(mci(uc, 0, "status cd number of tracks"), 0, "no reply buffer is fine");
        assert_eq!(call(uc, "MCIWndCreateA", &[]), 0);
    }

    #[test]
    fn movie_playback_through_mci_commands() {
        let mut e = env("movie");
        std::fs::write(e.root.join("disc/intro.avi"), b"RIFF\0\0\0\0AVI ").unwrap();
        let uc = &mut e.uc;
        let (params, name) = (buf(uc, 32), cstr(uc, "D:\\intro.avi"));
        wr32(uc, params + 12, name);
        // Open: the movie is parsed and parked until MCI_PLAY.
        assert_eq!(call(uc, "mciSendCommandA", &[0, 0x803, 0, params]), 0);
        assert_eq!(rd32(uc, params + 4), 1);
        assert!(uc.get_data().video_open.is_some());
        assert_eq!(call(uc, "mciSendCommandA", &[1, 0x806, 0, params]), 0);
        assert!(uc.get_data().video.is_some() && uc.get_data().video_open.is_none());
        call(uc, "mciSendCommandA", &[1, 0x814, 0, params]);
        assert_eq!(rd32(uc, params + 4), 0x20e, "playing");
        uc.get_data_mut().video.as_mut().unwrap().finished = true;
        call(uc, "mciSendCommandA", &[1, 0x814, 0, params]);
        assert_eq!(rd32(uc, params + 4), 0x20d, "stopped");
        assert_eq!(call(uc, "mciSendCommandA", &[1, 0x804, 0, params]), 0);
        assert!(uc.get_data().video.is_none());
        // A missing movie (or --skip-intro) reports success without video, so the game carries on.
        let nope = cstr(uc, "D:\\nope.avi");
        wr32(uc, params + 12, nope);
        assert_eq!(call(uc, "mciSendCommandA", &[0, 0x803, 0, params]), 0);
        assert!(uc.get_data().video_open.is_none());
        uc.get_data_mut().skip_movies = true;
        wr32(uc, params + 12, name);
        call(uc, "mciSendCommandA", &[0, 0x803, 0, params]);
        assert!(uc.get_data().video_open.is_none());
        assert_eq!(call(uc, "mciSendCommandA", &[0, 0x999, 0, params]), 0);
    }

    #[test]
    fn directx_entry_points_create_objects() {
        let mut e = env("dx");
        let uc = &mut e.uc;
        let out = buf(uc, 4);
        assert_eq!(call(uc, "DirectDrawCreate", &[0, out]), 0);
        assert_ne!(rd32(uc, out), 0);
        assert_eq!(call(uc, "DirectSoundCreate", &[0, out]), 0);
        assert_eq!(call(uc, "#1", &[0, out]), 0);
        assert_eq!(imp(uc, "#2", &[0x40_1000, 0x55]), None, "DirectPlayEnumerate calls back into the guest");
        assert!(uc.get_data_mut().pending.take().is_some());
        let (text, title) = (cstr(uc, "text"), cstr(uc, "title"));
        assert_eq!(call(uc, "MessageBoxA", &[0, text, title, 0]), 1);
        assert_eq!(call(uc, "GetOpenFileNameA", &[0]), 0);
    }
}
