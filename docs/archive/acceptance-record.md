# Implementation and acceptance record

> Phase-one historical archive: the implementation and tests below correspond to the old v1, and do not mean the current code still provides these interfaces. For current evidence, see the [Phase-two acceptance record](../phase-two-acceptance.md).

Validation date: 2026-09-10. This records local run evidence; the GitHub Actions workflow is configured, but there is no claim that remote CI has run.

## Delivery scope

P0–P5 in the implementation plan have landed in the runtime, registration macros, the two debugger adapters, container adaptation, examples, fault tests, documentation, and CI configuration. The original `debug_print_*` / CString protocol and the two hardcoded scripts have been removed.

| Plan requirement | Current implementation | Validation evidence |
| --- | --- | --- |
| P0: fix type mapping, loading, and change display | Switched to a registry; a separate dbgvis category; two checkpoints in the example | `gdb_checks.py`, `lldb_checks.py`, `codelldb.py` check results before and after insertion/deletion |
| P1: unified entry point, mailbox, and output region | The runtime's Module, Runtime, Request, Response | `runtime.rs::wire_layout_is_stable_on_v1_target` plus real GDB/LLDB calls |
| P1: UTF-8, empty values, NUL, exact capacity, truncation | BoundedWriter implements fmt::Write directly | writer unit tests, `formats_borrowed_data_and_distinguishes_modes`, small-capacity debugger output |
| P1: error, panic, reentrancy, argument checks | Integer status codes and capture of Rust-internal panics | runtime error tests, standalone faults subprocess |
| P1: no heap allocation in the bridge layer | Preallocated output region, no per-call CString allocate/free | `bridge_does_not_allocate_for_nonallocating_formatter` using a counting allocator |
| P2: register once, monomorphized function pointers | `register_visualizers!`, with optional Debug/Display/Children | four kinds of registration in the example; a compile-fail doctest with no Display implementation |
| P2: type identity and aliases | Owning main module, full name, size/alignment, ambiguity rejection | host protocol tests, GDB objfile and LLDB canonical type checks, real generic calls |
| P2: link retention | reachable retain anchors | `build_profiles.py` checks the final nm output for debug/release/LTO |
| P3: three output modes, type coverage, explicit and automatic | shared Config/Session and dbgvis commands | two integration test suites and a CodeLLDB DAP test |
| P3: coexistence with existing formatters | GDB does not take over native; LLDB registers a separate summary/synthetic by capability | integration test registers a NATIVE_POINT formatter, then verifies switching and restoration |
| P3: standalone embedded script, repeated loading | build.rs bundles GDB; LLDB source-load entry point | binary copied to a temporary directory loads successfully; two repeated-import test suites |
| P3: no address, wrong ABI, core fallback | pre-request checks and the native path | temporary values actually created by the debugger are rejected; host wrong-ABI test; core dump test |
| P3: timeout and interrupt state | backend timeout, poisoned session state | hang subprocess test; refresh does not clear the interrupted state |
| P4: Bytes, IndexMap, custom fields | public container interface, stable field borrow addresses | Rust adapter tests and typed child-node checks in both debuggers |
| P4: temporary scalars and host buffer ownership | Child::scalar; LLDB SetDataWithOwnership | u64::MAX child item, temporary value rejection, LLDB/CodeLLDB elision hint checks |
| P4: paging, caps, lazy fetching | at most 64 logical items per page; elision hint and page command | 10000-element container page queries; LLDB 64 child items require only one page call |
| P4: caching and state updates | stop/expression generation, config, object header changes, refresh | writing len at the same stop, writing point, continuing, restarting, and swapping the binary |
| P4: non-static references, empty containers, slices, nesting, cycles | &str slots and pointer descriptors, expanded level by level | adapter unit tests; Record nesting and depth-capped pointer display |
| P5: failure scenarios and process lifecycle | a disposable faults example, external timeout supervision | panic/error/hang/exit/abort validated independently in both GDB and LLDB |
| P5: CodeLLDB integration | initCommands + preRunCommands | actually launches the CodeLLDB adapter, sending DAP scopes/variables/evaluate/continue |
| P5: documentation, performance, CI | usage guide, protocol v1, this record, GitHub workflow | local checks and the performance measurements below; the remote workflow was not run in this session |

The files in the table above live in `crates/*/tests`, `tests/test_protocol.py`, and `tests/integration`. By default native does not execute this component's target functions; the default Debug behavior of the explicit command is described in the usage guide.

## Local support matrix

| Environment | Validation scope |
| --- | --- |
| Linux x86_64, little-endian | Primary support target, native reading of local processes and core dumps |
| Rust 1.99.0-nightly, 2026-08-10 | workspace tests, debug/release/LTO, debugger integration |
| Rust stable 1.97.1 | workspace tests; the implementation does not require nightly features |
| GDB 17.1 | native and trait summaries, paging, faults, restart/program swap, standalone embedded loading |
| LLDB 22.1.3 | same as above; map children are flattened key/value; Rust expressions bridged via the C ABI |
| CodeLLDB 1.12.2 | variable summaries, child nodes, mode switching, and continue-run through the real DAP adapter |
| GitHub Actions / Ubuntu 24.04 | workflow is configured, no remote run record yet corresponds to this change |

Older versions, remote debugging, multiple dynamic-library registries, other architectures, and other systems are not proven by these results. The no-feature build was checked to contain no entry point or registry; compile-time capacity coverage is checked by a separate build with `DBGVIS_BUFFER_BYTES=8192`.

## Performance baseline

Measurement command: `python3 tests/integration/run.py --benchmark`. Uses the current local debug example; the timing includes host protocol handling and target calls; a single measurement is not a stable performance commitment.

| Workload | GDB | LLDB | Target call count |
| --- | --- | --- | --- |
| 100 short summaries | 25.886 ms | 228.125 ms | 100 each |
| 128 distinct Point values | 33.178 ms | 266.972 ms | 128 each |
| 10 pages of 64 elements from a 10000-element Bytes | 3.213 ms | 21.692 ms | 10 each |
| LLDB synthetic consecutive read of the first 64 child items | — | tests page-cache reuse | one CHILD_PAGE; plus a length query |

The script outputs new JSON measurements, allowing later comparison. The current threshold is based on call count and bounded data volume, and does not set millisecond assertions that are easily affected by machine load.

## Reproducible checks

```sh
cargo fetch --locked
cargo fmt --all -- --check
cargo clippy --offline --workspace --all-targets -- -D warnings
cargo test --offline --workspace
python3 -m unittest discover -s tests -p 'test_*.py'
python3 tests/integration/build_profiles.py
python3 tests/integration/run.py --faults --core --benchmark
python3 tests/integration/codelldb.py --adapter /path/to/extension/adapter/codelldb
```

The integration runner sets a 45-second external time limit for each GDB/LLDB subprocess; target calls have an additional GDB/LLDB time limit. The core and standalone-binary tests only create files in their own newly created temporary directories and clean up afterward.

The CodeLLDB test uses a local loopback TCP connection and the real adapter, without launching a GUI; what it validates is the same DAP data flow as VS Code, and it cannot replace testing of all editor UI behavior.

## Design convergence and limitations

- The macro in the documentation draft added a required `name` and an optional `aliases`, used to explicitly record name differences in the debug info. No Python dictionary or additional export function is needed.
- The first version uses statically preallocated memory. Summary length, page count, names, and type names all have caps; insufficient output does not retry automatically.
- LLDB's generic name lookup may fail, so when needed a DWARF type catalog is built for the current root module, to verify against the actual canonical type identity. The type catalog is cached by module identity, and does not treat same-named strings as the same type.
- LLDB's structured map uses the plan-permitted `[i].key` / `[i].value` flattened representation.
- This component does not pre-traverse the object graph recursively; expansion depth uses the debugger's path-depth control; shared objects are not globally deduplicated.
- GDB does not cache values across requests; LLDB caches only provider pages and checks the execution generation and the object header. For external state writes that cannot be observed and that affect a custom adapter, an explicit refresh is required; we do not claim that arbitrary memory writes can be detected automatically.
- GDB's timeout granularity is whole seconds, so a 200 ms configuration is actually rounded up to 1 second. LLDB uses a microsecond interface. Neither can roll back fmt side effects.
- Bounded storage does not guarantee that the user's fmt is allocation-free, lock-free, pure, or recoverable; capturable panics and process abort are explicitly distinguished.
- Automatically collecting all traits, constructing a Formatter directly, remote/cross-architecture, pure in-memory container adaptation, multiple dynamic libraries, and proc-macro extensions remain the original plan's follow-up scope.
