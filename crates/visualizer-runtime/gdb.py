"""GDB-only v2 bridge. Packaged with dbgvis; no source-tree imports or target layouts."""
import gdb
import json
import math
import os
import shlex
import struct
import time

MODES = {"auto": 0, "visualize": 2, "debug": 4, "display": 8}
DEFAULTS = {"summary.mode": "native", "execution": "manual", "summary.buffer_bytes": 4096,
            "summary.alternate": False, "format.max_depth": 32, "format.max_nodes": 4096,
            "execution.timeout_ms": 200}


class NoMatch(Exception):
    pass


def type_shape(typ, seen=None, depth=0):
    """Compare complete DWARF type shape when GDB's Rust enum equality fails across CUs.

    Names alone are never enough. No value bytes or crate-private layout assumptions.
    Cross-build cache reuse is prohibited; ambiguous registered candidates are rejected.
    """
    typ = typ.strip_typedefs()
    if depth > 48:
        raise NoMatch("type metadata exceeds comparison depth")
    alignment = getattr(typ, "alignof", None)
    size = typ.sizeof
    if typ.code in (gdb.TYPE_CODE_STRUCT, gdb.TYPE_CODE_UNION) and alignment:
        # GDB's resolved Rust dynamic types can omit trailing alignment padding.
        # Compare the allocated extent, while retaining every field offset/type below.
        size = (size + alignment - 1) // alignment * alignment
    key = (str(typ), typ.code, size, alignment)
    seen = set() if seen is None else set(seen)
    if key in seen:
        return ("recursive", key)
    seen.add(key)
    if typ.code in (gdb.TYPE_CODE_PTR, gdb.TYPE_CODE_REF, gdb.TYPE_CODE_ARRAY):
        bounds = tuple(typ.range()) if typ.code == gdb.TYPE_CODE_ARRAY else ()
        return (key, bounds, type_shape(typ.target(), seen, depth + 1))
    if typ.code in (gdb.TYPE_CODE_STRUCT, gdb.TYPE_CODE_UNION, gdb.TYPE_CODE_ENUM):
        fields = []
        for field in typ.fields():
            fields.append((field.name, getattr(field, "bitpos", None), getattr(field, "bitsize", None),
                           getattr(field, "enumval", None),
                           type_shape(field.type, seen, depth + 1) if field.type else None))
        return (key, tuple(fields))
    return (key,)


def compatible_shape(declared, actual):
    if declared == actual:
        return True
    if not isinstance(declared, tuple) or not isinstance(actual, tuple) or not declared or not actual:
        return False
    if declared[0] != actual[0] or len(declared) != len(actual):
        return False
    if len(declared) == 3:  # pointer/reference/array
        return declared[1] == actual[1] and compatible_shape(declared[2], actual[2])
    if len(declared) != 2 or declared[0] == "recursive":
        return False
    expected, found = declared[1], actual[1]
    if len(expected) != len(found):
        # GDB materializes Rust enums as a type containing only the active variant.
        # Accept that *checked subset*, not arbitrary incomplete struct metadata.
        name = declared[0][0]
        if not expected or not found or expected[0][0] is not None or found[0][0] is not None:
            return False
        if not all(f[0] and f[4][0][0] == name + "::" + f[0] for f in expected[1:]):
            return False
        if len(found) != 2:
            return False
        expected = tuple(f for f in expected if f[0] in {x[0] for x in found})
    return len(expected) == len(found) and all(
        e[:4] == a[:4] and compatible_shape(e[4], a[4]) for e, a in zip(expected, found)
    )


class Session:
    def __init__(self):
        self.config = dict(DEFAULTS)
        self.overrides = {}
        self.process = None
        self.module = None
        self.entries = []
        self.active = False
        self.native_depth = 0
        self.poisoned = False
        self.sequence = 0
        self.calls = 0
        self.last_error = ""
        self.elapsed_ms = 0
        self.printer = self.lookup
        gdb.pretty_printers.insert(0, self.printer)
        self.events = []
        for name in ("new_objfile", "clear_objfiles", "exited", "cont"):
            event = getattr(gdb.events, name, None)
            if event:
                event.connect(self.changed)
                self.events.append(event)

    def dispose(self):
        if self.printer in gdb.pretty_printers:
            gdb.pretty_printers.remove(self.printer)
        for event in self.events:
            event.disconnect(self.changed)
        self.events.clear()

    def changed(self, event):
        self.module = None
        self.entries = []
        # An interrupted call may hold the Rust busy guard: never clear poison here.

    def read(self, address, length):
        return bytes(gdb.selected_inferior().read_memory(address, length))

    def text(self, address, length):
        if length > 8192:
            raise RuntimeError("metadata text exceeds limit")
        return self.read(address, length).decode("utf-8")

    def discover(self):
        inferior = gdb.selected_inferior()
        key = (inferior.num, inferior.pid, gdb.current_progspace().filename)
        if key != self.process:
            self.process = key
            self.module = None
            self.entries = []
            self.poisoned = False
        if self.module is not None:
            return
        with gdb.with_parameter("language", "c"):
            address = int(gdb.parse_and_eval("(unsigned long)&DBG_VIS_MODULE_V2"))
        if gdb.solib_name(address) is not None:
            raise RuntimeError("registry must belong to main executable")
        h = struct.unpack("<8s15Q", self.read(address, 128))
        if h[:8] != (b"DBGVIS02", 2, 128, 8, 1, 96, 80, 40):
            raise RuntimeError("ABI_MISMATCH: requires dbgvis v2, 64-bit little-endian")
        if h[8] != 1:
            raise RuntimeError("registry not enabled yet; stop after dbgvis::enable!")
        if not 0 <= h[10] <= 4096 or not 0 < h[14] <= 16 * 1024 * 1024:
            raise RuntimeError("invalid registry bounds")
        entries = []
        for slot in range(h[10]):
            row = struct.unpack("<12Q", self.read(h[9] + slot * 96, 96))
            name, anchor = self.text(row[0], row[1]), self.text(row[2], row[3])
            symbol = gdb.lookup_global_symbol(anchor) or gdb.lookup_static_symbol(anchor)
            typ = None
            if symbol is not None:
                fields = [f for f in symbol.type.strip_typedefs().fields() if f.name == "typed"]
                if len(fields) == 1:
                    candidate = fields[0].type.target().strip_typedefs()
                    objfile = candidate.objfile
                    owner = (getattr(objfile, "owner", None) or objfile) if objfile else None
                    if (owner and os.path.realpath(owner.filename) == os.path.realpath(key[2])
                            and candidate.sizeof == row[4]
                            and getattr(candidate, "alignof", row[5]) == row[5]):
                        typ = candidate
            entries.append(dict(slot=slot, name=name, anchor=anchor, type=typ,
                                size=row[4], align=row[5], capabilities=row[6], default=row[7]))
        self.entries = entries
        self.module = dict(request=h[11], response=h[12], output=h[13], capacity=h[14], dispatcher=h[15])

    def match(self, value):
        self.discover()
        typ = value.type.strip_typedefs()
        objfile = typ.objfile
        owner = (getattr(objfile, "owner", None) or objfile) if objfile else None
        if not owner or os.path.realpath(owner.filename) != os.path.realpath(self.process[2]):
            raise NoMatch("type does not belong to main executable")
        candidates = [e for e in self.entries if e["type"] is not None and str(e["type"]) == str(typ)]
        matches = [e for e in candidates if e["type"] == typ or compatible_shape(type_shape(e["type"]), type_shape(typ))]
        if len(matches) != 1:
            raise NoMatch("unregistered or ambiguous type: " + str(typ))
        return matches[0]

    def options(self, entry, overrides=None):
        result = dict(self.config)
        result.update(self.overrides.get(entry["name"], {}))
        result.update(overrides or {})
        return result

    def native(self, value):
        self.native_depth += 1
        try:
            return value.format_string()
        finally:
            self.native_depth -= 1

    def timeout_supported(self):
        try:
            report = gdb.execute("show direct-call-timeout", to_string=True)
            asynchronous = gdb.execute("maintenance show target-async", to_string=True)
            return asynchronous.strip().endswith(" on.") and "does not support" not in report
        except gdb.error:
            return False

    def ensure_callable(self, options):
        inferior = gdb.selected_inferior()
        if inferior.pid == 0 or not inferior.threads() or inferior.connection is None or inferior.connection.type == "core":
            raise RuntimeError("target execution unavailable: core dump or no live process")
        if any(thread.is_running() for thread in inferior.threads()):
            raise RuntimeError("all target threads must be stopped")
        if not gdb.parameter("may-call-functions"):
            raise RuntimeError("GDB may-call-functions is off")
        if options["execution"] == "automatic" and not self.timeout_supported():
            raise RuntimeError("automatic formatting requires GDB async call timeout support")

    def summary(self, value, overrides=None):
        entry = self.match(value)
        options = self.options(entry, overrides)
        mode = options["summary.mode"]
        if mode == "native":
            return self.native(value)
        if self.poisoned:
            raise RuntimeError("previous target call interrupted; restart process")
        if self.active:
            raise RuntimeError("recursive dbgvis invocation")
        if value.is_optimized_out or value.address is None:
            raise RuntimeError("value has no addressable live storage")
        address = int(value.address)
        if address == 0 or address % entry["align"]:
            raise RuntimeError("invalid target address")
        mode_number = MODES[mode]
        if mode_number and not entry["capabilities"] & mode_number:
            raise RuntimeError("UNSUPPORTED: mode not registered for " + entry["name"])
        if not 0 < options["summary.buffer_bytes"] <= self.module["capacity"]:
            raise RuntimeError("buffer exceeds compiled capacity")
        self.ensure_callable(options)
        self.sequence += 1
        request = struct.pack("<10Q", 2, 80, self.sequence, entry["slot"], address, mode_number,
                              options["summary.buffer_bytes"], int(options["summary.alternate"]),
                              options["format.max_depth"], options["format.max_nodes"])
        module = dict(self.module)  # cont events can invalidate discovery during a call.
        self.active = True
        started = time.monotonic()
        try:
            gdb.selected_inferior().write_memory(module["request"], request)
            call = f"((unsigned long (*)(unsigned long))0x{module['dispatcher']:x})({module['request']})"
            self.calls += 1
            try:
                with gdb.with_parameter("language", "c"):
                    if self.timeout_supported():
                        with gdb.with_parameter("direct-call-timeout", max(1, math.ceil(options["execution.timeout_ms"] / 1000))):
                            with gdb.with_parameter("unwind-on-timeout", True):
                                status = int(gdb.parse_and_eval(call))
                    else:
                        status = int(gdb.parse_and_eval(call))
                sequence, returned, length, outcome, nodes = struct.unpack("<5Q", self.read(module["response"], 40))
                if sequence != self.sequence or returned != status or length > options["summary.buffer_bytes"]:
                    raise RuntimeError("invalid or stale response")
            except BaseException:
                self.poisoned = True
                raise
            if status not in (0, 1):
                label = {2: "UNSUPPORTED", 3: "INVALID", 4: "ABI_MISMATCH", 5: "BUSY", 6: "FORMAT_ERROR", 7: "PANIC", 8: "NOT_READY"}.get(status, str(status))
                if status == 5:
                    self.poisoned = True
                raise RuntimeError(label)
            text = self.read(module["output"], length).decode("utf-8")
            if status == 1:
                text += " <dbgvis: " + {1: "byte limit", 2: "depth limit", 3: "node limit", 4: "cycle"}.get(outcome, "limited") + ">"
            return text
        finally:
            self.elapsed_ms += (time.monotonic() - started) * 1000
            self.active = False

    def lookup(self, value):
        if self.active or self.native_depth:
            return None
        if self.config["execution"] == "manual" and not any(o.get("execution") == "automatic" for o in self.overrides.values()):
            return None
        try:
            entry = self.match(value)
            options = self.options(entry)
            if options["execution"] != "automatic" or options["summary.mode"] == "native":
                return None
            mode = MODES[options["summary.mode"]]
            if mode and not entry["capabilities"] & mode:
                return None
            self.ensure_callable(options)
            return Printer(self, value)
        except NoMatch:
            return None
        except Exception as error:
            self.last_error = str(error)
            return None

    def command(self, argument):
        args = shlex.split(argument)
        if not args or args[0] == "help":
            return "dbgvis print [--mode auto|visualize|debug|display|native] [--buffer N] [--alternate] EXPR\ndbgvis config [--type NAME] KEY VALUE\ndbgvis types | status | refresh | reset"
        command = args.pop(0)
        if command == "reset":
            self.config = dict(DEFAULTS)
            self.overrides.clear()
            self.changed(None)
            return "configuration reset; poisoned calls still require process restart"
        if command == "refresh":
            self.changed(None)
            return "discovery refreshed; no value cache"
        if command == "status":
            return json.dumps(dict(calls=self.calls, elapsed_ms=self.elapsed_ms, poisoned=self.poisoned, last_error=self.last_error), ensure_ascii=False)
        if command == "types":
            self.discover()
            if not self.entries:
                return "no registered roots; use derive(Visualize), #[dbgvis::register] or dbgvis::register_type!"
            return "\n".join(f"{e['slot']}: {e['name']} capabilities={e['capabilities']} anchor={'ok' if e['type'] is not None else 'unavailable'}" for e in self.entries)
        if command == "config":
            target = self.config
            if not args:
                return json.dumps(self.config, ensure_ascii=False)
            if args[0] == "--type":
                self.discover()
                if len(args) < 4 or args[1] not in {e["name"] for e in self.entries}:
                    raise RuntimeError("unknown canonical type; see dbgvis types")
                target = self.overrides.setdefault(args[1], {})
                args = args[2:]
            if len(args) != 2 or args[0] not in DEFAULTS:
                raise RuntimeError("expected config KEY VALUE")
            key, value = args
            if key == "summary.mode" and value not in {"native", *MODES}:
                raise RuntimeError("invalid summary mode")
            if key == "execution" and value not in ("manual", "automatic"):
                raise RuntimeError("invalid execution policy")
            if key == "summary.alternate":
                if value not in ("true", "false"):
                    raise RuntimeError("expected true or false")
                value = value == "true"
            elif isinstance(DEFAULTS[key], int):
                value = int(value)
                limit = {"format.max_depth": 128, "format.max_nodes": 1_000_000}.get(key, 16 * 1024 * 1024)
                if not 0 < value <= limit:
                    raise RuntimeError("configuration out of range")
            target[key] = value
            return f"{key}={value}"
        if command == "print":
            overrides = {}
            while args and args[0].startswith("--"):
                flag = args.pop(0)
                if flag == "--alternate":
                    overrides["summary.alternate"] = True
                elif flag in ("--mode", "--buffer") and args:
                    value = args.pop(0)
                    if flag == "--mode":
                        if value not in {"native", *MODES}:
                            raise RuntimeError("invalid mode")
                        overrides["summary.mode"] = value
                    else:
                        overrides["summary.buffer_bytes"] = int(value)
                else:
                    raise RuntimeError("unknown or incomplete print option")
            if not args:
                raise RuntimeError("missing variable expression")
            value = gdb.parse_and_eval(" ".join(args))
            if overrides.get("summary.mode") == "native":
                return self.native(value)
            entry = self.match(value)
            if self.options(entry, overrides)["summary.mode"] == "native":
                overrides["summary.mode"] = "auto"
            return self.summary(value, overrides)
        raise RuntimeError("unknown command; use dbgvis help")


class Printer:
    def __init__(self, session, value):
        self.session, self.value = session, value

    def to_string(self):
        try:
            return self.session.summary(self.value)
        except Exception as error:
            self.session.last_error = str(error)
            return self.session.native(self.value) + " <dbgvis: " + str(error) + ">"

    def children(self):
        self.session.native_depth += 1
        try:
            printer = gdb.default_visualizer(self.value)
            if hasattr(printer, "children"):
                yield from printer.children()
            elif self.value.type.strip_typedefs().code == gdb.TYPE_CODE_STRUCT:
                for field in self.value.type.strip_typedefs().fields():
                    if field.name:
                        yield field.name, self.value[field.name]
        finally:
            self.session.native_depth -= 1


class Command(gdb.Command):
    def __init__(self):
        super().__init__("dbgvis", gdb.COMMAND_DATA)

    def invoke(self, argument, from_tty):
        try:
            gdb.write(_session.command(argument) + "\n")
        except Exception as error:
            _session.last_error = str(error)
            raise gdb.GdbError("dbgvis: " + str(error))


_previous = globals().get("_session", globals().get("_previous"))
if _previous is not None:
    _previous.dispose()
_session = Session()
if _previous is not None:
    # Reloading scripts is not a process restart and must not erase interrupted calls.
    for _key in ("config", "overrides", "process", "poisoned", "sequence", "calls", "last_error", "elapsed_ms"):
        setattr(_session, _key, getattr(_previous, _key))
del _previous
Command()


def dispose():
    _session.dispose()
