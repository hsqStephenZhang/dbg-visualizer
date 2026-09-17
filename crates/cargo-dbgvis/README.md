# cargo-dbgvis

A `cargo` subcommand that installs and manages the experimental
[`dbgvis`](https://crates.io/crates/dbgvis) auto-registration driver and debugger bridges,
so a project can use dbgvis with no hand-written `register_type!`.

```sh
cargo install cargo-dbgvis                       # the CLI builds on stable
cargo dbgvis setup                               # under your nightly: compile the driver, record its toolchain
cargo dbgvis config my-bin >> .cargo/config.toml # wire the project to the driver
cargo +nightly build                             # build through the driver
```

| Command | What it does |
| --- | --- |
| `setup` | compile the driver into `DBGVIS_HOME`, record its toolchain |
| `doctor` | check nightly, `rustc-dev`, and that the driver matches the toolchain |
| `config <bin>` | print the `.cargo/config.toml` snippet for a project |
| `script [--gdb\|--lldb\|--all] [DIR]` | write the debugger bridge(s) to a dir (default `target/dbgvis`) |
| `home` / `uninstall` | print / remove `DBGVIS_HOME` |

## Requirements

`setup` builds a driver that links `rustc_private`, so it needs a **nightly** toolchain with
the **`rustc-dev`** component (`rustup toolchain install nightly --component rustc-dev`), on
**Linux x86_64**. The generated LLDB bridge (`cargo dbgvis script --lldb`) needs no rebuild
of the target; the GDB script is embedded into the artifact by the `dbgvis` runtime.

See the [repository](https://github.com/hsqStephenZhang/dbg-visualizer) for details and the
auto-registration experiment write-up.

## License

Licensed under either of MIT or Apache-2.0 at your option.
