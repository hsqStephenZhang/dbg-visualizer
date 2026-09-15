# Usage Guide: nightly / GDB v2

This document describes the core API that is already implemented. Only Linux x86_64 local live processes and a single main-executable registry are supported. Enabling dbgvis requires nightly specialization; the v1 API, pagination, and the LLDB/CodeLLDB frontend are no longer provided. CLI installation and release are not yet implemented; see the [phase-two acceptance record](phase-two-acceptance.md) for unfinished items.

Root entries are collected automatically by linkme, so a consumer project only depends on dbgvis and does not need to depend on linkme directly.

When the same concrete type is registered by multiple crates (for example, a dependency library registers `u64` and the application also auto-discovers `u64`), the runtime verifies that the concrete-type TypeIds match before merging capabilities. A borrowed type uses the type identity obtained after replacing its free lifetimes with `'static`, while the formatter stays lifetime-generic. Candidates with the same name but different identity are rejected even if their size/align match; layout is used only as an additional consistency check. Registration tables from different runtime versions are not merged across tables.

The repository has two complete examples: [examples/explicit.rs](../examples/explicit.rs) uses derive Visualize and explicit registration; build it with `cargo build --example explicit`, its target is `target/debug/examples/explicit`, and its breakpoint is `explicit::checkpoint`. The GDB commands below assume this example. [examples/demo.rs](../examples/demo.rs) uses Debug and the experimental driver for automatic registration; see the [README](../readme.md) for how to build it. An ordinary Cargo build will not automatically register the containers it contains. Neither example uses a feature gate; the conditional-compilation examples below are for consumer projects.

## 1. Own types and third-party types

For now, use a local path dependency (replace with the actual path):

```toml
[features]
default = []
visualize = ["dep:dbgvis"]

[dependencies]
dbgvis = { path = "/absolute/path/dbg-visualizer/crates/dbgvis", optional = true, features = ["derive"] }
```

```rust
#[cfg_attr(feature = "visualize", derive(dbgvis::Visualize))]
struct State {
    entries: std::collections::HashMap<String, Vec<Option<u32>>>,
    address: std::net::SocketAddr,
    #[cfg_attr(feature = "visualize", dbgvis(skip))]
    cache: InternalCache,
}

#[cfg(feature = "visualize")]
dbgvis::register_type!(std::net::SocketAddr; debug, display, default = "display");

fn main() {
    #[cfg(feature = "visualize")]
    dbgvis::enable!();
    // Construct the business object and set a breakpoint at a location where the object is still alive and addressable.
}
```

You can also use `#[cfg_attr(feature = "visualize", dbgvis::main)]` to inject the enabling logic into main, which no longer takes a registry argument. There is no general compatibility guarantee about the ordering relative to other main attribute macros; on conflict, call enable directly in the function body.

A `derive(Visualize)` with no generic parameters generates an auto root registration by default at the same time. A derive that includes type parameters, const parameters, or lifetime parameters only generates the recursive implementation and does not enumerate concrete instances. When you only need the recursive implementation, use the type-level `#[dbgvis(no_register)]` to turn off automatic registration.

For non-generic own types that only reuse Debug/Display, use the attribute:

```rust
#[cfg_attr(feature = "visualize", dbgvis::register)]
#[derive(Debug)]
struct Foo { x: i32, y: String }
```

The standard `#[derive(Debug)]` does not generate dbgvis metadata; the attribute handles registration and does not implement Visualize. If a type both derives Visualize and needs a specified root mode, first add `#[dbgvis(no_register)]`, then add `#[dbgvis::register(auto, visualize, display)]`, so that two root registrations are not generated. In explicit, Point uses only the derive's default auto root and does not additionally register Display.

The derive supports struct, tuple struct, unit, enum, generics, const generics, and borrowed fields, and does not require the type to also implement Debug. The field selection order is fixed as: explicit `via` > Visualize > Debug > Display.

- A third-party field that only has Debug/Display can still be output automatically; it does not require root registration and does not require modifying the third-party library.
- When a third-party container itself has no Visualize, its entire Debug/Display is delegated; it will not automatically switch back to the elements' Visualize.
- Built-in Visualize implementations such as Vec and HashMap apply the automatic priority again to each element.
- When none of the three capabilities are present and the field is not skipped: **no compile error is raised**; the value renders as a `<unformattable typename>` placeholder, and the surrounding fields and container are output as usual. Compile-time checks are reserved for explicit declarations—`via = "debug"/"display"` missing the corresponding trait, an explicit root mode like `register_type!(T; display)` not being satisfied, and explicitly registering a root type that has none of the three—these are still compile errors (and `cargo check` alone will find them).
- After the selected implementation fails, lower-priority implementations are not retried. Debug/Display are not guaranteed to be pure functions, and `dbg!` is not a runtime-callable trait method; this component reuses the fmt implementations and does not call the dbg! macro.

To print a third-party root or a generic concrete instance directly, use the function-like macro or an attributed alias:

```rust
dbgvis::register_type!(bytes::Bytes); // auto by default
dbgvis::register_type!(indexmap::IndexMap<String, Vec<u32>>);
#[dbgvis::register(debug)]
type Borrowed<'a> = indexmap::IndexMap<&'a str, u32>;
```

You register concrete instances of a generic; whole-type-family registration in the form `type All<T> = Vec<T>` is not supported. Lifetimes can be generalized; the static null pointer generated by the macro only carries DWARF type information and does not turn a real borrow into `'static`. A borrowed type that implements Display only for `'static` cannot be declared as an explicit Display root for an arbitrary lifetime.

In `register_type!(T; debug, display, default = "display")`, the part after the semicolon is an optional mode list; `#[dbgvis::register(...)]` accepts the same options and can be applied to a struct, enum, or alias. The attribute macro and the function-like macro share the macro namespace, so the function-like entry point is called `register_type!`, not `register!`.

## 2. a/b/c cross-crate composition and switches

a/b/c only need their own respective derives; private fields are accessed by the generated code of their own crate. The root entries for non-generic types and AppState are collected by the linker, so main only needs to enable; generically nested types do not need to be registered independently, and enable is not called in libraries. Recursion still goes through static trait calls and does not access the linkme table field by field.

a/b/c are illustrative names for cross-library integration. The repository does not keep demo-a/b/c fixture crates; the tests create these libraries in the system temporary directory, verify them after building, and clean them up automatically, so the repository examples do not depend on them.

Only entries that are actually linked into the final program and use the same runtime are collected. A dependency that is not referenced at all in Cargo.toml is not guaranteed to be linked; if you intentionally bring in a dependency that only has registration side effects, you can use `use that_crate as _;` to explicitly establish a link reference, and check `dbgvis types` in the target artifact. Incompatible multi-version runtime tables are not merged automatically, and dynamic library loading/unloading is not supported.

Each library turns off `visualize` by default and, when enabled, brings in the same version of dbgvis. The application's feature is passed through explicitly:

```toml
[features]
default = []
visualize = ["dep:dbgvis", "a/visualize", "b/visualize", "c/visualize"]
```

The project switch simultaneously wraps the derive, the field `dbgvis` attributes, the register attribute/register_type! calls, and enable. When off, macros are not expanded, the runtime/linkme/dispatcher are not linked, anchors/registration entries are not generated, the output area is not reserved, and the registry is not initialized; the business field layout does not depend on this switch.

```sh
cargo build --features visualize
cargo build --release --no-default-features
```

For the consumer project above, release is not an off switch; `--release --features visualize` still enables the Rust-side registry. However, the embedded GDB script and the anchor type information both depend on debug info: the final crate's profile must retain `debug = 2` and not `strip`. At `debug = 0`, rustc does not generate `.debug_gdb_scripts` at all (and since Rust 1.77, release also defaults to `strip = "debuginfo"`); at `debug = 1`, there is only a line-number table and the anchor type cannot be resolved. In these two cases, `DBG_VIS_MODULE_V2` is still in the binary, but GDB has no `dbgvis` command or reports everything as unregistered, which is a silent failure; the compile test fixes this behavior with `CARGO_PROFILE_RELEASE_DEBUG=0`. The compile test verifies stable-off, nightly-on, and a/b/c pass-through in a separate temporary consumer project; the repository demo is always enabled, and a root-package visualize feature is no longer provided. Cargo features accumulate, so when another dependency path turns dbgvis on, `--no-default-features` cannot force it off on its behalf.

enable can be declared only once per executable, using the no-argument `dbgvis::enable!()`. It walks the `fn() -> Root` factories collected by linkme, sorts them by canonical type name, checks for duplicates, and publishes the v2 table. An empty table is allowed, and the collection cap is 4096 entries before merging. Same-type registrations across modules/crates merge capabilities after verifying the TypeId; after sorting by type name and anchor name, the first default mode and the first entry for each capability are kept. Candidates with the same name but different identity are rejected; a marker cannot be used for a different type. Duplicate declarations in the same scope may fail to compile earlier due to symbol redefinition.

The linkme collection itself has no startup constructor, but enable still performs one factory walk and allocation; these factories only construct metadata and do not call business fmt. Automatic registration increases the code and debug-info retention, and may also expose non-generic types' field capability errors earlier; `no_register` only turns off that type's root registration, not its recursive implementation or other types' use of it.

## 3. Field control and built-in types

| Attribute | Position and semantics |
| --- | --- |
| `skip` | Field; not read, not output, no formatting constraint added |
| `rename = "..."` | Type, enum variant, or named field; changes the display label |
| `via = "debug"` / `"display"` | Field; strictly requires the corresponding trait, and no longer auto-selects another capability |
| `transparent` | A struct with a single effective field; outputs only that field |
| `bound = "T: SomeTrait"` | Type; adds a where condition |
| `no_register` | Type; only derives Visualize, does not generate an automatic root registration |

Third-party remote derive, getters, custom adapters, and a Serde bridge are not provided. A hand-written implementation can use `Visualize::visualize(&self, &mut Formatter) -> dbgvis::Result` and write via the record/tuple/sequence/map builder; write errors should be propagated promptly.

The built-in recursive implementations include slice, array, Vec, VecDeque, HashMap, BTreeMap, HashSet, BTreeSet, Option, Result, 1–6 tuples, references, Box/Rc/Arc, and PhantomData. HashMap does not require the hasher to implement Debug/Clone; PhantomData does not require the marker parameter to implement any formatting trait. Other types are delegated according to their existing Debug/Display; deep Visualize semantics are not guaranteed for arbitrary containers.

## 4. GDB loading and commands

The runtime ships its own build script and GDB Python, and the dependency-side `debugger_visualizer` embeds them into the final artifact. A consumer project does not need to copy the script or build.rs. This section is only generated for ELF targets such as Linux: the rustc target spec's `emit-debug-gdb-scripts` is false for Apple and MSVC targets, so it is never embedded, and only non-GDB tests can run on macOS. The script executes in a separate `dbgvis_gdb` Python module; when the same binary is reloaded, the old session is released and its configuration, counters, and poisoned state are inherited, and a load failure does not replace the already-loaded module. It is enough to trust the selected binary:

```sh
rust-gdb -iex "add-auto-load-safe-path /absolute/path/to/binary" /absolute/path/to/binary
```

Do not set a global `safe-path *`. Using rust-gdb also keeps the toolchain's standard-library formatters. Auto-loading Python itself requires trust, even when the current execution is manual.

The example breaks at checkpoint, so first `up` to the main frame:

```text
dbgvis types
dbgvis print app
dbgvis print bytes
dbgvis print --mode display address
dbgvis print --mode native map
dbgvis print --buffer 32 --alternate app
dbgvis status
```

Common commands support short aliases: `p`/`print`, `c`/`config`, `t`/`types`, `s`/`status`, `rf`/`refresh`, `rs`/`reset`. The options of `print` also support `-m` (`--mode`), `-b` (`--buffer`), and `-a` (`--alternate`), for example `dbgvis p -m auto -b 128 -a app`; the mode names still require the full values such as `debug`, `display`.

When the effective configuration is native, an explicit print uses that entry's default mode instead; `--mode native` always uses the native display and allows unregistered types. `--mode auto` means the registered default mode: an attribute-free registration automatically selects Visualize > Debug > Display, while a registration with explicit capabilities selects only from the declared capabilities. Requesting an unregistered Debug/Display/Visualize is rejected before the target call.

Print options go before the expression, and `--` can be used to end the options. The quoting, escaping, and internal whitespace of the remaining expression are passed to GDB as-is, for example `dbgvis p -m native "hello"` and `dbgvis p -m native -- -1`. Option values can still use shell-style quoting; a configured type name with spaces still requires quoting. Use `dbgvis p --help` to see the full modes and short arguments.

Turn on automatic summaries:

```text
dbgvis config summary.mode auto
dbgvis config execution automatic
dbgvis config --type explicit::Point summary.alternate true
```

Native children are kept, but no new logical child-value tree is generated; there is no `page` command. Global configuration, type configuration, and explicit command options override in that order. Type names are taken from `dbgvis types`, and quoting is used when they contain spaces.

| Configuration | Default | Range |
| --- | --- | --- |
| `summary.mode` | native | native / auto / visualize / debug / display |
| `execution` | manual | manual / automatic |
| `summary.buffer_bytes` | 4096 | 1 to compiled capacity |
| `summary.alternate` | false | true / false |
| `format.max_depth` | 32 | 1–128 |
| `format.max_nodes` | 4096 | 1–1000000 |
| `execution.timeout_ms` | 200 | GDB timeout rounded up to a whole second, minimum 1 second |

Use `dbgvis reset` to restore the configuration and `dbgvis refresh` to refresh metadata. Neither clears the poisoned state of a failed process. `status` shows the call count, cumulative duration, and most recent error; a single root text output calls the dispatcher only once, without pagination and without automatically growing the buffer to retry.

## 5. Budget, lifetime, and failure boundary

The compiled default output area is 65536 bytes and can be adjusted to 1–16 MiB with `DBGVIS_BUFFER_BYTES=131072 cargo build --example explicit`. Each request writes UTF-8 into that area, transmits by length, and preserves NUL. Over the limit, it returns a valid prefix plus `<dbgvis: byte/depth/node limit>` or a cycle diagnostic, with no guarantee that brackets are closed. This text is not a serialization archive.

The framework's recursive path shares the depth/node/byte budget and checks before iterating to the next item; cycle detection is only for the current visit path, and ZSTs are not deduplicated by address alone. It does not build a full DOM or String and then truncate. User fmt / hand-written Visualize can still compute, allocate, ignore errors, or loop infinitely; the buffer does not restrict these behaviors, and the Debug/Display whole-value boundary does not provide an internal node budget either.

By default, manual/native does not automatically execute target code; the derive does not insert monitoring into ordinary business accesses. The one-time enable initialization has a registry allocation, and the static output area has a memory footprint; they are controlled only by the compile feature.

It must be called after enable, when the object is truly alive, sharable by borrow, and addressable. When the object is being modified, holds a mutable borrow, is uninitialized, or does not satisfy Rust invariants, pausing does not automatically make the call safe. You cannot copy the bytes of a value that has been optimized away and pass them off as the real object. Debugger type-metadata checks are not a memory-safety proof.

GDB first compares the typed anchor type belonging to the main executable; when necessary, it verifies the full DWARF structure (including field offsets/types, the enum's active variant, and alignment). It rejects when information is insufficient or there are multiple candidates, and does not guess by size. It cannot promise to handle all same-name different-crate-version cases, all optimization states, or lifetime-sensitive specialization. Currently only the nightly/GDB versions listed in the documentation are verified, and upgrades require re-acceptance.

automatic requires GDB's async-call timeout to be available; without that capability, native is retained. An explicit call may require the user to interrupt manually when the backend has no timeout capability. It does not proactively acquire the application's Mutex/RwLock; a user fmt that blocks on its own is still a risk.

An unwind panic is caught on the inner side of the C ABI, and the panic hook still runs; fmt::Error returns an error and does not retry another trait. On timeout, abort, target exit, or an unconfirmable return, the process is marked poisoned and can only be recovered after a restart; you cannot clear the Rust busy flag and force a continuation. Timeout/recovery of registers does not undo side effects that have already occurred. A core dump cannot call into Rust and can only fall back to native.

Text is not cached across requests; after a value is modified within the same stop, the next explicit request reads the new result. The varobj/display cache held by the UI itself still needs the frontend to re-request.
