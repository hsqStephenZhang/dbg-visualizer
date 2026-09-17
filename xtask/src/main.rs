//! Repository verification gates.
//!
//! Every gate that drives subprocesses lives here; only scripts that run *inside*
//! GDB's own interpreter remain Python (`tests/inferior/`, plus the embedded bridge
//! `crates/dbgvis-runtime/gdb.py` and the `RUSTC_WORKSPACE_WRAPPER` shim).
//!
//! ```text
//! cargo xtask compiler       # offline consumer compile contracts
//! cargo xtask integration    # real rust-gdb scenarios   [--faults --lto --relocated]
//! cargo xtask autoregister   # experimental two-pass registration driver  [--gdb]
//! cargo xtask visibility     # module/function/type visibility matrix     [--gdb]
//! cargo xtask all            # everything above, GDB gates included
//! ```

#[macro_use]
mod harness;
mod assets;
mod autoregister;
mod compiler;
mod integration;
mod visibility;

use harness::Result;

const USAGE: &str = "\
cargo xtask <gate> [options]

  compiler                              offline consumer compile contracts
  integration [--faults --lto --relocated]  real rust-gdb scenarios
  autoregister [--gdb]                  experimental two-pass registration driver
  visibility [--gdb]                    module/function/type visibility matrix
  assets [--sync]                       verify/refresh cargo-dbgvis vendored assets
  driver                                build the auto-register rustc driver only
  all                                   every gate above, with --gdb and all scenarios
  help                                  this message";

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let (gate, flags) = match arguments.split_first() {
        Some((gate, rest)) => (gate.as_str(), rest),
        None => {
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    };
    let has = |flag: &str| flags.iter().any(|value| value == flag);

    let result: Result = match gate {
        "help" | "-h" | "--help" => {
            println!("{USAGE}");
            return;
        }
        "compiler" => compiler::run(),
        "integration" => integration::run(&integration::Options {
            faults: has("--faults"),
            lto: has("--lto"),
            relocated: has("--relocated"),
        }),
        "autoregister" => autoregister::run(has("--gdb")),
        "visibility" => visibility::run(has("--gdb")),
        "assets" => assets::run(has("--sync")),
        "driver" => autoregister::build_driver(),
        "all" => run_all(),
        other => {
            eprintln!("unknown gate `{other}`\n\n{USAGE}");
            std::process::exit(2);
        }
    };

    if let Err(failure) = result {
        eprintln!("\nxtask {gate} FAILED\n{failure}");
        std::process::exit(1);
    }
}

fn run_all() -> Result {
    compiler::run()?;
    integration::run(&integration::Options {
        faults: true,
        lto: true,
        relocated: true,
    })?;
    autoregister::run(true)?;
    println!("XTASK_ALL_OK");
    Ok(())
}
