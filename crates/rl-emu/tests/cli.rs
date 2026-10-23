//! Runs the `rl-emu` binary against a tiny generated executable: argument handling, headless
//! scripting, exit codes and error reporting, without needing the game files.

use std::{
    path::{Path, PathBuf},
    process::{Command, Output},
};

/// A PE32 with `code` at 0x401000 and the named imports from `KERNEL32.dll` (IAT slot `i` is at
/// 0x402100 + 4 * i). Mirrors the generator used by the unit tests.
fn exe(code: &[u8], imports: &[&str]) -> Vec<u8> {
    let mut d = vec![0u8; 0x1600];
    let put32 = |d: &mut [u8], at: usize, v: u32| d[at..at + 4].copy_from_slice(&v.to_le_bytes());
    d[..2].copy_from_slice(b"MZ");
    put32(&mut d, 0x3c, 0x40);
    d[0x40..0x44].copy_from_slice(b"PE\0\0");
    d[0x46..0x48].copy_from_slice(&2u16.to_le_bytes());
    d[0x54..0x56].copy_from_slice(&0xe0u16.to_le_bytes());
    let opt = 0x58;
    d[opt..opt + 2].copy_from_slice(&0x10bu16.to_le_bytes());
    put32(&mut d, opt + 16, 0x1000);
    put32(&mut d, opt + 28, 0x40_0000);
    put32(&mut d, opt + 56, 0x4000);
    put32(&mut d, opt + 60, 0x200);
    put32(&mut d, opt + 104, 0x2000);
    for (i, (va, raw, size)) in [(0x1000u32, 0x200u32, 0x1000u32), (0x2000, 0x1200, 0x400)].into_iter().enumerate() {
        let s = opt + 0xe0 + i * 40;
        put32(&mut d, s + 8, size);
        put32(&mut d, s + 12, va);
        put32(&mut d, s + 16, size);
        put32(&mut d, s + 20, raw);
    }
    d[0x200..0x200 + code.len()].copy_from_slice(code);
    put32(&mut d, 0x1200, 0x2100);
    put32(&mut d, 0x1200 + 12, 0x2180);
    put32(&mut d, 0x1200 + 16, 0x2100);
    d[0x1380..0x138d].copy_from_slice(b"KERNEL32.dll\0");
    for (i, name) in imports.iter().enumerate() {
        let rva = 0x2200 + i * 0x20;
        put32(&mut d, 0x1300 + i * 4, rva as u32);
        let at = 0x1200 + (rva - 0x2000) + 2;
        d[at..at + name.len()].copy_from_slice(name.as_bytes());
    }
    d
}

fn call_slot(i: u32) -> Vec<u8> {
    [&[0xffu8, 0x15][..], &(0x40_2100 + 4 * i).to_le_bytes()].concat()
}

fn push(v: u32) -> Vec<u8> {
    [&[0x68u8][..], &v.to_le_bytes()].concat()
}

/// A guest that polls the message queue forever, like the game's main loop (each poll hands control to the host).
fn idle_exe() -> Vec<u8> {
    let slot = 0x40_2100u32.to_le_bytes();
    let code = [&[0x6a, 0x01, 0x6a, 0x00, 0x6a, 0x00, 0x6a, 0x00, 0x68, 0x00, 0x31, 0x40, 0x00, 0xff, 0x15][..], &slot, &[0xeb, 0xeb]].concat();
    exe(&code, &["PeekMessageA"])
}

struct Sandbox {
    root: PathBuf,
}

impl Sandbox {
    fn new(name: &str) -> Sandbox {
        let root = std::env::temp_dir().join(format!("rl-cli-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("install")).unwrap();
        std::fs::create_dir_all(root.join("user")).unwrap();
        Sandbox { root }
    }

    fn with_exe(self, bytes: &[u8]) -> Sandbox {
        std::fs::write(self.root.join("install/RISING.EXE"), bytes).unwrap();
        self
    }

    fn path(&self, p: &str) -> PathBuf {
        self.root.join(p)
    }

    /// Runs rl-emu headless with the sandbox as game dir, disc and user dir.
    fn run(&self, args: &[&str], env: &[(&str, &str)]) -> Output {
        let mut c = Command::new(env!("CARGO_BIN_EXE_rl-emu"));
        c.args(args)
            .arg(self.path("install"))
            .arg(&self.root)
            .env("RL_USERDIR", self.path("user"))
            .env("RL_HEADLESS", "1")
            .env_remove("RL_SCRIPT")
            .env_remove("RL_SECS")
            .env_remove("RL_SHOT");
        for (k, v) in env {
            c.env(k, v);
        }
        c.output().unwrap()
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

#[test]
fn missing_game_files_print_usage_and_exit_2() {
    let s = Sandbox::new("usage");
    let o = s.run(&[], &[]);
    assert_eq!(o.status.code(), Some(2));
    assert!(stderr(&o).contains("Cannot find RISING.EXE"), "{}", stderr(&o));
}

#[test]
fn an_invalid_executable_is_reported_as_an_error() {
    let s = Sandbox::new("bad").with_exe(b"not a PE file");
    let o = s.run(&[], &[]);
    assert_eq!(o.status.code(), Some(1));
    assert!(stderr(&o).contains("error:"), "{}", stderr(&o));
}

#[test]
fn guest_exit_code_ends_the_run_successfully() {
    let code = [push(3), call_slot(0)].concat();
    let s = Sandbox::new("exit").with_exe(&exe(&code, &["ExitProcess"]));
    let o = s.run(&["--skip-intro"], &[]);
    assert!(o.status.success());
    assert!(stderr(&o).contains("guest exited with 3"), "{}", stderr(&o));
}

#[test]
fn guest_faults_fail_the_run_and_name_the_address() {
    let s = Sandbox::new("fault").with_exe(&exe(&[0xa1, 0x78, 0x56, 0x34, 0x12], &[]));
    let o = s.run(&[], &[]);
    assert_eq!(o.status.code(), Some(1));
    assert!(stderr(&o).contains("guest fault READ_UNMAPPED"), "{}", stderr(&o));
}

#[test]
fn scripts_run_and_quit_ends_the_session() {
    let s = Sandbox::new("script").with_exe(&idle_exe());
    let script = "0.1:move:10,20;0.15:click:30,40;0.2:key:13;0.25:dbg:poke 403000 5 1;0.3:mem:403000:2;0.35:shot:;0.4:quit";
    let o = s.run(&[], &[("RL_SCRIPT", script)]);
    assert!(o.status.success(), "{}", stderr(&o));
    let err = stderr(&o);
    assert!(err.contains("[dbg] 0x403000 = 5"), "{err}");
    assert!(err.contains("[mem 403000] [05, 00]"), "{err}");
}

#[test]
fn timed_runs_finish_and_report_the_busiest_calls() {
    let s = Sandbox::new("timed").with_exe(&idle_exe());
    let shot = s.path("shot.png");
    let o = s.run(&[], &[("RL_SECS", "0.3"), ("RL_SHOT", shot.to_str().unwrap())]);
    assert!(o.status.success(), "{}", stderr(&o));
    let err = stderr(&o);
    assert!(err.contains("no primary surface to dump"), "{err}");
    assert!(err.contains("top calls:"), "{err}");
}

#[test]
fn launch_flags_are_accepted_and_the_last_host_is_loaded() {
    let s = Sandbox::new("flags").with_exe(&idle_exe());
    std::fs::write(s.path("user/lasthost.txt"), "192.168.0.7\n").unwrap();
    for flags in [&["--new-game"][..], &["--mission", "5"], &["--map", "WORLD/MONDE12.ETA"], &["--mission", "99"]] {
        let o = s.run(flags, &[("RL_SECS", "0.2")]);
        assert!(o.status.success(), "{flags:?}: {}", stderr(&o));
    }
}

#[test]
fn avi_dump_mode_writes_nothing_for_an_empty_movie() {
    let s = Sandbox::new("avi");
    let avi = s.path("m.avi");
    std::fs::write(&avi, b"RIFF\0\0\0\0AVI ").unwrap();
    let out = s.path("f");
    let o = Command::new(env!("CARGO_BIN_EXE_rl-emu")).args(["--avi", avi.to_str().unwrap(), out.to_str().unwrap(), "0", "1"]).output().unwrap();
    assert!(o.status.success());
    assert!(stderr(&o).contains("0 frames"), "{}", stderr(&o));
    assert!(!Path::new(&format!("{}0000.png", out.display())).exists());
}
