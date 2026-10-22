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
    let r = call_import_inner(uc, idx, addr, a);
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
            uc.get_data_mut().windows.push(Window { class: cls, proc_ });
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
            uc.get_data_mut().cursor = (a[0] as i32, a[1] as i32);
            1
        }
        "ShowWindow" | "UpdateWindow" | "MoveWindow" | "SetFocus" | "MessageBeep" => 1,
        "ShowCursor" => 0,
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
            // Always inside the 800x600 screen: the original indexed tables with the raw cursor
            // position and crashed when it came back from outside the window (archer bug).
            let (x, y) = uc.get_data().cursor;
            let (x, y) = (x.clamp(0, 799), y.clamp(0, 599));
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

/// The game loop polls for messages once per pass; hold it to the frame budget so the
/// game runs at the intended speed instead of as fast as the host allows.
fn pace_frame(uc: &mut Uc) {
    let now = std::time::Instant::now();
    let (next, interval) = (uc.get_data().next_frame, uc.get_data().frame_interval);
    if next > now {
        std::thread::sleep(next - now);
        uc.get_data_mut().next_frame = next + interval;
    } else {
        // Fell behind (or first call): don't try to catch up with a burst of frames.
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
            let Some(host) = uc.get_data().vfs.resolve(&name) else {
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
