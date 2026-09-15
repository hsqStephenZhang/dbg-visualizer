# Phase-two Q0 technical validation record

Date: 2026-09-10. Status: **historical Q0 evidence; the toolchain decision is resolved.** The user subsequently approved nightly specialization, GDB first, and not keeping v1; the current core and real-backend results are in the [Phase-two acceptance record](../phase-two-acceptance.md). What follows records the independent experiments conducted before the decision, and does not pass them off as the whole phase-two acceptance.

## 1. How it was actually run

```sh
# Only reproduces the limitation on stable; does not run a debugger.
python3 docs/archive/q0/run.py

# Standalone nightly experiment and real GDB/LLDB subprocesses; requires local ptrace permission.
python3 docs/archive/q0/run.py --nightly --debuggers
```

The second command was actually run, exit code 0. The runner checks each expected success/failure, the specific error code, and the output markers; all build artifacts live in isolated temporary directories. The default toolchain, workspace dependencies, and the current v1 runtime were not modified.

The tested toolchains were stable rustc 1.97.1 and nightly rustc 1.99.0-nightly (12c36e253 2026-08-10). These are the versions actually installed locally, and do not imply that any other version or remote CI has been validated.

## 2. The stable prototype did not satisfy automatic priority for generics

Source files: [autoref.rs](q0/autoref.rs), [overlap.rs](q0/overlap.rs).

The same `Both` type has both `Visualize` and `Debug`. Using autoref to resolve method candidates, `Visualize` is selected at a concrete call site; inside `fn generic_debug<T: Debug>` `Debug` is selected, and even when a `Both` is actually passed in, it is not re-selected as `Visualize` at monomorphization:

```text
CONCRETE=visualize:Both
GENERIC=debug:Both
AUTOREF_GENERIC_PRIORITY_LOST
```

An unconstrained `fn generic_unbounded<T>` reports E0599 and cannot call any candidate. Implementing the same blanket trait separately for `T: Debug` and `T: Display` reports E0119 (overlapping implementations). These results confirm that these two common tricks cannot satisfy the general generic contract in the plan; we do not repackage them as a "finished implementation" that only supports concrete fields.

This is not a formal proof of impossibility for every possible Rust program, but for now no stable-version implementation path has been found that satisfies the full contract. A proc-macro's token input and TypeId do not automatically supply this kind of generic capability selection. [Rust method resolution](https://doc.rust-lang.org/reference/expressions/method-call-expr.html), [trait coherence rules](https://doc.rust-lang.org/reference/items/implementations.html#trait-implementation-coherence)

## 3. Standalone nightly dispatch experiment

Source files: [dispatch.rs](q0/dispatch.rs), [consumer.rs](q0/consumer.rs). The former is compiled separately into an rlib and the latter is brought in as another crate, so this is not a fake generic test that places all concrete types at the same macro call site.

Using `#![feature(specialization)]`, selection follows Visualize → Debug → Display via three independent specialization layers, without unsafe type transmutation, TypeId hashing, private layout, or `'static` bounds. The experiment only validates call selection, collecting assertion text with a plain String; it is not yet hooked up to the bounded formatter, proc-macro, v2 mailbox, or debugger frontend.

Both actual builds, debug and opt-level=3/fat LTO, pass:

- Correct selection for Visualize-only, Debug-only, Display-only, Debug+Display, and all three present.
- When all three are present, the lower-priority implementations contain a panic and are in fact never called.
- Vec of single-capability elements and their priority; HashMap of nested Vecs; HashMap with a unit key, a Display-only value, and a BuildHasherDefault policy.
- `Wrapper<'a, T, Policy, const N: usize>` defined in another crate; a str borrowing a local String, a PhantomData Policy that implements no formatting trait, and const parameter 13.
- Building `Vec<NoTraits>` reports E0080, with a message that explicitly states there is no Visualize/Debug/Display, rather than returning the missing capability at runtime.

### Important limitation

Missing-capability detection relies on an associated constant evaluated at code generation. An actual `rustc` build can reject invalid concrete instances, but a `--emit=metadata` check passes, so we cannot claim that `cargo check` provides the same diagnostic. The runner explicitly prints `NIGHTLY_METADATA_ONLY_DOES_NOT_REJECT_*`, preserving this difference.

This is only an isolated experimental candidate backend. specialization is still a nightly feature, and compiling this experiment on stable reports E0554. It cannot be set as a project default requirement without confirmation; the full derive, recursive cycle detection, field attributes, feature-off, and dispatch compatibility have not yet been validated. [Rust specialization](https://doc.rust-lang.org/unstable-book/language-features/specialization.html)

## 4. Preliminary typed anchor validation

Source files: [anchor.rs](q0/anchor.rs), [GDB validation script](q0/anchor_gdb.py), [LLDB validation script](q0/anchor_lldb.py).

An immutable static anchor with a slot and a `*const T` field references a concrete type; the typed field is a null pointer and is never dereferenced. The anchor is retained via a reachable black_box reference. GDB/LLDB read its type information and then compare it against the real debugger type of the variable at the breakpoint, without calling the target formatting function to make the discovery.

Both test sets on the stable build, debug and release-equivalent opt-level=3/fat LTO, pass, with types including:

- HashMap<String, Vec<u64>> with default generic parameters.
- A HashMap with a unit key and a ZST BuildHasherDefault.
- Borrowed<'a, Zst, 17> actually borrowing a local String.
- The ZST itself.

GDB must try both global and static symbol lookup: a Rust static variable cannot be found with `lookup_global_symbol` alone, but can be found with `lookup_static_symbol`. LLDB can obtain the corresponding anchor via `FindFirstGlobalVariable`.

### Follow-up validation for same-layout const instances and same-named types

The original fixture was extended with `Borrowed<'a, Zst, 19>`, retained at the same time as the const-parameter-17 instance; also added were `left::State(u64)` / `right::State(u64)` with identical field layouts. `python3 docs/archive/q0/run.py --debuggers` was actually rerun, and the debug and fat LTO results agree:

- GDB's type-identity comparison can distinguish const 17/19, and can also distinguish the two State types in different modules.
- LLDB can distinguish the State types in different modules, but cannot fully resolve the second Borrowed const instance in this example. Its anchor pointee shows as `void` and the variable's canonical type name is empty; a trustworthy type cannot be constructed from this.
- The validation script preserves and explicitly reports this failure boundary, excluding that entry from the usable type set, rather than "passing" by deleting the test, matching by size, or forcing a conversion. The current output includes `LLDB_CONST_GENERIC_UNAVAILABLE_MUST_REJECT BORROWED_OTHER 'void' ''`.

`LLDB_TYPED_ANCHOR_OK` only means the still-associable types pass the check, and **does not mean all const generics are supported**; the runner also prints the rejected entries. This is not yet a fallback acceptance of the full frontend. Follow-up work should study a unique non-generic anchor or other debug-info association schemes; we must not fall back to matching only by identical name and size, and even more so must not call a function of the wrong type when information is incomplete.

So far this does not cover same-name different-crate/version, all forms of lifetime erasure, values optimized away, CodeLLDB DAP, and different target platforms. Q0 type association is still not fully accepted.

## 5. Decision record (resolved)

The original plan required a stable default and automatic cross-crate generic selection. The stable prototype did not satisfy the latter requirement; the nightly experiment provides feasibility evidence, but required the user to approve the toolchain change.

The user has explicitly approved adopting nightly specialization, and explicitly not keeping v1. nightly is now the only backend when dbgvis is enabled, and GDB is this round's frontend; we no longer wait for the stable path or for LLDB const type association to pass. derive, the bounded formatter, v2 registration, general HashMap, third-party types, and the generic matrix have entered core acceptance.

If a stable requirement is reintroduced in the future, it will need independent research or a contract adjustment; this round was not reduced to "all fields must be Debug" or "third-party fields must go via".
