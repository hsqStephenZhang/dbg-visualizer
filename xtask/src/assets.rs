//! Keeps `crates/cargo-dbgvis/assets/` in sync with the canonical embedded files.
//!
//! `cargo-dbgvis` publishes to crates.io, where `cargo publish` packages only files
//! under the crate root, so it embeds committed copies of the driver sources and the
//! debugger bridges rather than reaching out with `include_str!("../../..")`. This gate
//! fails if a copy drifts from its source; `cargo xtask assets --sync` refreshes them.

use crate::harness::{root, Failure, Result};

/// (canonical source, vendored copy) relative to the repo root.
const PAIRS: &[(&str, &str)] = &[
    ("tools/auto-register/wrapper.py", "crates/cargo-dbgvis/assets/wrapper.py"),
    ("tools/auto-register/driver.rs", "crates/cargo-dbgvis/assets/driver.rs"),
    ("tools/auto-register/tracked.rs", "crates/cargo-dbgvis/assets/tracked.rs"),
    ("crates/dbgvis-runtime/gdb.py", "crates/cargo-dbgvis/assets/gdb.py"),
    ("crates/dbgvis-runtime/lldb.py", "crates/cargo-dbgvis/assets/lldb.py"),
];

pub fn run(sync: bool) -> Result {
    let base = root();
    let mut stale = Vec::new();
    for (source, vendored) in PAIRS {
        let want = std::fs::read(base.join(source))
            .map_err(|error| Failure(format!("read {source}: {error}")))?;
        let have = std::fs::read(base.join(vendored)).ok();
        if have.as_deref() == Some(want.as_slice()) {
            continue;
        }
        if sync {
            std::fs::write(base.join(vendored), &want)
                .map_err(|error| Failure(format!("write {vendored}: {error}")))?;
            println!("synced {vendored}");
        } else {
            stale.push(*vendored);
        }
    }
    if !stale.is_empty() {
        return Err(Failure(format!(
            "vendored assets are stale (run `cargo xtask assets --sync`):\n  {}",
            stale.join("\n  ")
        )));
    }
    println!("ASSETS_OK");
    Ok(())
}
