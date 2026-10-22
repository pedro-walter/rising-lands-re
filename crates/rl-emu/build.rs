//! On Linux the system SDL2 is linked. Without `libsdl2-dev` there is no `libSDL2.so` for the
//! linker, but the runtime library (`libSDL2-2.0.so.0`) is usually there: link against that.

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("linux") {
        return;
    }
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    for dir in ["/usr/lib/x86_64-linux-gnu", "/usr/lib/aarch64-linux-gnu", "/usr/lib64", "/usr/lib"] {
        let dir = std::path::Path::new(dir);
        if dir.join("libSDL2.so").exists() {
            return;
        }
        let rt = dir.join("libSDL2-2.0.so.0");
        if rt.exists() {
            let link = out.join("libSDL2.so");
            let _ = std::fs::remove_file(&link);
            if std::os::unix::fs::symlink(&rt, &link).is_ok() {
                println!("cargo:rustc-link-search=native={}", out.display());
            }
            return;
        }
    }
}
