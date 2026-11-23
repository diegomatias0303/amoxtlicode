use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=assets/amoxlicode.rc");
    println!("cargo:rerun-if-changed=assets/icon.ico");
    println!("cargo:rerun-if-changed=assets/icon.png");

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os != "windows" {
        return;
    }

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let rc_file = manifest_dir.join("assets").join("amoxlicode.rc");
    let ico_file = manifest_dir.join("assets").join("icon.ico");
    if !rc_file.exists() || !ico_file.exists() {
        return;
    }

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let res_file = out_dir.join("amoxlicode.res");

    // Intentar primero llvm-rc, luego rc.exe (Windows SDK)
    let compiled = try_compile_rc("llvm-rc", &rc_file, &res_file)
        || try_compile_rc("rc", &rc_file, &res_file);

    if compiled && res_file.exists() {
        println!("cargo:rustc-link-arg-bins={}", res_file.display());
    }
}

fn try_compile_rc(tool: &str, rc_file: &PathBuf, res_file: &PathBuf) -> bool {
    let mut cmd = Command::new(tool);
    if tool == "llvm-rc" {
        cmd.arg("/FO").arg(res_file).arg(rc_file);
    } else {
        cmd.arg("/nologo").arg("/fo").arg(res_file).arg(rc_file);
    }
    matches!(cmd.status(), Ok(s) if s.success())
}
