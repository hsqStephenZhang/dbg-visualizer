"""LLDB v2 bridge for dbgvis. Same mailbox ABI as gdb.py; loaded manually.

Unlike GDB, LLDB has no `.debug_gdb_scripts` auto-load, so this script is not
embedded via `#[debugger_visualizer]`. It is generated into the target directory
and loaded by hand:

    (lldb) command script import /path/to/target/dbgvis/lldb.py
    (lldb) dbgvis print app

It drives the process through the exact same exported mailbox the GDB bridge
uses (`DBG_VIS_MODULE_V2` + a dispatcher function pointer + request/response/output
buffers), so the Rust runtime needs no LLDB-specific support.
"""
import os
import re
import shlex
import struct
import subprocess
import sys
import tempfile

import lldb

MODES = {"auto": 0, "visualize": 2, "debug": 4, "display": 8}
DEFAULTS = {
    "buffer_bytes": 4096,
    "alternate": False,
    "max_depth": 32,
    "max_nodes": 4096,
    "timeout_ms": 200,
}
STATUS = {
    2: "UNSUPPORTED", 3: "INVALID", 4: "ABI_MISMATCH",
    5: "BUSY", 6: "FORMAT_ERROR", 7: "PANIC", 8: "NOT_READY",
}
TRUNCATION = {1: "byte limit", 2: "depth limit", 3: "node limit", 4: "cycle"}


class Error(Exception):
    pass


def _read(process, address, length):
    err = lldb.SBError()
    data = process.ReadMemory(address, length, err)
    if not err.Success():
        raise Error(f"read {length} bytes at 0x{address:x}: {err.GetCString()}")
    return data


def _text(process, pointer, length):
    if length > 8192:
        raise Error("metadata text exceeds limit")
    return _read(process, pointer, length).decode("utf-8")


def _module_address(target, frame):
    """Load address of the exported registry anchor in the main executable."""
    var = target.FindFirstGlobalVariable("DBG_VIS_MODULE_V2")
    if var and var.IsValid():
        addr = var.GetLoadAddress()
        if addr != lldb.LLDB_INVALID_ADDRESS:
            return addr
    value = frame.EvaluateExpression("(unsigned long long)&DBG_VIS_MODULE_V2")
    if value.GetError().Success():
        return value.GetValueAsUnsigned()
    raise Error("DBG_VIS_MODULE_V2 not found; is dbgvis linked into this target?")


# Matching reconciles the registry `name` (Rust's own type spelling) with LLDB's
# DWARF type names, which diverge in several mechanical ways AND drop information
# Rust keeps. `_canonical` reduces BOTH to the common form LLDB can actually see;
# `_match` then treats two registry entries that collapse together as ambiguous
# and refuses, rather than guessing -- picking the wrong slot would reinterpret the
# value's bytes. LLDB exposes no type for an anchor symbol, so the GDB bridge's
# DWARF-shape comparison is unavailable and this name reconciliation stands in.

# 1. Defaulted generic parameters Rust elides but LLDB spells out in full. A
#    non-default allocator or hasher is never one of these exact literals.
_ELIDED_DEFAULTS = (
    ", alloc::alloc::Global",
    ", hashbrown::alloc::inner::Global",
    ", std::hash::random::RandomState",
)
# 2. Primitive integers LLDB renders with C spellings. Longest first so
#    "unsigned long long" is consumed before "long". `usize`/`u64` share a C
#    spelling; the size check in `_match` disambiguates.
_PRIMITIVES = [
    ("unsigned long long", "u64"), ("long long", "i64"),
    ("unsigned long", "u64"), ("long", "i64"),
    ("unsigned int", "u32"), ("int", "i32"),
    ("unsigned short", "u16"), ("short", "i16"),
    ("unsigned char", "u8"), ("signed char", "i8"),
    ("void", "()"),  # LLDB spells the Rust unit type `()` as `void`
]


def _canonical(name):
    """Reduce a Rust or LLDB type name to the common form LLDB can observe.

    LLDB drops lifetimes and const-generic arguments and adds defaulted params and
    C integer spellings; none of that is recoverable from DWARF. Stripping it from
    both sides makes comparable names compare equal -- and makes types that differ
    only in what LLDB drops (e.g. `Foo<_, 17>` vs `Foo<_, 19>`) collapse together,
    which `_match` then reports as ambiguous instead of matching the wrong one.
    """
    if not name:
        return name
    for default in _ELIDED_DEFAULTS:
        name = name.replace(default, "")
    for c_name, rust in _PRIMITIVES:
        name = re.sub(r"\b" + re.escape(c_name) + r"\b", rust, name)
    name = re.sub(r"([^\s,<>]+(?:<[^\[\]]*>)?) ?\[(\d+)\]", r"[\1; \2]", name)  # T[N]/T [N] -> [T; N]
    name = re.sub(r"'\w+", "", name)                       # drop lifetimes
    name = re.sub(r"(?<=[<,])\s*\d+\s*(?=[,>])", "", name)  # drop const-generic ints
    name = re.sub(r"\s+", " ", name)
    for _ in range(3):  # tidy the empty argument slots the removals leave behind
        name = re.sub(r"<\s*,\s*", "<", name)
        name = re.sub(r"\s*,\s*>", ">", name)
        name = re.sub(r",\s*,", ",", name)
    name = re.sub(r"\s*<\s*", "<", name)
    name = re.sub(r"\s*>", ">", name)
    name = re.sub(r"\s*,\s*", ", ", name)
    return name.strip()


def discover(target, process, frame):
    """Parse the mailbox header and the registered entry table."""
    address = _module_address(target, frame)
    header = struct.unpack("<8s15Q", _read(process, address, 128))
    if header[:8] != (b"DBGVIS02", 2, 128, 8, 1, 96, 80, 40):
        raise Error("ABI_MISMATCH: requires dbgvis v2, 64-bit little-endian")
    if header[8] != 1:
        raise Error("registry not enabled yet; stop after dbgvis::enable!")
    count, base = header[10], header[9]
    if not 0 <= count <= 4096 or not 0 < header[14] <= 16 * 1024 * 1024:
        raise Error("invalid registry bounds")
    entries = []
    for slot in range(count):
        row = struct.unpack("<12Q", _read(process, base + slot * 96, 96))
        entries.append(dict(
            slot=slot,
            name=_text(process, row[0], row[1]),
            anchor=_text(process, row[2], row[3]),
            size=row[4], align=row[5], capabilities=row[6], default=row[7],
        ))
    module = dict(request=header[11], response=header[12], output=header[13],
                  capacity=header[14], dispatcher=header[15])
    return module, entries


def _candidates(value):
    """(type name, object address) pairs to try against the registry.

    A registered root is a concrete type; a `&T`/`*T` local names the pointee,
    so also offer the dereferenced type at the address the pointer holds.
    """
    out = []
    typ = value.GetType()
    addr = value.GetLoadAddress()
    if addr != lldb.LLDB_INVALID_ADDRESS:
        out.append((typ.GetName(), addr))
        out.append((typ.GetDisplayTypeName(), addr))
    if typ.IsReferenceType() or typ.IsPointerType():
        pointee = value.Dereference()
        target = value.GetValueAsUnsigned()
        if pointee.IsValid() and target:
            out.append((pointee.GetType().GetName(), target))
            out.append((pointee.GetType().GetDisplayTypeName(), target))
    return out


def _match(entries, value):
    names = {}
    for name, addr in _candidates(value):
        key = _canonical(name)
        if key:
            names.setdefault(key, addr)
    size = value.GetType().GetByteSize()
    hits = [(entry, names[_canonical(entry["name"])])
            for entry in entries
            if _canonical(entry["name"]) in names and (not size or entry["size"] == size)]
    if len(hits) == 1:
        return hits[0]
    if len(hits) > 1:
        raise Error("ambiguous type; indistinguishable to LLDB (lifetimes and const "
                    "generics are dropped from DWARF):\n  "
                    + "\n  ".join(entry["name"] for entry, _ in hits))
    raise Error("unregistered type: " + (value.GetType().GetName() or "<unknown>")
                + "\n  registered: " + ", ".join(sorted(e["name"] for e in entries)))


def render(target, process, frame, value, overrides, discovered=None):
    module, entries = discovered if discovered else discover(target, process, frame)
    entry, address = _match(entries, value)
    options = dict(DEFAULTS)
    options.update(overrides)
    mode = MODES[options["mode"]] if options.get("mode") else 0
    if mode and not entry["capabilities"] & mode:
        raise Error("UNSUPPORTED: mode not registered for " + entry["name"])
    if address % entry["align"]:
        raise Error("value has no aligned addressable storage")
    if not 0 < options["buffer_bytes"] <= module["capacity"]:
        raise Error("buffer exceeds compiled capacity")

    sequence = (process.GetUniqueID() << 16) ^ (address & 0xFFFF) ^ entry["slot"] ^ 0x9E37
    sequence &= 0xFFFFFFFFFFFFFFFF
    request = struct.pack("<10Q", 2, 80, sequence, entry["slot"], address, mode,
                          options["buffer_bytes"], int(options["alternate"]),
                          options["max_depth"], options["max_nodes"])
    err = lldb.SBError()
    process.WriteMemory(module["request"], request, err)
    if not err.Success():
        raise Error("write request: " + err.GetCString())

    call = (f"((unsigned long long (*)(unsigned long long))0x{module['dispatcher']:x})"
            f"(0x{module['request']:x})")
    opts = lldb.SBExpressionOptions()
    opts.SetIgnoreBreakpoints(True)
    opts.SetUnwindOnError(True)
    opts.SetTryAllThreads(False)
    opts.SetFetchDynamicValue(lldb.eNoDynamicValues)
    opts.SetTimeoutInMicroSeconds(max(1, options["timeout_ms"]) * 1000)
    result = frame.EvaluateExpression(call, opts)
    if not result.GetError().Success():
        raise Error("target call failed: " + result.GetError().GetCString())
    status = result.GetValueAsUnsigned()

    seq, returned, length, outcome, _nodes = struct.unpack(
        "<5Q", _read(process, module["response"], 40))
    if seq != sequence or returned != status or length > options["buffer_bytes"]:
        raise Error("invalid or stale response")
    if status not in (0, 1):
        raise Error(STATUS.get(status, str(status)))
    text = _read(process, module["output"], length).decode("utf-8")
    if status == 1:
        text += " <dbgvis: " + TRUNCATION.get(outcome, "limited") + ">"
    return text


def _parse(command):
    """Split `[--all] [--no-pager] [-m MODE] [-b BYTES] [-a] [--] EXPR`.

    `--all` renders every registered local in the frame instead of one EXPR.
    Returns (render overrides, control flags, EXPR-or-None).
    """
    overrides = {}
    control = {"all": False, "pager": True}
    tokens = shlex.split(command)
    i = 0
    while i < len(tokens):
        token = tokens[i]
        if token == "--":
            i += 1
            break
        if token == "--all":
            control["all"] = True
            i += 1
        elif token == "--no-pager":
            control["pager"] = False
            i += 1
        elif token in ("-m", "--mode"):
            value = tokens[i + 1]
            if value not in {"native", *MODES}:
                raise Error("invalid mode: " + value)
            overrides["mode"] = None if value == "native" else value
            i += 2
        elif token in ("-b", "--buffer"):
            overrides["buffer_bytes"] = int(tokens[i + 1])
            i += 2
        elif token in ("-a", "--alternate"):
            overrides["alternate"] = True
            i += 1
        elif token.startswith("-"):
            raise Error("unknown print option: " + token)
        else:
            break
    expr = " ".join(tokens[i:]).strip()
    if not control["all"] and not expr:
        raise Error("missing variable expression (or pass --all)")
    return overrides, control, (expr or None)


def _evaluate(frame, expr):
    """Resolve EXPR to a value. LLDB's Rust expression evaluator is unreliable (it
    falls back to C/ObjC++ and fails on plain locals), so read DWARF locals and
    `a.b`/`a[0]` paths directly first, and only fall back to a full evaluation."""
    value = frame.GetValueForVariablePath(expr)
    if not value or not value.IsValid():
        value = frame.EvaluateExpression(expr)
    if not value or not value.IsValid() or not value.GetError().Success():
        detail = value.GetError().GetCString() if value else "not found"
        raise Error("cannot evaluate `" + expr + "`: " + (detail or "not found"))
    return value


def render_all(target, process, frame, overrides):
    """Render every in-scope local whose type is registered, one per entry.

    Discovery runs once and is shared; a local whose type is not registered (or
    is ambiguous to LLDB) is listed at the end rather than raising, so one bad
    local never hides the rest."""
    discovered = discover(target, process, frame)
    variables = frame.GetVariables(True, True, False, True)  # args + locals, in scope
    shown, skipped = [], []
    for value in variables:
        name = value.GetName() or "<anon>"
        try:
            shown.append(f"{name} = {render(target, process, frame, value, overrides, discovered)}")
        except Error as error:
            skipped.append(f"{name}: {str(error).splitlines()[0]}")
    lines = shown or ["no registered locals in this frame"]
    if skipped:
        lines.append("")
        lines.append(f"skipped {len(skipped)}:")
        lines.extend("  " + s for s in skipped)
    return "\n".join(lines)


def _emit(result, text, pager):
    """Print `text`, paging it through $PAGER when asked and a terminal is present.
    Falls back to the debugger's own output when there is no TTY or the pager fails."""
    if pager and sys.stdin.isatty() and sys.stdout.isatty():
        command = os.environ.get("PAGER") or "less -R -F -X"
        path = None
        try:
            with tempfile.NamedTemporaryFile("w", suffix=".dbgvis", delete=False) as handle:
                handle.write(text + "\n")
                path = handle.name
            subprocess.call(shlex.split(command) + [path])
            return
        except Exception:
            pass  # fall through to inline output
        finally:
            if path:
                try:
                    os.unlink(path)
                except OSError:
                    pass
    result.AppendMessage(text)


def dbgvis(debugger, command, exe_ctx, result, internal_dict):
    try:
        overrides, control, expr = _parse(command)
        frame = exe_ctx.GetFrame()
        if not frame or not frame.IsValid():
            raise Error("no stopped frame; run to a breakpoint first")
        target, process = exe_ctx.GetTarget(), exe_ctx.GetProcess()
        if control["all"]:
            text = render_all(target, process, frame, overrides)
            _emit(result, text, control["pager"])
        else:
            value = _evaluate(frame, expr)
            result.AppendMessage(render(target, process, frame, value, overrides))
    except Error as error:
        result.SetError(str(error))


def dbgvis_types(debugger, command, exe_ctx, result, internal_dict):
    try:
        _module, entries = discover(exe_ctx.GetTarget(), exe_ctx.GetProcess(), exe_ctx.GetFrame())
        if not entries:
            result.AppendMessage("no registered types")
            return
        caps = {2: "visualize", 4: "debug", 8: "display"}
        for entry in entries:
            modes = " ".join(name for bit, name in caps.items() if entry["capabilities"] & bit)
            result.AppendMessage(f"[{entry['slot']}] {entry['name']}  ({modes or 'none'})")
    except Error as error:
        result.SetError(str(error))


def __lldb_init_module(debugger, internal_dict):
    module = __name__
    debugger.HandleCommand(f"command script add -o -f {module}.dbgvis dbgvis-print")
    debugger.HandleCommand(f"command script add -o -f {module}.dbgvis_types dbgvis-types")
    # Convenience: `dbgvis` regex command dispatching print/p, all/a, types/t.
    debugger.HandleCommand(
        "command regex dbgvis "
        "'s/^(all|a)$/dbgvis-print --all/' "
        "'s/^(all|a) +(.+)$/dbgvis-print --all %2/' "
        "'s/^(print|p) +(.+)$/dbgvis-print %2/' "
        "'s/^(types|t)$/dbgvis-types/'"
    )
    print("dbgvis: LLDB bridge loaded (dbgvis print EXPR | dbgvis all | dbgvis types)")
