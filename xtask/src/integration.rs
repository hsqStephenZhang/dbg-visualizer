//! Real rust-gdb scenarios against the built examples.
//!
//! Port of the former `tests/integration/run.py`. The GDB-side assertions stay in
//! Python under `tests/inferior/` because they run inside GDB's interpreter.

use crate::harness::{Failure, Result, Run, TempDir, root};
use std::path::Path;

pub struct Options {
    pub faults: bool,
    pub lto: bool,
    pub relocated: bool,
}

/// Build the rust-gdb invocation for one inferior script.
fn debugger(binary: &Path, script: &str, extra: &[&str]) -> Run {
    Run::new("rust-gdb")
        .args(["-q", "-nx", "--batch", "-iex"])
        .arg(format!("add-auto-load-safe-path {}", binary.display()))
        .arg("-x")
        .arg(root().join("tests/inferior").join(script))
        .arg("--args")
        .arg(binary)
        .args(extra)
        .timeout(45)
}

fn cargo_example(name: &str, profile: &[&str]) -> Run {
    Run::new("cargo")
        .args(["build", "--offline", "--example", name])
        .args(profile)
}

pub fn run(options: &Options) -> Result {
    if !cfg!(target_os = "linux") {
        // Apple/MSVC rustc targets never emit `.debug_gdb_scripts`, and GDB cannot ptrace here.
        println!("GDB_V2_SUITE_SKIPPED: live GDB auto-load scenarios require Linux");
        return Ok(());
    }
    let root = root();

    cargo_example("explicit", &[]).check()?;
    debugger(
        &root.join("target/debug/examples/explicit"),
        "explicit.py",
        &[],
    )
    .marker("GDB_V2_INTEGRATION_OK")
    .check()?;

    if options.lto {
        cargo_example("explicit", &["--profile", "release-lto"]).check()?;
        debugger(
            &root.join("target/release-lto/examples/explicit"),
            "explicit.py",
            &[],
        )
        .marker("GDB_V2_INTEGRATION_OK")
        .check()?;
    }

    if options.relocated {
        let scratch = TempDir::new("dbgvis relocated ")?;
        // A path with a space, to keep the auto-load safe-path quoting honest.
        let binary = scratch.path().join("explicit binary");
        std::fs::copy(root.join("target/debug/examples/explicit"), &binary)
            .map_err(|error| Failure(format!("copy relocated: {error}")))?;
        debugger(&binary, "explicit.py", &[])
            .marker("GDB_V2_INTEGRATION_OK")
            .check()?;
    }

    if options.faults {
        cargo_example("faults", &[]).check()?;
        for kind in ["panic", "error", "hang", "exit"] {
            debugger(
                &root.join("target/debug/examples/faults"),
                "faults.py",
                &[kind],
            )
            .marker("GDB_V2_FAULT_OK")
            .check()?;
        }
        cargo_example("faults", &["--profile", "abort"]).check()?;
        debugger(
            &root.join("target/abort/examples/faults"),
            "faults.py",
            &["panic"],
        )
        .marker("GDB_V2_FAULT_OK")
        .check()?;
    }

    println!("GDB_V2_SUITE_OK");
    Ok(())
}
