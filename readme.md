# Rust debugger visualizer

`#[derive(dbgvis::Visualize)]` renders a value's *logical* view recursively, inside
Rust: an ordinary field prefers `Visualize`, then falls back to `Debug`, then
`Display`. `HashMap`/`Vec` and the like go through their public iteration APIs -- no
parsing of internal storage, and no paging for the user to write.

A non-generic derive also registers a root entry, collected across crates by `linkme`;
`main` only needs `dbgvis::enable!()`, with no central registration module. Reuse
`Debug` on a type with `#[dbgvis::register]`; register a third-party or generic concrete
instance with `dbgvis::register_type!(T)`. The type-level `#[dbgvis(no_register)]` turns
auto-registration off.

When several crates register the same concrete type, the runtime compares concrete-type
`TypeId`s before merging their modes and formatter functions. A borrowed root uses, as
its identity, the same type with its free lifetimes replaced by `'static`, while the
formatter stays lifetime-generic. A same-name but different-identity type is rejected
even when its size/align match: names and layout are not type identity.

The current backend is **nightly specialization + GDB v2**, with no v1 compatibility
layer. An **experimental LLDB bridge** drives the same mailbox ABI: `cargo dbgvis script
--lldb` writes `target/dbgvis/lldb.py`, which you load by hand (`command script import
target/dbgvis/lldb.py`) since LLDB has no `.debug_gdb_scripts` auto-load. It reuses the
runtime unchanged and offers `dbgvis print EXPR`; type matching reconciles LLDB's type
names with the registry's Rust spellings (LLDB gives no type for an anchor symbol, so the
GDB bridge's DWARF-shape comparison is unavailable) and refuses types LLDB cannot tell
apart, such as ones differing only by lifetime or const generic.
[examples/explicit.rs](examples/explicit.rs) shows `derive(Visualize)`
plus explicit registration; [examples/demo.rs](examples/demo.rs) keeps `Debug` types and
registers them through the experimental driver. Neither example is feature-gated; the
runtime still defaults to `native`/`manual`, and loading the script does not call target
functions on its own. The optional per-project feature setup is in the usage guide.

## Quick start

```sh
cargo build --example explicit
rust-gdb -iex "add-auto-load-safe-path /absolute/path/dbg-visualizer/target/debug/examples/explicit" target/debug/examples/explicit
```

Replace the safe-path with the binary's real absolute path; do not use `*`. The GDB
script ships inside the runtime and is embedded into the artifact, so a consumer project
needs no `build.rs` or Python copy of its own.

```text
break explicit::checkpoint
run
up
dbgvis print app
dbgvis print map
dbgvis print external_map
dbgvis print --mode display address
dbgvis print --buffer 8 map
dbgvis config summary.mode auto
dbgvis config execution automatic
print point
```

`map` renders as:

```text
{"points": [Some(Point { x: 1, y: 2 }), None]}
```

The self-contained `AppState` exercises third-party fields, `skip`, and nested
containers. There are also three `IndexMap` instances, a `hashbrown::HashMap`, `Bytes`,
`SocketAddr`, borrowed/const generics, and a ZST hasher with no `Debug`. Cross-crate and
feature-toggle coverage is driven from temporary projects the test suite generates; the
workspace keeps only the three core crates plus an `xtask` verification entry point.

To run `demo`, which needs no hand-written root registration, from the repository root
(the driver needs a nightly toolchain with `rustc-dev`; it is compiled against whatever
nightly is active and the build must then use that same nightly -- last verified on
`rustc 1.99.0-nightly (12c36e253 2026-08-10)`):

```sh
cargo xtask driver
DBGVIS_AUTO_CRATE=demo RUSTC_WORKSPACE_WRAPPER="$PWD/tools/auto-register/wrapper.py" CARGO_TARGET_DIR=target/auto-register/examples cargo build --example demo
rust-gdb -iex "add-auto-load-safe-path /absolute/path/dbg-visualizer/target/auto-register/examples/debug/examples/demo" target/auto-register/examples/debug/examples/demo
```

Outside this repository, install the driver instead of using `cargo xtask`:
`cargo install --path crates/cargo-dbgvis` (unpublished; from a checkout for now), then
`cargo dbgvis setup` compiles it into a self-contained `DBGVIS_HOME` and `cargo dbgvis
config <bin>` prints the `.cargo/config.toml` to point a project at it -- no repository on
the build path. `setup` is not pinned to one exact build: run it under any nightly with
`rustc-dev` and it compiles a driver for that nightly and records it, so each nightly gets
its own matching driver; the wrapper then requires the project to build under that same
nightly. `cargo dbgvis doctor` checks the toolchain is nightly, rustc-dev is present, and
the installed driver matches the active toolchain.

Break at `demo::checkpoint`, then after `run`, `up` use `dbgvis p map` or
`dbgvis p index_borrowed`. A plain `cargo build --example demo` does not enable the
driver, so do not expect those roots to be registered from it. The driver scans only the
executable's own functions by default; for a "logic in a library, `main` is just an entry
point" layout, `DBGVIS_SCAN_DEPS=1` is needed to reach a library's locals, at the cost of
building the workspace libraries with `-Zalways-encode-mir` and registering more types.
Details in the [two-pass auto-registration experiment](docs/auto-registration-experiment.md).
`dbgvis p -m native "hello"` keeps the expression's quotes; `--` ends the print options.
See `dbgvis help` for the full command set.

## Documentation

- [Usage guide](docs/usage-guide.md): the integration API, feature toggles, GDB commands, and safety boundaries.
- [Protocol v2](docs/protocol-v2.md): registration, the mailbox, and bounded text.
- [Phase-two acceptance record](docs/phase-two-acceptance.md): actual tests, known limits, and open items.
- [Q0 validation](docs/archive/q0-validation.md): why nightly (archived; the toolchain decision is settled).
- [Two-pass auto-registration experiment](docs/auto-registration-experiment.md): the driver prototype that removes per-type hand registration, its validation, and known limits (optional; it does not change the default build).

## Verification

```sh
cargo fetch --locked
cargo fmt --all -- --check
cargo clippy --offline --workspace --all-targets --all-features -- -D warnings
cargo test --offline --workspace --all-features
cargo xtask compiler
cargo xtask integration --faults --lto --relocated
cargo xtask autoregister --gdb
```

The verified environment is Linux x86_64, GDB 17.1, rustc 1.99.0-nightly
(12c36e253 2026-08-10). The integration suite needs local ptrace permission; on non-Linux
platforms such as macOS it is skipped outright (Apple/MSVC targets do not emit
`.debug_gdb_scripts`), while the other four steps run as usual. A debug build must keep
`debug = 2` and stay unstripped, or the embedded script and anchor type information both
disappear. Automatic selection renders a value with none of the three capabilities as a
`<unformattable TypeName>` placeholder instead of failing compilation; explicit `via` and
root modes are still checked at compile time.

The bounded buffer does not guarantee a user `fmt` is pure, allocation-free, or
non-blocking. Core dumps and values that cannot be located reliably fall back to native.
The core is implemented, and `cargo-dbgvis` (`setup`/`doctor`/`config`/`uninstall`)
installs and manages the experimental driver outside the repository; a full release
dry-run is not done, and the crates are unpublished.
