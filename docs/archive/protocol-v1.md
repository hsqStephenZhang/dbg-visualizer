# Debugger visualization protocol v1

> Historical archive: the v1 implementation has been deleted at the user's request; the source paths and commands below correspond only to historical Git versions. The current protocol is [protocol v2](../protocol-v2.md).

Implementation locations: `crates/visualizer-runtime/src/protocol.rs`, `lib.rs`, and `debugger/common/core.py`.

v1 supports 64-bit little-endian Linux local targets. All cross-boundary structures use `#[repr(C)]`; Python validates the ABI, structure sizes, pointer width, and byte order, and does not rely on the host machine's default layout. Future extensions require a new version or an explicit compatibility strategy.

## Discovery and invocation

The sole execution entry point: `unsafe extern "C" fn dbgvis_dispatch_v1(request_addr: usize) -> u32`.

It exports a read-only `DBG_VIS_MODULE_V1`, 112 bytes in total:

| Offset | Field | Encoding |
| --- | --- | --- |
| 0 | magic | 8 bytes `DBGVIS01` |
| 8 | abi, size, pointer_width, little_endian | 4 u32 values: 1, 112, 8, 1 |
| 24 | entry_size, request_size, response_size, child_size | 4 u32 values: 112, 88, 48, 304 |
| 40 | entries, entry_count | 2 target machine words |
| 56 | request, response | 2 target pointers |
| 72 | output, output_capacity | output address and capacity |
| 88 | children, children_capacity | descriptor region address and count limit of 128 |
| 104 | dispatcher | C ABI function address |

GDB can obtain the descriptor address via a minimal symbol; LLDB obtains it from a data symbol in the main module. Reading the metadata does not execute any target function. The first version only accepts the root registry in the executable and does not handle registries from multiple dynamic libraries.

Requests can only be written to the published request mailbox; the dispatcher rejects any other request address. The debugger must submit serially and write the mailbox while the target is stopped, and must not overwrite a call that is already executing. Rust protects execution with a non-waiting atomic flag, and nested calls return BUSY.

## Registration entries

Each entry is 112 bytes:

- Three `(pointer, length)` pairs of static UTF-8 text: canonical name, GDB alias, LLDB alias, for 48 bytes total.
- `size`, `align`, `capabilities`, `shape`, 8 bytes each, for 32 bytes total.
- Four optional module-internal function pointers: Debug, Display, count, page, for 32 bytes total.

Capability bits: Debug=1, Display=2, Children=4. shape: struct=0, sequence=1, map=2. The concrete callbacks are only invoked by the dispatcher within the same Rust module and do not constitute a stable Rust plugin ABI. The type ID is an array index, valid only for the current registry; it does not use TypeId or an unvalidated type-name hash.

The descriptor region and entry structure sizes form the v1 adapter version constraint; changing child-item semantics requires updating the ABI. There is no separate layout version for third-party private containers, because the built-in adapters call the containers' public interfaces.

## Requests and responses

A Request is 11 u64 values, 88 bytes, in the following order:

```text
abi, size, sequence, operation, type_id, object,
mode, alternate, capacity, start, count
```

operation: FORMAT=1, CHILD_COUNT=2, CHILD_PAGE=3. mode: Debug=1, Display=2; 0 may only be used for element queries that do not format. alternate is 0/1.

`capacity` constrains the summary byte count; FORMAT requires 1 up to the output region limit; `start/count` are the logical element offset and count; CHILD_PAGE requires count to be 1–64 and rejects out-of-bounds accesses and additive overflow. The last page may contain fewer than count. An empty container queried from start=0 may return zero elements.

A Response is 6 u64 values, 48 bytes:

```text
sequence, status, written, total, returned, shape
```

`written` is the number of valid UTF-8 prefix bytes; a NUL terminator is not required, and embedded NULs are allowed. `total` is the total number of logical elements in the container; `returned` is the number of descriptors on this page, which for a map equals twice the number of logical elements on this page. Each operation resets the counting semantics of the response; a failed response must not be used as the previous round's result.

The caller verifies that the returned integer status matches response.status and that sequence corresponds to this request, then copies the result into debugger memory according to the length. No subsequent call may be issued before the copy completes. BUSY does not overwrite the response of the outer call, so this return code must be handled first.

## Child element descriptors

Each Child is 304 bytes:

| Field | Size | Semantics |
| --- | --- | --- |
| name | 64 bytes | UTF-8, zero-terminated; may be left empty for sequences, with the front end filling in the index |
| type_name | 192 bytes | UTF-8, zero-terminated Rust type name |
| address | u64 | address of the actual target object or field slot |
| size, align | u64 each | actual type size and alignment |
| kind | u64 | 0 = typed address; 1 = inline unsigned scalar; 2 = &str slot with auxiliary string information |
| data, length | u64 each | scalar data, or, for kind=2, the string data address and byte length |

For kind=2, `address` points to the actual `&str` field slot and must not be replaced by data when passed to a formatting function targeting `&str`. Auxiliary string reads are limited to 4096 bytes; when a trustworthy debug type is available, prefer constructing a typed child node.

Map child items are laid out alternating key, value, named `key` and `value` respectively. All text fields and page storage have fixed limits; truncating type names is not allowed, and leaking the address of a temporary local value is not allowed. The caller must discard the descriptors after resuming execution or when the object layout changes.

## Status codes

| Value | Name | Meaning |
| --- | --- | --- |
| 0 | OK | Success |
| 1 | TRUNCATED | Insufficient output; a valid UTF-8 prefix has been retained |
| 2 | UNSUPPORTED | The type has no corresponding capability, or the descriptor cannot be represented |
| 3 | INVALID_REQUEST | Invalid parameters, object base-address check, or paging result |
| 4 | ABI_MISMATCH | Incompatible request version or structure size |
| 5 | BUSY | A call is already executing; its response is not overwritten |
| 6 | FORMAT_ERROR | No truncation, but fmt returned an error |
| 7 | PANIC | A recoverable panic was caught |

Timeouts, missing addresses, core dumps, memory read failures, and process exits are debugger backend errors and are not disguised as Rust status codes. Checking alignment and type names cannot prove that an address is valid, nor that the object's invariants hold while it is paused.
