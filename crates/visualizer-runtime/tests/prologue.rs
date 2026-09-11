//! The generated GDB prologue must load, reload and fail safely. It runs the real
//! build.rs output under a stub `gdb` module; no debugger or live process is needed.
use std::process::Command;

#[test]
fn generated_prologue_reloads_safely() {
    let output = Command::new("python3")
        .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/prologue.py"))
        .arg(concat!(env!("OUT_DIR"), "/dbgvis_gdb_v2.py"))
        .output()
        .expect("python3 is required for the GDB prologue test");
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
