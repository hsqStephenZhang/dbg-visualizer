# debugger_visualizer phase one implementation plan (archived)

Status: the original phase one design is archived, and P0–P5 have been delivered. Subsequent work follows the phase two implementation plan (deleted, see the Git history) and no longer treats paging expansion as the primary integration approach. For the actual phase one API and local validation evidence, see the [Acceptance record](acceptance-record.md), the [Usage guide](../usage-guide.md), and [protocol v1](protocol-v1.md). The originally proposed plan is preserved below; where the illustrative syntax differs from the actual interface, defer to those documents.

## 1. Goals and terminology

Evolve the current POC—hand-written per-type FFI functions and a Python mapping table—into a debugger-visualization component with registrable types, selectable output modes, and expandable containers.

This document defines "native formatting" as **using the display methods that GDB / LLDB already provide, including the pretty-printers supplied by the toolchain**, with the configuration name `native`. The two modes that invoke a Rust type's own implementations are named `debug` and `display` respectively, to avoid having "native" refer to both the debugger display and the Rust trait implementations.

Core goals:

1. The user can select the `native`, `debug`, or `display` summary output mode.
2. `debug` / `display` write directly into a bounded buffer of adjustable capacity, reusing the existing trait implementations.
3. Provide a unified invocation entry point, a discoverable type registry, and per-type function pointers.
4. Containers support on-demand, paged expansion, and child elements retain type and further-expansion capability.
5. A single Rust registration declaration serves both GDB and LLDB, reducing manual mapping.
6. Every stage produces a result that can be independently demonstrated and verified.

The first version's scope is Linux x86_64, local live processes, GDB and LLDB; CodeLLDB validates its integration behavior. Cross-architecture, remote debugging, and other operating systems are treated as later compatibility phases and are not committed to in advance. Core dumps support `native` and any display path that does not require target execution.

## 2. Current foundation and problems to fix first

The current call chain is: Python obtains the variable address → Rust C ABI wrapper function → `format!` / `write!` → `CString` → Python reads → Rust frees.

Reusable parts already in place:

- The native `Debug` calls for `Bytes` and `IndexMap<i32, &str>` in `src/main.rs`.
- The GDB pretty-printer integration in `src/printer.py`.
- The LLDB summary integration in `src/lldb_linter.py`.
- The CodeLLDB script-loading entry point in `.vscode/launch.json`.

Known problems to address:

- GDB's `IndexMap` mapping uses a nonexistent `debug_print_indexmap`.
- A regex function with no implementation yet is registered, which easily lets the capability table drift from the actual implementation.
- LLDB registers a category but does not explicitly enable it; repeated imports and registration scope also need to be managed.
- The Python exception path can skip freeing the CString, and LLDB's free failures are ignored.
- Rust handles formatting and CString-construction errors with `unwrap()`, which is unsuitable for a general-purpose interface.
- LLDB's read limit is 2048 bytes, but Rust still generates the full string first, so this is not truly bounded output.
- The two sets of scripts hard-code the debugger type names; differences such as `i32` / `int` need to be adapted.
- `#[used]` alone cannot guarantee that the entry point and registry are retained in the final linked artifact.
- `p` may generate a temporary value, but "no `p` value has an address" cannot be taken as a fixed rule; the actual storage location of each value should be checked.

## 3. User configuration and output behavior

### 3.1 Summary, execution permission, and expansion are configured separately

The following is the proposed configuration; the specific command syntax is fixed during the adapter-layer stage:

| Configuration | Values / suggested initial value | Semantics |
| --- | --- | --- |
| `summary.mode` | `native` / `debug` / `display`; default `native` | summary display method |
| `summary.alternate` | default `false` | `debug` uses `{:#?}`; `display` uses `{:#}` |
| `summary.buffer_bytes` | default 4096 | maximum UTF-8 bytes written for this formatting |
| `execution` | `manual` / `automatic`; default `manual` | target code executes only via explicit commands, or the variable view is allowed to execute automatically |
| `children.mode` | `native` / `structured`; default `native` | use the existing child nodes, or use the registered element enumerator |
| `children.page_size` | default 64 | number of logical elements per page; a key/value pair in a map counts as one element |
| `children.max_items` | default 1024 | upper bound on logical elements displayable during this pause |
| `execution.timeout_ms` | default 200 | target request timeout, effective only when the backend supports it |

Global defaults and per-type overrides are supported; an explicit print command can temporarily override the mode, capacity, and alternate without modifying the long-term configuration. Priority: command arguments > type configuration > global configuration.

Proposed usage examples:

```text
visualization config: default native
visualization config: MyType uses display, 8192-byte buffer, automatic calls allowed
visualization config: IndexMap uses debug summary and structured children
explicit print: print map in debug mode, 16384-byte buffer
```

When `execution=manual`, the variable view keeps using native; only an explicit command runs the selected Rust formatter. The configuration and help text must explain this. Once automatic calls are chosen, the user does not need to confirm each time.

### 3.2 Modes and fallback rules

| Request | Actual behavior |
| --- | --- |
| `native` | this component does not execute target functions; it hands back to the existing formatter |
| `debug` with the capability registered | invoke the concrete type's `Debug` adapter function |
| `display` with the capability registered | invoke the concrete type's `Display` adapter function |
| the selected trait is not registered | the automatic view falls back to native; an explicit command reports the missing capability |
| no valid target address, process not executable, or ABI incompatible | fall back to native and provide a queryable reason |
| buffer insufficient | return the valid UTF-8 prefix written so far and mark "truncated" in the UI |
| target call fails | do not return a stale cache masquerading as a new result; fall back to native and retain diagnostic information |

`display` is not silently changed to `debug`. `Bytes` and `IndexMap` register `Debug` first; `Display` is validated with a custom example struct, without assuming a third-party container implements `Display`.

The native fallback must avoid re-entering this component: GDB decides not to take over at the lookup layer; LLDB handles it via a separate category, registration enable/disable, and a non-synthetic raw-value path. For the case "a failure occurs after this component has already taken over," verify whether the original formatter can recover; when it cannot, display the raw fields and a clear reason, and do not claim that the native formatter was restored.

## 4. Overall architecture

```text
user configuration / explicit command
        ↓
GDB adapter layer / LLDB adapter layer
        ↓
shared protocol, type matching, cache, and error handling
        ↓
read the versioned registry → determine capabilities and target state
        ↓
unified C ABI entry point
        ↓
concrete-type adapter functions
   ├─ Debug → bounded writer
   ├─ Display → bounded writer
   └─ children → paged child-element descriptors
```

The native path ends directly at the adapter layer and does not pass through this component's unified execution entry point. The behavior of a pre-existing third-party formatter is defined by that formatter itself.

Suggested final directory structure:

```text
crates/visualizer-runtime/       registry, protocol, dispatcher, bounded writer
crates/visualizer-adapters/      container adapters such as Bytes, IndexMap
debugger/common/                protocol decoding, configuration, type matching, cache
debugger/gdb/                   GDB pretty-printer, commands, children
debugger/lldb/                  LLDB summary, synthetic children, commands
examples/                      example programs with breakpoints for verification
tests/integration/             GDB, LLDB batch verification
docs/                          usage guide, protocol, support matrix
```

Implement the declarative registration macro first; only add a proc-macro crate when there is a clear benefit. When migrating to a workspace, retain one demonstration entry point with clear behavior.

## 5. Unified entry point and ABI

### 5.1 Protocol form

The first version adopts "one execution entry point + one exported module descriptor." Illustrative signature:

```rust
// Design sketch; the field layout and error codes must be fixed in phase one.
unsafe extern "C" fn dbgvis_dispatch_v1(request_addr: usize) -> u32;
// Also export DBG_VIS_MODULE_V1 for the debugger to read directly.
```

The module descriptor contains: ABI version, structure sizes, target pointer width, type-table address and count, request mailbox address, output region address and capacity, and dispatcher locating information. Discovering the static metadata does not require executing a Rust function first.

A request includes: structure size, request sequence number, opcode, type-entry ID, object address, mode, capacity, and paging parameters; the response is written into a fixed response region owned by the module. The entry point returns only an integer status code, avoiding a complex return struct across debuggers.

First-version opcodes:

- `FORMAT`: format the summary.
- `CHILD_COUNT`: query the number of logical child elements and the container shape.
- `CHILD_PAGE`: read one page of child-element descriptors.

The protocol structures use `#[repr(C)]`, explicit integer and length fields, and do not expose the internal layout of Rust `String`, `Vec`, trait objects, `Result`, or enums. Python decodes according to target-side information and does not use the host machine's pointer width and byte order as default assumptions.

The entry point validates the request version, structure size, opcode, type ID, capability bits, capacity, and integer-arithmetic overflow. The first version only accepts the request mailbox address declared by the module, to reduce the exposure of arbitrary request pointers; the object address must still satisfy type, alignment, and liveness constraints.

The error codes include at least: `OK`, `TRUNCATED`, `UNSUPPORTED`, `INVALID_REQUEST`, `ABI_MISMATCH`, `BUSY`, `FORMAT_ERROR`, `PANIC`. The debugger separately records backend errors such as inability to execute, unreadable memory, timeout, and missing address, and does not disguise them as Rust return values.

### 5.2 Type registration and function pointers

Each type entry contains:

- module-internal type ID, display name, size, and alignment requirements.
- GDB / LLDB name-matching information and explicit aliases.
- capability bits: `DEBUG`, `DISPLAY`, `CHILDREN`.
- optional concrete-type `debug_fn`, `display_fn`, `children_count_fn`, `children_page_fn`.
- container shape and adapter version (plain struct, sequence, map).

The function pointers are invoked by the Rust dispatcher; Python does not directly call any type's internal functions. The module may use the Rust calling convention internally, but it must not be treated as a cross-module, cross-version plugin ABI.

Proposed usage of the registration macro:

```rust
// API design draft only; not an existing macro.
register_visualizers! {
    bytes::Bytes => { debug, children: BytesElements },
    indexmap::IndexMap<i32, &str> => { debug, children: IndexMapEntries },
    MyType => { debug, display },
}
```

The macro generates monomorphized adapter functions and static entries, and the compiler checks the declared trait capabilities. The first version enumerates the concrete types centrally in the executable and does not attempt to auto-collect all trait implementations across dependencies. Generics are registered per concrete instance; registering a single abstract `IndexMap<K, V>` cannot cover all instances.

Types carrying references must have an explicit lifetime-erasure strategy: they only borrow the object within the current call, do not require the actual data to be `'static` just because the type is registered, and do not impose a `'static` bound via `Any`. The example adds a non-static string reference to validate this constraint.

Type matching is done by "module identity + full type match + size/alignment cross-check." The ID does not use `TypeId`, whose cross-build stability is undefined, and does not rely solely on a name hash. Size/alignment and name can only reduce mismatches, not prove that an arbitrary pointer is valid. Normalizations such as `i32` / `int` must be handled according to target information; blind string replacement is forbidden. When a name is ambiguous, execution is refused.

The first version supports only a single root registry; discovery across multiple dynamic libraries, same-name symbol disambiguation, and module unloading are scheduled for later phases.

### 5.3 Symbol retention and script distribution

- The demo program references the module descriptor and dispatcher via reachable initialization/anchor code.
- Make final-binary symbol checks an acceptance item, covering debug, release, and LTO.
- Do not equate `#[used]` with a linker-level retention guarantee; when necessary, provide explicit link-retention configuration for the supported platforms.
- The entry point and registry layout carry a version number, and scripts should fall back when they encounter an incompatible version.
- GDB's embedded script should preferentially produce a standalone loadable bundle, solving the path problem of embedded scripts importing shared Python modules.
- LLDB provides an explicit loading entry point; the CodeLLDB configuration uses the same artifacts.
- Document the GDB auto-load safe-path configuration, and forbid globally disabling load protection as a default installation step.

## 6. Bounded formatting buffer

### 6.1 Memory and ownership

The first version reserves the request, response, and output regions inside the runtime, with a suggested default output-region limit of 64 KiB. The user's `summary.buffer_bytes` can be adjusted between 1 and that limit; when a larger limit is needed, change the reserved capacity via a compile-time configuration and publish it in the module descriptor. Exceeding the limit reports an explicit error and does not silently grow memory.

The debugger writes parameters into the fixed request mailbox, executes the unified entry point, and then reads the module-owned output region according to the response length. This way the first version does not depend on GDB-side target-memory allocation, nor does it need to free a CString after each return.

The buffer is compiled into the program only when the debug-specific feature is enabled. A single module handles only one request at a time, using non-waiting re-entry protection; when it is occupied it returns `BUSY` immediately. Python additionally maintains session-level call protection to prevent the formatter from recursively triggering a new call.

The response carries the request sequence number, and the debugger verifies that it matches the request. The output must be copied into debugger memory before the next call; all operations that modify the output region share the same serial channel. The sequence number is used to detect stale responses and does not replace serialization.

### 6.2 Writer semantics

Directly implement `std::fmt::Write`, letting `write!` write content into the reserved region:

- Do not `format!` into an unbounded String and then truncate.
- Compute capacity in bytes, but only keep the valid UTF-8 prefix; embedded NULs are supported.
- The response returns `written_len` and the truncation status; Python reads the exact length and does not call a C-string read interface.
- On the first inability to hold content, set the truncation flag and return `fmt::Error`; subsequent writes stay in the rejected state.
- If a `fmt::Error` is received without truncation, return `FORMAT_ERROR`, distinguishing it from insufficient capacity.
- Do not guarantee the length needed to return the full output, since obtaining that length might require performing the full formatting.
- Do not automatically grow the capacity and retry; only re-run formatting when the user explicitly requests a larger capacity.

The buffer constrains the bridge-layer storage, not the compute cost or internal memory allocation of an arbitrary user `fmt`. A user implementation may ignore write errors, keep looping, or even run indefinitely without writing output; a timeout still requires debugger cooperation.

### 6.3 Call-failure boundary

In a `panic=unwind` build, catch a recoverable panic inside Rust, within the C ABI boundary, and convert it into `PANIC`. Freedom from side effects cannot be guaranteed—the panic hook may still run—and the global panic hook is not temporarily replaced. Recovery cannot be promised for `panic=abort`, illegal memory access, or process crashes.

Do not wait on the internal mutex and do not perform automatic retries. After a target call is interrupted, first check the process and call-frame state; when it cannot be confirmed that the dispatcher has exited, mark this session's target calls as unusable, and do not forcibly clear the busy flag and continue re-entering.

## 7. Container expansion

### 7.1 Child-element protocol

Summary and expansion are independent, allowing a `debug` summary with `native` children, and also allowing a `native` summary with explicitly configured `structured` children.

`CHILD_COUNT` returns the total number of logical elements; `CHILD_PAGE(start, count)` returns one page of descriptors with a fixed upper limit, and validates index and count overflow. It does not copy the entire container at once and does not reverse-parse elements from the Debug string.

Each descriptor contains at least a name or index, a type reference for the value, a target address or inline scalar, and a value category. A map's single logical-element record contains two descriptors, key and value. Name and descriptor storage also have their own fixed capacity, and the summary buffer limit cannot be bypassed with a large number of labels.

Child elements do not necessarily register Debug / Display; when no formatting capability is registered, a typed value can still be reconstructed from debug information and displayed with native. When no trustworthy type information is found, return a non-expandable scalar/diagnostic rather than doing a speculative pointer cast.

Elements with stable storage may return a borrowed address; temporarily computed values must be copied into the bounded response region and immediately copied by the script, and returning the address of a temporary local variable is forbidden. The first version's complex child values only support types with a stable storage location in the original object; fat pointers such as `str` and slices are handled via explicit descriptors or an actual reference slot, and are not treated as an ordinary address.

### 7.2 First batch of containers

| Type | Expansion method | Verification focus |
| --- | --- | --- |
| `Bytes` | return u8 elements by index using the public access interface | empty value, offset after slicing, shared underlying storage |
| `IndexMap<i32, &str>` | return key/value in insertion order, paged | order after insert/remove, non-static &str, page boundaries |
| custom struct | register field names and borrowed addresses via a macro or adapter | nested values, fields with missing traits, reference cycles |

The Rust adapter preferentially obtains the length and element positions through the container's public interface, avoiding Python hard-coding a third party's private layout. This also means a `structured` query executes target code and is subject to the same execution policy.

A pure memory-reading container adapter can be a later independent capability: it suits core dumps but needs a separately maintained layout version, and it cannot claim that the existing Rust enumerator requires no target execution.

### 7.3 Debugger display and caching

- GDB: implement `children()`; sequences use the array shape, and maps use alternating key/value child items following GDB conventions along with a map hint.
- LLDB: implement a synthetic children provider; a map can display `[i]` nodes with `key` / `value` underneath. Do not fabricate contiguous key/value objects that do not match the actual layout just for display convenience.
- If the first-version LLDB cannot reliably express a pair of address-separated key/value, first flatten typed `[i].key` / `[i].value` nodes and note this in the usage documentation.
- When `max_items` is exceeded, show an explicit elision hint; provide a command to change the limit or explicitly query a page.
- The cache key includes the process, module, pause generation, object address, type, configuration, and page number; even within a single pause it must be invalidated in response to debugger writes, expression execution, and detectable target calls.
- Clear the relevant cache after resume, single-step, module load/unload, and type-configuration changes; LLDB uses the stop ID, and GDB maintains a corresponding event generation.
- Every target execution by this component invalidates at least other objects' caches; when external execution or writes cannot be detected, do not claim the cache is reliable—provide an explicit refresh and adopt a conservative caching strategy.
- Handle this component's own formatter re-entry protection separately from object-graph cycle detection; limit depth by access path, avoiding global deduplication that would wrongly affect shared child objects.

## 8. Phased implementation and acceptance

Dependency order: P0 → P1 → P2 → P3 → P4 → P5. Cross-platform extension comes after P5.

### P0: fix the POC and establish a running baseline

Deliverables: fix the existing mapping and loading problems; prepare fixed-breakpoint examples; record the debugger and toolchain versions. If this stage continues to keep the CString protocol, complete the free and error paths, and remove the old protocol after P1/P3 are finished.

Acceptance: GDB, bare LLDB, and CodeLLDB can display Bytes and IndexMap; stopping consecutively before and after insertion shows the value change; unimplemented regex capability is no longer mistakenly registered.

### P1: implement the runtime protocol and bounded formatting

Deliverables: versioned descriptor, unified entry point, mailbox and output region, status codes, bounded writer; first register two concrete types and one Display example by hand to verify the protocol is usable.

Acceptance:

- `Debug` / `Display` output correctly, and the bridge layer does not depend on CString allocation and freeing.
- Verify exact capacity, empty output, multi-byte characters, embedded NULs, truncation, fmt errors, and catchable panics.
- Request errors, over-large capacity, missing capabilities, and re-entry return explicit statuses; tests use validly allocated objects and do not test "safety" by dereferencing random addresses.
- For a controlled allocation-free fmt implementation, verify that the bridge layer performs no heap allocation, without generalizing the conclusion to arbitrary fmt.

### P2: generate the type registry and stabilize symbol discovery

Deliverables: declarative registration macro, monomorphized function pointers, explicit type aliases, centralized registration entry point, and a symbol-retention strategy.

Acceptance: adding a new type only requires implementing the trait and registering it, with no new named FFI exports or Python dictionary entries; declaring a nonexistent trait capability fails to compile; same-shape different types do not mismatch; the registry and entry point are discoverable in the final debug/release/LTO binaries.

### P3: unify the two debugger front ends and the configuration

Deliverables: shared protocol layer; native/debug/display modes; capacity, type override, explicit print, and automatic-call configuration; GDB embedded bundle script and LLDB installation entry point.

Acceptance:

- The native path does not initiate target calls from this component; an existing formatter is not shadowed by this component.
- Under manual, only explicit commands execute; under automatic, the variable view outputs per configuration.
- The two debuggers produce consistent valid text and truncation status; capacity and mode changes take effect immediately.
- GDB auto-load, LLDB repeated imports, and CodeLLDB loading have repeatable steps.
- No-address values, ABI mismatch, and non-executable processes fall back correctly; temporary values are not passed off as valid object addresses.
- The timeout capability is probed; backends that do not support an interruptible timeout clearly indicate the limitation, and by default do not execute automatically.

Completing P3 forms the first usable version: after a type is registered, output the summary per mode.

### P4: implement paged container expansion

Deliverables: child-element protocol, Bytes / IndexMap adapters, GDB children, LLDB synthetic children, paging cache and refresh.

Acceptance:

- Expand Bytes and IndexMap in both debuggers, with child elements having the correct type and value.
- Switching summary modes does not break expansion; selecting native children can restore the existing tree display.
- A large container only requests the pages the user expands; a single return does not exceed the configured limit.
- After map insert, delete, and container resume execution, old element addresses are not reused.
- Verify non-static string references, empty containers, nested objects, and reference cycles.
- The tree appearance of GDB and LLDB may differ, but the element order, type, count, and paging semantics are consistent.

### P5: failure handling, release documentation, and continuous integration

Deliverables: controlled-failure scenarios, support matrix, installation steps, registration guide, protocol description, performance records, and CI.

Acceptance:

- Standalone subprocesses cover controlled panic, call timeout, formatting recursion, and target exit; the tests themselves are supervised by an external timeout to avoid CI hangs.
- `panic=abort` is verified to be an explicit target-process exit and cannot be counted as a recoverable error.
- Across multiple runs, reloads, and binary swaps within the same session, no stale registrations or caches remain.
- On a core dump, native is available and active formatting is explicitly unavailable, with no target calls performed.
- Record the latency and call counts for short summaries, many variables, and large paged containers; measure the baseline first before setting a performance threshold.
- The documentation makes clear that bounded storage does not equate to a pure function, safety at arbitrary breakpoints, or a strict execution time limit.

After P4/P5 are finished, release the first version with a structured container view.

## 9. Later extensions and explicit boundaries

Later possibilities: a proc-macro to auto-register custom struct fields; explicit cross-crate composition of registries; multiple dynamic libraries; remote and cross-architecture debugging; pure in-memory container adapters; more format options such as width/precision.

The following are not committed to as first-version promises:

- Auto-discovering the Debug / Display implementations of arbitrary types, or generating missing generic machine code during debugging.
- Constructing a Rust Formatter directly in Python or calling an unstable trait ABI.
- Automatically deciding whether an arbitrary user fmt is pure, lock-free, or allocation-free.
- Guaranteeing recovery at an invalid address, mid-update object state, corrupted process, or after panic=abort.
- Assuming that copying an object's raw bytes into a temporary space makes it safe to call that object's methods.
- Treating a timeout, interrupt, or register restoration as undoing all side effects of a target call.

The first version's focus is turning the currently validated invocation approach into a selectable, registrable, diagnosable component; the output text and structured expansion evolve separately.

## 10. Design basis

- [Rust debugger_visualizer attribute](https://doc.rust-lang.org/reference/attributes/debugger.html): embedded scripts, Natvis, and GDB loading mechanisms.
- [Rust Display](https://doc.rust-lang.org/std/fmt/trait.Display.html) and [Formatter](https://doc.rust-lang.org/std/fmt/struct.Formatter.html): the interface boundary for reusing formatting implementations.
- [Rust used attribute](https://doc.rust-lang.org/reference/abi.html#the-used-attribute): distinguishing compilation-artifact retention from final-link retention.
- [GDB calling target program functions](https://sourceware.org/gdb/current/onlinedocs/gdb.html/Calling.html): calling, interruption, timeout, and target-state effects.
- [LLDB visualization extensions](https://lldb.llvm.org/use/variable.html): summary and synthetic children integration.

When implementing each stage, validate these interfaces against the actually adopted toolchain version; the default values, protocol names, and directory structure in this document are all proposed plans.
