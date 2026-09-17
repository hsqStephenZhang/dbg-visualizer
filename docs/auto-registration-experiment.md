# Two-Pass Auto-Registration: A Limited Feasibility Validation

This experiment validates "scan the concrete types → generate Rust registration source → compile it into the original crate on a second pass"; it is not about modifying rustc's monomorphization collector, nor about guessing function addresses with regexes. The core runtime, proc-macro, and GDB protocol stay unchanged.

## Conclusion and Scope

The main path already works on this machine: a HashMap, an IndexMap, and an own type that implements only Debug, none of them with a manual `register_type!`, can be output via `dbgvis print`; both the debug and the release/LTO real GDB calls have been verified. The full reproducible compile and GDB verification commands are below.

This is still an **experimental backend** and cannot yet replace all manual registration. The first pass filters by root trait, the second pass compiles into the original crate; a container whose elements are unformattable no longer fails the second pass, but instead renders an `<unformattable …>` placeholder at runtime. An existing same-type root registration in a dependency library is merged by the lifetime-erased concrete-type TypeId; a same-name-but-different-identity registration is refused at enable time. It must not be treated as a product promise of "whole-program automatic discovery that never breaks the original build."

## Usage

Requires Linux x86_64, Python 3, GDB, and exactly the following compiler with rustc-dev installed:

```text
rustc 1.99.0-nightly (12c36e253 2026-08-10)
```

### Install and Use (no need to clone this repository)

`cargo-dbgvis` embeds the wrapper and driver source into the binary; at `setup` time it writes them to a self-contained `DBGVIS_HOME` (default `$XDG_CACHE_HOME/dbgvis`) and compiles the driver there:

```sh
cargo install cargo-dbgvis        # the CLI itself builds on stable
cargo dbgvis setup                # must run under a nightly toolchain with rustc-dev
cargo dbgvis doctor               # checks that the toolchain, rustc-dev, and driver are ready
cargo dbgvis config my_bin        # prints a paste-ready .cargo/config.toml snippet
```

Write this into the consumer project's `.cargo/config.toml` (`cargo dbgvis config` generates it):

```toml
[build]
rustc-workspace-wrapper = "/home/you/.cache/dbgvis/wrapper.py"
[env]
DBGVIS_HOME = "/home/you/.cache/dbgvis"
DBGVIS_AUTO_CRATE = "my_bin"
```

After that an ordinary `cargo build` enables auto-registration, and the generated plans land in `DBGVIS_HOME/plans` without touching the repository. Once `DBGVIS_HOME` is set, the wrapper locates the driver, sources, and plans entirely relative to it — this is the key to using it away from the repository. Core visualization itself does not need the driver: add the `dbgvis` dependency, `enable!()`, and hand-write `register_type!`; the driver only saves you those hand-written lines.

### Developing Inside the Repository

Inside this repository (with `DBGVIS_HOME` unset) the wrapper falls back to the checkout layout, and `cargo xtask driver` compiles the driver to `target/auto-register/`. `cargo xtask driver`, `cargo dbgvis setup`, and the wrapper all check the rustc version; this is not a compatibility promise for a floating nightly. Building the driver calls rustc directly, and dynamic-library lookup uses the rpath of the build artifact.

`tools/auto-register/Cargo.toml` is a standalone nested workspace and declares `[package.metadata.rust-analyzer] rustc_private = true`. VS Code's `.vscode/settings.json` treats the main workspace and this tool as separate linked projects; so RA can index the rustc-dev crates such as `rustc_ast` and `rustc_middle` without pulling the driver into the main workspace. `RUSTC_BOOTSTRAP=1` is provided only to the RA/Cargo check sessions; the driver's official build is still locked to the toolchain by `cargo xtask driver`. From the Command Palette you can run `dbgvis: build auto-register driver`, `dbgvis: check auto-register driver`, and `dbgvis: run auto-register contracts`.

From the repository root, run:

```sh
cargo xtask driver
DBGVIS_AUTO_CRATE=auto_poc \
RUSTC_WORKSPACE_WRAPPER="$PWD/tools/auto-register/wrapper.py" \
cargo build --offline --example auto_poc

rust-gdb -q -nx \
  -iex "add-auto-load-safe-path $PWD/target/debug/examples/auto_poc" \
  target/debug/examples/auto_poc
```

```text
break auto_poc::checkpoint
run
up
dbgvis print map
dbgvis print ordered
dbgvis print debug_only
```

The expected outputs are, respectively:

```text
{7: [Some(Point { x: 42 })]}
{1: "one"}
DebugOnly { count: 9 }
```

examples/auto_poc.rs contains no container registration, and map is not a field of an already-derived struct. Point has no Debug, which proves that the HashMap really goes through Visualize recursion. The IndexMap reuses its own Debug, which does not mean an element-level Visualize adapter for third-party containers has been implemented.

An ordinary `cargo build --example auto_poc` still builds and runs, but it will not auto-register these roots; the wrapper must be enabled. The consumer program still has to depend on dbgvis and call enable. DBGVIS_AUTO_CRATE is rustc's crate-name, and the first version only allows selecting a single bin/example; other compile requests are forwarded verbatim.

## How It Works

1. The wrapper starts two independent driver processes within a single Cargo target-compile request. The scan pass removes the official output/incremental directory, uses a temporary directory, stops after after_analysis, and runs neither LLVM codegen nor linking.
2. The driver queries the concrete mono instances, reads the user-variable debug records of the MIR of this crate's functions, and substitutes the generic arguments. It handles constants/ZSTs and composite variable fragments, and filters out macro-expansion sources, inlined sources, and registration helper functions. It does not promise to cover variable information lost after optimization.
3. It builds a DefId → accessible source path mapping from the public module children of the crate root and the loaded direct dependencies. Dependency aliases are matched against the actual --extern files; it never splices a diagnostic type string directly into Rust source.
4. It recursively renders the concrete arguments of primitive types, tuples, arrays, and accessible ADTs. It omits a type only when a trailing default type parameter equals the actual argument; const is currently limited to values convertible to the target usize. A type with a lifetime generates a single `'a` generic alias, and `#[dbgvis::register]` generates the formatter/anchor; anonymous, opaque, or unsized types still report SKIP. A top-level referenced temporary variable is skipped, to avoid mistaking the unconditional Visualize implementation of `&T` for T being formattable.
5. It reads the actual field types of the typed anchors already generated in the current crate, excluding local existing registrations; this is an experimental implementation that relies on the existing macro naming convention, not a stable registration-metadata protocol.
6. Once a root type satisfies Visualize, Debug, or Display, it generates register_type!, keeping the current auto priority. The candidate table is sorted by source expression, with a 4096 cap on the candidate count; the original runtime still checks the total count including existing roots.
7. The second pass injects the root submodule and include! via after_crate_root_parsing. Macro expansion, function instantiation, anchor/linkme, and GDB dispatch all reuse the existing implementation.

The generated artifacts are located at target/auto-register/plans/<crate>-<compile-unit-hash>/:

- register.rs: the Rust source actually compiled in on the second pass.
- register.tsv: CANDIDATE / EXISTING / SKIP with the diagnostic type and reason.
- invocations.log: a record of the real wrapper scan calls, used to distinguish execution from a Cargo-cached log replay.

The generated files are not rewritten when the source content is unchanged. The official dep-info tracks the generated files, the tool source, and the enabling environment variables. Generating the include file for the first time may cause the next build to run another pass, after which it should converge to fresh; the test checks the real call record rather than only the stderr that Cargo will replay. The scan does not read the previous pass's generated files, and does not keep adding formatter helper types to the candidates.

Once `DBGVIS_AUTO_CRATE` is set, every actual workspace compile the wrapper passes through (including forwarded libraries, build scripts, and proc-macros) uniformly goes through the driver's `Config::track_state`, recording in the rustc dep-info the raw values of `DBGVIS_AUTO_CRATE` and `DBGVIS_SCAN_DEPS` (including the unset state) and the tool-source dependency. Pass-through mode does not scan and does not inject registration; Cargo's compiler probes are still handed directly to rustc. This way, when the scan switch is toggled, libraries are recompiled to add or remove the MIR encoding, and the final executable also regenerates the registration, without needing a different target directory. Artifacts from an older wrapper version did not record these dependencies, so after upgrading you should clear the consumer project's build cache once, then verify toggling the switch.

## Verification Commands and Criteria

```sh
# Compile contracts, baseline/negative tests, auto_poc/demo debug and LTO build/run
cargo xtask autoregister

# Additionally, the real GDB breakpoint, output, call count, anchor, and normal-process-exit assertions
cargo xtask autoregister --gdb

# Run only the module/function/type visibility matrix (also includes debug and LTO)
cargo xtask visibility --gdb
```

At test time a consumer project (including the library used to validate the cross-library duplicate-registration boundary) is generated in the system temporary directory and cleaned up afterward; no boilerplate library is added to the repository.

| Item | Criterion |
| --- | --- |
| HashMap, IndexMap, DebugOnly with no manual registration | actual GDB output and call count agree in debug/LTO |
| Dependency renaming | generates ::dv, ::idx paths and builds/runs successfully |
| Local variable in a generic function | Vec<u16> of generic::<u16> appears in the plan |
| const generics | Packet<u8, 17> is generated and builds under both profiles |
| Local derive / explicit Display root | not registered twice, keeps the original root configuration, enable runs normally |
| Visibility matrix | 27 module/function/type combinations, each checking the registration source, report, and actual GDB output of T and Vec<T>; also checks that an uncalled private function and a same-package lib function are not scanned |
| `IndexMap<&str, ...>`, borrowed generic alias | generates the `'a` alias, passes GDB output in debug/LTO |
| Plain NoFormat | root trait not satisfied, skipped |
| Vec<NoFormat> | registration succeeds and the second pass compiles; at runtime the element renders as an `<unformattable …NoFormat>` placeholder |
| u64 already registered in a dependency library | both baseline and automatic mode run successfully, the runtime merges the duplicate root |
| Incremental behavior | a first-time warm-up is allowed; after that there is no actual scan and the generated source mtime is unchanged |
| Modifying a business type | rediscovers Vec<u128>; after removal the plan leaves no residue |

Markers: AUTO_COMPILER_CONTRACTS_OK, AUTO_UNFORMATTABLE_ELEMENT_OK, AUTO_CROSS_CRATE_DEDUP_OK, VISIBILITY_CONTRACTS_OK, VISIBILITY_GDB_OK, AUTO_GDB_OK, AUTO_DEMO_GDB_OK (the GDB markers once each for debug/LTO), AUTO_POC_SUITE_OK. A negative marker is a validated limitation, not the corresponding capability being implemented.

This round's acceptance: `cargo xtask autoregister --gdb` has fully passed the assertions above, with GDB 17.1. It has also passed `cargo fmt --all -- --check`, the standalone driver's rustfmt check, `cargo clippy --offline --workspace --all-targets --all-features -- -D warnings`, and `cargo test --offline --workspace --all-features`. The three core crates have no source changes.

### Actual Results of the Visibility Matrix

The test generates standalone `.rs` module files, combining root submodules `mod`/`pub mod`, `fn`/`pub fn`, and type visibility private/`pub(super)`/`pub(crate)`/`pub`, and further covers nested modules, re-exports, private methods, private generic functions, and function-local types. All type fields stay private. It uses `#[inline(never)]` and `black_box(&value)` before and after the breakpoint to preserve the function and variable under test; this does not promise that any variable can be discovered or addressed after optimization.

| Scenario | driver behavior / GDB result |
| --- | --- |
| private root submodule + private function + private Debug type | discovers T and Vec<T>, SKIP because the root scope cannot reference them; print refuses and does not call the target formatter |
| private root submodule + private function + pub(crate)/pub type | auto-registers T and Vec<T>, both output normally; does not require public fields |
| pub(super) type inside a root submodule | its visible reach arrives at the crate root, so it can be auto-registered |
| private type at the crate root + private function | can be auto-registered; different from a private type inside a submodule |
| deeply nested private modules and types | SKIP if any path segment is inaccessible from the root; pub(super) is only sufficient when it reaches all the way to the root |
| private nested module exposing a type via pub(crate) use | registered using the accessible re-export path |
| concrete instances of a private method and a private generic function | can be scanned and register the accessible concrete types |
| a Debug type declared inside a function | discovered but not nameable from the root, so both T and Vec<T> are SKIP |
| a private/function-local type deriving Visualize directly | the driver still SKIPs because of the path restriction, but the macro has already registered T in the original scope, so T can be printed; Vec<T> does not automatically become a root because of this |
| an uncalled private function, a private function in the same package's src/lib.rs | this test produces no candidate; the latter belongs to another crate, so even if the type is public its function body is not automatically scanned |

Both debug/LTO are verified: a printable root calls the dispatcher exactly once each time; a root that is skipped and has no separate registration does not call the dispatcher. The visibility matrix has been wired into the `cargo xtask autoregister` main entrypoint, and even without `--gdb` it still verifies the registration plan and the actual run under both builds.

## Dependency-Library Scanning (`DBGVIS_SCAN_DEPS`, off by default)

A non-generic function is codegen'd in its own crate and does not appear in the mono-item set of the executable target, so a library's local-variable types are invisible by default. For the common "logic in the lib, main is just the entrypoint" layout, this leaves auto-registration with almost nothing to scan — for example the `lesson3` of a regex tutorial project yields 2 roots when only the bin is scanned, and 31 with dependency scanning on, at which point `compile::OpCode` enters registration.

How it works: the wrapper adds `-Zalways-encode-mir` to the workspace libraries that pass through it and records the crate name, then hands the driver a regex list of "which crates to scan" (if the user set `DBGVIS_SCAN_CRATES` it is forwarded verbatim, otherwise it is the escaped form of the recorded library names); the driver full-matches each of `tcx.crates` and reads the MIR of the hits. Enumeration covers three kinds: free functions in the module tree, a type's inherent methods, and the whole crate's trait implementations (`impl Trait for Type` is neither a module child nor reachable from the implemented type, so it must be enumerated per crate separately). "Non-generic" is judged only by type and const parameters, not lifetimes — `Instance::mono` erases them, and requiring completely no generic parameters would drop an impl method with early-bound lifetimes entirely. The list is explicit rather than "scan whatever has MIR available" — the distribution's std/core also encode MIR, and 6683 non-generic std functions were measured to be readable; filtering by availability would turn the entire standard library's local variables into roots.

The switch is controlled by an environment variable read by the wrapper, off by default:

```sh
cargo build                     # scans only the executable target itself, without -Zalways-encode-mir
DBGVIS_SCAN_DEPS=1 cargo build  # also scans the workspace dependency libraries
```

It can also be written into the `[env]` section of the consumer project's `.cargo/config.toml`.

`DBGVIS_SCAN_CRATES` decides **which** crates to scan: comma-separated, each segment a regex that full-matches the entire crate name (in underscore form, so `regex` selects only `regex`, and `regex.*` also brings in `regex_syntax`). Setting it implicitly turns scanning on, so `DBGVIS_SCAN_DEPS=1` is no longer needed; when unset, `DBGVIS_SCAN_DEPS=1` is equivalent to "all workspace libraries."

```sh
DBGVIS_SCAN_CRATES='mylib,regex.*' cargo build
```

A pattern that matches no scannable crate is reported on stderr, and an illegal regex fails outright and points out which segment. Two kinds of crate are never scanned regardless of the pattern: those in the sysroot (the distribution's std/core also encode MIR, and 6683 non-generic functions were measured to be readable; scanning them in would turn the entire standard library's local variables into roots) and dbgvis itself (its internal types are meaningless as roots). The wrapper only adds `-Zalways-encode-mir` to workspace libraries, so even a registry crate matched by a pattern has, by default, MIR only on its `#[inline]`/generic functions — measured on regex-syntax 0.8.11, scanning directly gets 15 roots, and 51 after MIR is retained. To scan a registry crate you do not have to move it into the workspace, just hand the flag to cargo, in one of two ways:

```toml
# targeted: give it only to the target package (nightly cargo, requires declaring cargo-features at the top of Cargo.toml)
cargo-features = ["profile-rustflags"]
[profile.dev.package.regex-syntax]
rustflags = ["-Zalways-encode-mir"]
```

```sh
# whole graph: convenient, but every rlib in the dependency graph grows
RUSTFLAGS=-Zalways-encode-mir cargo build
```

Both yield exactly the same root set; with the targeted form the other crates' rlib sizes are unchanged, and regex-syntax itself grows by about 3.8%. When the wrapper detects that the flag is already in the arguments it does not add it again, and cargo also counts rustflags into the fingerprint.

Indirect dependencies can be scanned too. The generated registration code needs to write the type path out from the crate root, but cargo only passes `--extern` for direct dependencies, and an indirect crate is not in the executable target's extern prelude — measured by indirectly pulling in `regex_automata` via `regex` and scanning it: 170 types have MIR, 0 can be registered, and 155 report `type is not accessible/nameable from root`. Now the driver, during the scan phase, writes the "selected but non-direct" crates together with their rlib paths into the plan directory (`register.externs`), and the wrapper adds `--extern <name>=<rlib>` in the single call that compiles the generated registration. The crate is already linked; this just makes its name available; stderr will print `exposing indirect dependency ... via --extern`. Two situations are not auto-exposed but instead prompt the user: multiple versions of the same name linked at once, or a clash with the alias of some direct dependency (occupied names are collected from all the `--extern` that cargo passes, including a dependency that is declared but unused in source, otherwise a second `--extern` would appear and report E0464). When the library name is a keyword (e.g. `[lib] name = "async"`), the path root is generated from the raw identifier as `::r#async`, and the name handed to `--extern` stays as-is.

It is off by default because the cost is real:

- `-Zalways-encode-mir` grows the workspace libraries' rlibs and slows the build.
- The candidate count rises significantly, and the registration table and artifacts grow with it; a container whose elements are unformattable no longer causes a build failure, but only produces an `<unformattable …>` placeholder in the output.
- It covers only workspace members: `RUSTC_WORKSPACE_WRAPPER` does not act on registry dependencies, so third-party libraries are out of scope.
- The scan scope is judged by the crate a definition belongs to, not by the re-export path: modules, functions, types, and inherent methods all check `DefId.krate`. A selected library's re-export of an external function/type does not bring in the external function's local variables; the selected library's own re-exports are still supported, and an external type is still registrable when it appears as a local variable of the executable target itself.
- The list files are written under `target/auto-register/scan/<crate>/`. A name not actually linked by the executable target matches no crate, so entries left over from an old session do not widen the scan scope.

`cargo xtask autoregister` includes two regression acceptances: the registration change and cache convergence of `DBGVIS_SCAN_DEPS=0 → 1 → 0` within the same target directory; and that when a workspace library re-exports a function, module, or struct/enum/union inherent method of a dependency outside the workspace, the scan does not cross the boundary.

## Unsolved Productization Problems

- **Cross-library dedup and configuration**: same-name roots merge their capabilities and function pointers only when the lifetime-erased concrete-type TypeId agrees, with size/align as an additional check; same-name-different-identity or inconsistent-shape is refused, and cross-version runtime tables are not merged automatically. A proper solution still needs queryable registration metadata and a clearer pattern priority.
- **Opt-out semantics**: the existing no_register only turns off derive registration; the experimental driver does not interpret it as a global ban on auto-registration. There is no per-type exclusion configuration yet, so do not use it for a project that depends on that opt-out behavior.
- **Scope and nameability**: there is no private-module/block-level injection, no transitive-dependency path synthesis, type-alias search, or all const forms. Root-path traversal still conservatively excludes underscore-prefixed names. The lifetime alias currently uses a single `'a` uniformly, and if a type's trait implementation depends on multiple unequal lifetime relations, the second pass will be refused by the Rust compiler.
- **Coverage**: it handles the visible MIR debug variables of the current target's local functions, plus the same kind of variables in workspace dependency libraries' non-generic functions when `DBGVIS_SCAN_DEPS=1` is explicitly on. It does not scan the standard library, registry dependencies, or macro-generated variables; closure interiors have no dedicated acceptance, and inlined sources are filtered. It is not whole-program type reflection.
- **Cache and distribution**: only the local Cargo ordinary incremental path is validated; sccache, cross-compilation, parallel multi-target, no_std, and general cargo-dbgvis install/upgrade are still unaccepted. It does not modify ~/.cargo or ~/.gdbinit.

Priority order going forward: first define the registration/capability metadata and the auto-registration opt-out semantics, then implement safe candidate filtering; finally extend the scope and the install experience. Do not add regex-symbol lookup into the call-safety chain.
