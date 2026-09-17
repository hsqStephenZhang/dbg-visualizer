# dbgvis

Render a value's *logical* view — a type's `Visualize`, else `Debug`, else `Display` — live
in the debugger at a breakpoint, instead of reaching for `dbg!` or `tracing`. The formatter
runs in-process and the debugger asks for it; `HashMap`/`Vec` and the like go through their
public iteration APIs, recursively.

```rust
#[derive(dbgvis::Visualize)]
struct AppState { scores: std::collections::HashMap<String, Vec<u32>>, label: String }

fn main() {
    dbgvis::enable!();               // once per executable; roots are collected by linkme
    // ... stop at a breakpoint where the value is alive and addressable
}
```

```text
(gdb) dbgvis print app        # or: dbgvis print --all
```

## Requirements

- A **nightly** toolchain (the runtime uses `#![feature(specialization)]`).
- **Linux x86_64**; GDB is the primary frontend (script auto-embedded), with an
  experimental LLDB bridge. The final binary must keep `debug = 2` and stay unstripped.

## Registration

- `#[derive(dbgvis::Visualize)]` — recursive rendering; a non-generic derive also registers
  a root.
- `#[dbgvis::register]` on a `#[derive(Debug)]` type — register a root that reuses `Debug`.
- `dbgvis::register_type!(T)` — a third-party root or a concrete instance of a generic.

To avoid hand-written registration entirely, use the [`cargo-dbgvis`](https://crates.io/crates/cargo-dbgvis)
driver. See the [repository](https://github.com/hsqStephenZhang/dbg-visualizer) for the full
usage guide, debugger commands, and safety boundary.

## License

Licensed under either of MIT or Apache-2.0 at your option.
