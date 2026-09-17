# Rust debugger visualizer

`dbgvis` renders a value's *logical* view — a type's `Visualize`, else `Debug`, else
`Display` — live in the debugger at a breakpoint, instead of reaching for `dbg!` or
`tracing`. The formatter runs in-process and the debugger just asks for it; `HashMap`/`Vec`
and the like go through their public iteration APIs, recursively, with no paging to write.

## Requirements

- **A nightly toolchain with the `rustc-dev` component.** The runtime uses
  `#![feature(specialization)]`, and the auto-registration driver links `rustc_private`.
  `rustup toolchain install nightly --component rustc-dev`. (The `cargo-dbgvis` CLI itself
  builds on stable.)
- **Linux x86_64.** GDB is the primary frontend — its script is embedded into the artifact
  automatically. An experimental LLDB bridge is loaded by hand. Apple/MSVC targets do not
  emit `.debug_gdb_scripts`.
- **Debug info.** The final binary must keep `debug = 2` and stay unstripped, or the
  embedded script and type anchors both disappear.

## Quick start

The `explicit` example registers its types by hand, so it needs no driver:

```sh
cargo build --example explicit
rust-gdb -iex "add-auto-load-safe-path $PWD/target/debug/examples/explicit" target/debug/examples/explicit
```

```text
break explicit::checkpoint
run
up
dbgvis print app            # AppState { points: {"app": [Some(Point { x: 5, y: 6 })]}, ... }
dbgvis print --all          # every registered local in the frame (alias: dbgvis all)
dbgvis types                # list registered types
```

Use the binary's real absolute path in the safe-path, not `*`. The GDB script ships inside
the runtime, so a consumer project needs no `build.rs` or Python copy of its own.

## Use it in your project

### Recommended: the `cargo dbgvis` driver

The driver auto-registers the concrete types a debug session wants, with no hand-written
`register_type!`:

```sh
cargo install cargo-dbgvis                      # the CLI itself builds on stable
cargo dbgvis setup                              # under your nightly: build the driver, record its toolchain
cargo dbgvis config my-bin >> .cargo/config.toml
cargo +nightly build                            # build through the driver (same nightly as setup)
```

`cargo dbgvis config` writes a `.cargo/config.toml` pointing the build at the driver
(`rustc-workspace-wrapper`, `DBGVIS_HOME`, `DBGVIS_AUTO_CRATE`). By default the driver
scans the executable's own functions; `DBGVIS_SCAN_DEPS=1` or `DBGVIS_SCAN_CRATES=regex.*`
extends the scan into libraries (at the cost of keeping their MIR).

| Command | What it does |
| --- | --- |
| `setup` | compile the driver into `DBGVIS_HOME`, record its toolchain |
| `doctor` | check nightly, `rustc-dev`, and that the driver matches the toolchain |
| `config <bin>` | print the `.cargo/config.toml` snippet for a project |
| `script [--gdb\|--lldb\|--all] [DIR]` | write the debugger bridge(s) to a dir (default `target/dbgvis`) |
| `home` / `uninstall` | print / remove `DBGVIS_HOME` |

`setup` is not pinned to one build: run it under any nightly with `rustc-dev` and it
compiles a matching driver and records it; the wrapper then requires the project to build
under that same nightly, and `cargo dbgvis doctor` verifies it.

### Alternative: register by hand

Skip the driver, add the dependency (`cargo add dbgvis --features derive`), register roots
yourself, then `enable!()`:

```rust
#[derive(dbgvis::Visualize)] struct AppState { /* ... */ }     // a non-generic derive also registers a root
#[dbgvis::register] #[derive(Debug)] struct Cfg { /* ... */ }  // reuse an existing Debug
dbgvis::register_type!(bytes::Bytes);                          // a third-party or generic concrete instance
fn main() { dbgvis::enable!(); /* ... */ }
```

Roots are collected across crates by `linkme`; `main` needs only `enable!()`, no central
module. `#[dbgvis(no_register)]` turns a type's auto-root off. The [usage
guide](docs/usage-guide.md) covers field attributes and feature-gating.

## Debugging with LLDB (experimental)

LLDB has no `.debug_gdb_scripts` auto-load, so generate the bridge and load it by hand:

```sh
cargo dbgvis script --lldb          # writes target/dbgvis/lldb.py
```

```text
(lldb) command script import target/dbgvis/lldb.py
(lldb) dbgvis print app
(lldb) dbgvis all
```

Same commands and mailbox as GDB, and it works under VSCode CodeLLDB via `initCommands`.
Type matching is name-based rather than the GDB bridge's DWARF-shape comparison, so it
refuses types LLDB cannot tell apart (e.g. differing only by lifetime or const generic).

## Documentation

- [Usage guide](docs/usage-guide.md) — setup, the registration API, debugger commands, safety.
- [Auto-registration experiment](docs/auto-registration-experiment.md) — the driver, its scope and limits.
- [Protocol v2](docs/protocol-v2.md) — the mailbox ABI, for reference.
- [Phase-two acceptance record](docs/phase-two-acceptance.md) — tests, known limits, open items.
- [Q0 validation](docs/archive/q0-validation.md) — why nightly (archived).

## Verification

```sh
cargo fmt --all -- --check
cargo clippy --offline --workspace --all-targets --all-features -- -D warnings
cargo test --offline --workspace --all-features
cargo xtask compiler
cargo xtask integration --faults --lto --relocated
cargo xtask autoregister --gdb
```

Verified on Linux x86_64, GDB 17.1, rustc 1.99.0-nightly (12c36e253 2026-08-10). The
integration suite needs local ptrace permission and is skipped on non-Linux. A value with
none of the three capabilities renders as a `<unformattable T>` placeholder instead of
failing compilation; explicit `via` and root modes are still checked at compile time.
`dbgvis`, `dbgvis-runtime`, `dbgvis-macros`, and `cargo-dbgvis` are published on crates.io
(0.2.x); the backend is still experimental.
