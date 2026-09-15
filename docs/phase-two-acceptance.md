# Phase Two Core Acceptance Record

Date: 2026-09-10; updated 2026-09-11 with the linkme auto-collection acceptance; updated 2026-09-12 with the type-identity, example-entrypoint, and expression-parsing acceptance. Direction: **nightly specialization, GDB first, remove v1**. What is recorded here is the actual local result; it does not mean every planned item is done, nor that remote CI or a crates.io release has been completed.

The explicit-registration demo entrypoint is examples/explicit.rs: your own types derive Visualize directly, with no feature gate; run `cargo build --example explicit`. examples/demo.rs is kept for the Debug + driver auto-registration use case, and an ordinary Cargo build cannot substitute for an auto-registration build. The a/b/c fixtures have been removed from the repository; the workspace now holds only the three core crates plus xtask. xtask/src/compiler.rs generates a/b/c and a consumer project in a temporary directory, keeping the enable/disable and debug/LTO checks, and cleans up the sources automatically when finished.

## 1. Core Delivered

- The `dbgvis` facade, `dbgvis-macros`, and the nightly `dbgvis-runtime`.
- derive, the field skip/rename/via/transparent/bound attributes, concrete-root registration, main/enable.
- Automatic root registration for non-generic derive, which no_register can turn off; the register attribute, register_type!; linkme cross-crate factory collection, parameterless enable/main; the centralized visualizers entrypoint has been removed.
- Generic automatic selection of Visualize > Debug > Display; a failure does not retry the other capabilities.
- Generic containers, borrows, const parameters, and the ZST strategy; standalone a/b/c recursive composition.
- The marker TypeId, a separate concrete-type identity TypeId, the typed anchor, type-pinned function pointers, and a single v2 dispatcher; same-name-but-different-identity is refused rather than merged.
- Byte/depth/node budgets, UTF-8/NUL, and current-path cycle detection.
- GDB native/manual/automatic modes, metadata validation, single text call, and fault isolation.
- The runtime embeds the script; the consumer needs no build.rs or Python copy.
- The default-off feature of the standalone consumer project, turned off together with the conditional derive on a/b/c; the demo itself is always enabled.

The old adapters, the v1 runtime entrypoint, pagination, and the LLDB/CodeLLDB-specific scripts and tests have been removed. Historical versions are recoverable in Git; the phase-one acceptance record cannot be used as v2 test evidence.

## 2. Environment and Reproduction

Local Linux x86_64, GDB 17.1, nightly rustc `1.99.0-nightly (12c36e253 2026-08-10)`. Turning the feature off uses stable rustc 1.97.1. There is as yet no general guarantee for rolling nightly updates or for other GDB versions/platforms.

```sh
cargo fmt --all -- --check
cargo clippy --offline --workspace --all-targets --all-features -- -D warnings
cargo test --offline --workspace --all-features
cargo xtask compiler
cargo xtask integration --faults --lto --relocated
cargo xtask autoregister --gdb
```

Running offline requires `cargo fetch --locked` beforehand. Real GDB tests need ptrace; the runner sets an external timeout for each debugger subprocess and reaps only the process groups it created itself.

## 3. Automation Coverage

| Layer | Actual checks |
| --- | --- |
| Rust formatting tests | Visualize/Debug/Display single-capability and priority; HashMap/Vec/BTreeMap/HashSet/VecDeque; field strategies, enum, transparent |
| Recursion budget | UTF-8, NUL, byte/node/depth stopping, non-global dedup of shared values, Rc cycles, ZST, ignoring writer errors |
| Protocol unit tests | 128/96/80/40-byte ABI, illegal arguments, uninitialized, wrong mailbox, busy, panic/error recovery, zeroing out the old output length, duplicate registration |
| Standalone consumer project | path with spaces, the `dv` dependency alias, no direct dependency on linkme needed; multi-module entries collected, cfg root entries, borrows and PhantomData; auto-registration, no_register, empty table; duplicate registration of the same type merges, same-name-same-layout-but-different-identity is refused |
| Dependency version conflict | temporarily create identity-library 1.0/2.0; a same-name-same-layout Shared is distinguished by concrete-type TypeId in debug/LTO and refused for merging |
| Placeholder rendering (debug + release) | a plain field, a Vec element, and the value of a nested HashMap, all with no capability, output `<unformattable type-name>` and build successfully |
| Compile failures (debug + release) | explicit missing Debug/Display; explicit root capability missing; generic type family; illegal attributes/default; cannot register a static-only Display as an arbitrary borrow |
| Real GDB debug/LTO | native zero calls, explicit root exactly once each time, automatic mode, type-level coverage, zero calls when the mode is unsupported/function calls are off, modification at the same stop and a new value after continuing execution |
| GDB command parsing | string/char literals, quotes, escapes, internal whitespace, and negative-number expressions match the native result; short options, equals-values, `--`, illegal arguments zero calls |
| driver auto-registration | debug/LTO builds and real GDB for auto_poc/demo; HashMap, IndexMap, borrowed strings, const 17/19 borrowed generics, Debug-only root output |
| Third-party and generic matrix | Bytes; three IndexMap sets (different K/V/S, borrows, arrays, ZST); hashbrown::HashMap; SocketAddr Display; const 17/19 same-layout instances |
| Across a/b/c | in the temporary project a/b have no Debug and c::Label is Display-only, yet debug/LTO actually recurse and output successfully; check that the auto-registration count is 4 (App, Item, UnusedRegistered, State), and that the generic Record and Label are not registered |
| Link retention | the final symbols of the temporary project's debug/LTO build include b::UnusedRegistered, which has no business reference, along with the Item/State anchors; the real GDB demo independently verifies the Local inside main and the own Counter; a/b/c is no longer claimed as the current GDB scenario |
| Relocated binary | after copying to a path with spaces it still loads the embedded script and completes the same GDB assertions, with no need to copy Python |
| Faulting subprocess | unwind panic, fmt::Error, timeout, process exit, and panic=abort; the first two can be called again after fixing the value, the rest are poisoned, and refresh/reset does not clear it |
| Feature off | the standalone consumer project builds on stable; the dependency graph has no dbgvis/runtime/macros/linkme; the final symbols have no anchor, linkme, registration entries, dispatcher, or dedicated runtime/buffer; once on, a/b/c actually recurse |

Source entrypoints: `crates/dbgvis/tests/formatting.rs`, `crates/dbgvis-runtime/src/protocol_tests.rs`, `xtask/src/compiler.rs`, `tests/inferior/`. The full test count is per the output; old test-file names are not used as evidence.

## 4. Actual Comparison with the Current rust-gdb

The current toolchain ships with a pretty-printer, so **the standard HashMap is not entirely unusable**; the test read:

```text
HashMap(size=1) = {["points"] = Vec(size=2) = {core::option::Option<demo::Point>::Some(demo::Point {x: 1, y: 2}), core::option::Option<demo::Point>::None}}
```

dbgvis gets, for the same object:

```text
{"points": [Some(Point { x: 1, y: 2 }), None]}
```

So "every rust-gdb HashMap only shows the internal layout" cannot be taken as a measured conclusion. The benefit is that the type author defines the recursive summary uniformly, field strategies are preserved across crates, and it does not depend on each debugger version's dedicated display capability for every container.

The native representation of the current self-contained AppState is around 1500 characters (including the internal object structure and addresses, varying with build and type name); dbgvis gives:

```text
AppState { points: {"app": [Some(Point { x: 5, y: 6 })]}, bytes: b"hello world", ordered: {1: "one", 2: "two", 3: "three"}, address: 127.0.0.1:8080, state: State::Pending }
```

The secret field is not read/displayed. Cross-crate recursion without Debug is verified separately by the temporary consumer project. hashbrown outputs `{9: [8, 7]}` with just the dependency added and a register_type! declaration, without modifying the type logic of the runtime/Python.

## 5. Explicit Boundaries and Follow-up Work

1. Automatic selection no longer refuses a value that lacks a capability at compile time; instead it renders an `<unformattable type-name>` placeholder. Explicit declarations are still checked at compile time. Any lifetime-sensitive specialization is out of scope for what is promised.
2. LTO can optimize away an unused enum field or an entire local value. The first version of the test was therefore refused by type validation; rather than relaxing validation to guess, the example was changed to pass a live AppState reference into checkpoint so the object under test stays fully alive. Arbitrary optimized-away business values cannot be recovered.
3. GDB keeps only the active enum variant for some Rust dynamic types, or omits trailing padding; the implementation adds structured DWARF cross-checking. All same-name-different-crate-version cases and all forms of debug info are still unaccepted, and type metadata is not a proof of memory safety.
4. The compile feature-off eliminates the runtime from the standalone consumer project's dependency graph, but enable, once on, does one registry allocation. The demo is always enabled; the clean build/derive cost, enable time, and the performance of 100 summaries or 128 independent variables have not yet been measured systematically.
5. Installing the CLI setup/doctor/uninstall, a full cargo package clean-room release rehearsal, and merge/uninstall tests against an existing configuration are still to be implemented. The temporary path consumer project and the relocated binary only prove part of the integration/embedding, and are not equivalent to being released.
6. core dump, all optimization-missing values, repeated loading/cross-version upgrade, all panic-payload behaviors, and the full platform matrix still need expanded real-backend acceptance; the core already refuses targets with no execution capability. LLDB/CodeLLDB and v1 are not in this round of delivery.
7. CI has been adjusted to nightly/GDB, which cannot be used to claim that the remote workflow has run or that all CI environments support the same GDB timeout interface.

Conclusion: the scope of "implementing the core and preliminarily validating HashMap, third-party fmt, and generic tolerance" has been reached; the full install/distribution, performance, and broad-compatibility acceptance in Q4/Q5 is not yet done.
