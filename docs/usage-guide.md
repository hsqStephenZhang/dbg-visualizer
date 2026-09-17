# Usage Guide

`dbgvis` renders a value's logical view — `Visualize`, else `Debug`, else `Display` — in
the debugger, by calling the formatter in-process. This guide covers setup, the
registration API, the debugger commands, and the safety boundary.

## Requirements

- **A nightly toolchain with the `rustc-dev` component** — the runtime uses
  `#![feature(specialization)]` and the driver links `rustc_private`
  (`rustup toolchain install nightly --component rustc-dev`). The `cargo-dbgvis` CLI itself
  builds on stable.
- **Linux x86_64.** GDB is the primary frontend (script auto-embedded); the LLDB bridge is
  experimental and loaded by hand. Apple/MSVC do not emit `.debug_gdb_scripts`.
- **`debug = 2`, unstripped.** At `debug = 0` rustc emits no `.debug_gdb_scripts` (and
  release defaults to `strip = "debuginfo"` since 1.77); at `debug = 1` the anchor type
  cannot be resolved. In both cases `DBG_VIS_MODULE_V2` is still linked but the debugger
  reports everything as unregistered — a silent failure.

Scope is deliberately narrow: local live processes, one main-executable registry. Core
dumps fall back to native; dynamic-library load/unload and multi-version tables are not
merged.

## Recommended setup: the `cargo dbgvis` driver

The driver auto-registers the concrete types a debug session wants, so a project needs no
hand-written `register_type!`:

```sh
cargo install cargo-dbgvis                   # the CLI itself builds on stable
cargo dbgvis setup                           # under your nightly: build the driver, record its toolchain
cargo dbgvis config my-bin >> .cargo/config.toml
cargo +nightly build                         # build through the driver, same nightly as setup
```

`cargo dbgvis config <bin>` prints a `.cargo/config.toml` wiring the build to the driver:

```toml
[build]
rustc-workspace-wrapper = "<DBGVIS_HOME>/wrapper.py"
[env]
DBGVIS_HOME = "<DBGVIS_HOME>"
DBGVIS_AUTO_CRATE = "my-bin"
```

| Command | What it does |
| --- | --- |
| `setup` | compile the driver into `DBGVIS_HOME`, record its toolchain |
| `doctor` | check nightly, `rustc-dev`, and that the driver matches the active toolchain |
| `config <bin>` | print the `.cargo/config.toml` snippet for a project |
| `script [--gdb\|--lldb\|--all] [DIR]` | write the debugger bridge(s) to a dir (default `target/dbgvis`) |
| `home` / `uninstall` | print / remove `DBGVIS_HOME` |

`setup` is not pinned to one build — run it under any nightly with `rustc-dev`, it compiles
a matching driver and records the toolchain, and the wrapper then requires the project to
build under that same nightly. Scope of the scan: by default only the executable's own
functions; `DBGVIS_SCAN_DEPS=1` also reaches workspace-library locals (building those with
`-Zalways-encode-mir`), and `DBGVIS_SCAN_CRATES=regex.*` selects dependency crates by
regex. A plain `cargo build` without the wrapper does not enable the driver. See the
[auto-registration experiment](auto-registration-experiment.md) for its limits.

## Debugger commands

### GDB

The script is embedded in the artifact, so it is enough to trust the binary:

```sh
rust-gdb -iex "add-auto-load-safe-path /absolute/path/to/binary" /absolute/path/to/binary
```

Do not use a global `safe-path *`. `rust-gdb` keeps the standard-library formatters.

```text
dbgvis types                       # list registered types (with capabilities)
dbgvis print app                   # render one value
dbgvis print --all                 # every registered local in the frame (alias: dbgvis all)
dbgvis print -m display address    # -m/--mode, -b/--buffer, -a/--alternate
dbgvis print -m native -- -1       # native display; `--` ends the options
dbgvis status                      # call count, cumulative time, last error
```

Aliases: `p`/`print`, `c`/`config`, `t`/`types`, `s`/`status`, `rf`/`refresh`,
`rs`/`reset`. `--mode native` uses the debugger's own display and allows unregistered
types; `--mode auto` uses the registered default (`Visualize > Debug > Display` for an
attribute-free root, or only the declared capabilities). Requesting a capability a type did
not register is rejected before the target call. Print options precede the expression; the
expression's quoting is passed to the debugger unchanged. `dbgvis p --all` renders each
in-scope registered local as `name = value` and lists the rest under `skipped`.

Automatic summaries (a registered value is rendered on every stop) are opt-in:

```text
dbgvis config summary.mode auto
dbgvis config execution automatic
dbgvis config --type explicit::Point summary.alternate true
```

`automatic` requires GDB's async-call timeout; without it, native is kept. Global config,
per-type config, and per-command options override in that order. `dbgvis reset` restores
config, `dbgvis refresh` re-reads metadata; neither clears a poisoned process.

| Config | Default | Range |
| --- | --- | --- |
| `summary.mode` | native | native / auto / visualize / debug / display |
| `execution` | manual | manual / automatic |
| `summary.buffer_bytes` | 4096 | 1 … compiled capacity |
| `summary.alternate` | false | true / false |
| `format.max_depth` | 32 | 1–128 |
| `format.max_nodes` | 4096 | 1–1000000 |
| `execution.timeout_ms` | 200 | rounded up to whole GDB seconds, min 1 |

### LLDB (experimental)

LLDB has no `.debug_gdb_scripts` auto-load, so the bridge is generated, not embedded, and
loaded by hand. It drives the **same mailbox** as GDB, so the runtime is unchanged.

```sh
cargo dbgvis script --lldb          # writes target/dbgvis/lldb.py
```

```text
(lldb) command script import target/dbgvis/lldb.py
(lldb) dbgvis print app
(lldb) dbgvis all                   # --all; paged through $PAGER (less), --no-pager to disable
(lldb) dbgvis print -m debug address
(lldb) dbgvis types
```

`-m`/`-b`/`-a` match GDB. Under VSCode CodeLLDB, put the import in `initCommands` and run
the commands in the Debug Console with a leading backtick. The one real difference from GDB
is matching: LLDB exposes no type for an anchor symbol, so instead of DWARF-shape
comparison the bridge reconciles LLDB's type names with the registry's Rust spellings
(mapping C integer names and arrays, eliding defaulted generics). LLDB's names drop
lifetimes and const generics, so two types differing only in those are indistinguishable —
the bridge reports them ambiguous and refuses rather than match the wrong slot.

## Registration API (by hand)

Registration decides which types the registry holds; the driver above does it
automatically. To do it yourself, add the dependency (feature-gated is recommended):

```toml
[features]
visualize = ["dep:dbgvis"]
[dependencies]
dbgvis = { version = "0.2", optional = true, features = ["derive"] }
```

```rust
#[cfg_attr(feature = "visualize", derive(dbgvis::Visualize))]
struct State {
    entries: std::collections::HashMap<String, Vec<Option<u32>>>,
    #[cfg_attr(feature = "visualize", dbgvis(skip))]
    cache: InternalCache,
}

#[cfg(feature = "visualize")]
dbgvis::register_type!(std::net::SocketAddr; debug, display, default = "display");

fn main() {
    #[cfg(feature = "visualize")]
    dbgvis::enable!();
    // stop at a breakpoint where the value is alive and addressable
}
```

- **`derive(Visualize)`** — recursive rendering with the fixed priority `via > Visualize >
  Debug > Display`; a field only needs `Debug`/`Display`, and does not require the type to
  implement `Debug`. A **non-generic** derive also registers a root; a generic/const/
  lifetime-parameterized derive emits only the recursive impl (`#[dbgvis(no_register)]`
  suppresses the root when it would be generated).
- **`#[dbgvis::register]`** on a `#[derive(Debug)]` type — register a root that reuses
  `Debug`, without implementing `Visualize`.
- **`register_type!(T; modes…)`** — register a third-party root or a *concrete instance* of
  a generic (`register_type!(indexmap::IndexMap<String, Vec<u32>>)`); whole families
  (`Vec<T>`) are not supported. Lifetimes can be generalized.
- **`enable!()`** — call exactly once per executable. It walks the `linkme`-collected
  factories, sorts and de-duplicates by canonical type name, merges same-`TypeId`
  registrations' capabilities, rejects same-name/different-identity candidates, and
  publishes the v2 table. Libraries never call it.

When a field has none of the three capabilities and is not skipped, it renders as
`<unformattable TypeName>` at runtime — **not** a compile error. Compile-time errors are
reserved for explicit declarations: `via = "debug"` without `Debug`, an explicit root mode
that is not satisfied, or registering a root type with no capability at all.

Only entries actually linked in and using the same runtime are collected (add
`use that_crate as _;` to keep a side-effect-only dependency, and confirm with `dbgvis
types`). Each library keeps `visualize` off by default; the application enables the chain
(`visualize = ["dep:dbgvis", "a/visualize", …]`).

### Field attributes and built-ins

| Attribute | Position and effect |
| --- | --- |
| `skip` | field — not read, not output |
| `rename = "…"` | type / variant / field — display label |
| `via = "debug"` \| `"display"` | field — force that trait, no auto-selection |
| `transparent` | single-field struct — output only that field |
| `bound = "T: Trait"` | type — extra `where` clause |
| `no_register` | type — derive the impl but generate no root |

Built-in recursive impls: slice, array, `Vec`, `VecDeque`, `HashMap`, `BTreeMap`,
`HashSet`, `BTreeSet`, `Option`, `Result`, 1–6 tuples, references, `Box`/`Rc`/`Arc`, and
`PhantomData`. `HashMap` does not require the hasher to implement anything; each element
re-applies the automatic priority. A container without its own `Visualize` delegates wholly
to its `Debug`/`Display` and does not switch back to per-element `Visualize`. Remote derive,
getters, and a Serde bridge are not provided; hand-written impls use
`Visualize::visualize(&self, &mut Formatter) -> dbgvis::Result` with the record/tuple/
sequence/map builders.

## Safety and limits

The formatter runs **in the inferior**, so the value must be genuinely alive, addressable,
and consistent — a stopped process, a mutable-borrowed or half-initialized value, or bytes
copied off an optimized-away value do not make the call safe, and a debugger type check is
not a memory-safety proof.

The output area (default 65536 bytes, `DBGVIS_BUFFER_BYTES` to 1–16 MiB) receives UTF-8 by
length; over budget it returns a valid prefix plus `<dbgvis: byte/depth/node limit>` with
no guarantee brackets close. The depth/node/byte budget is shared and checked before each
step, but a user `fmt` may still allocate, block, or loop — the buffer does not constrain
that, and dbgvis does not acquire the program's locks for you. A panic is caught at the C
ABI (the panic hook still runs); on timeout, abort, exit, or an unconfirmable return the
process is marked poisoned and recovers only after a restart. Text is not cached across
requests, so a value changed within the same stop reads fresh on the next explicit request.

The mailbox ABI (the exported registry, request/response/output buffers, and dispatcher)
is documented in [protocol v2](protocol-v2.md); the runtime and both debugger bridges are
just clients of it.
