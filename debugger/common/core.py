"""Debugger-independent v1 protocol. Bundled into the binary for GDB auto-load."""
import argparse
import json
import math
import os
import re
import shlex
import struct
import time


class VisualizerError(Exception):
    pass


class NoMatch(VisualizerError):
    pass


DEFAULTS = {
    "summary.mode": "native", "summary.alternate": False,
    "summary.buffer_bytes": 4096, "execution": "manual",
    "children.mode": "native", "children.page_size": 64,
    "children.max_items": 1024, "execution.timeout_ms": 200,
}
CHOICES = {"summary.mode": ("native", "debug", "display"),
           "execution": ("manual", "automatic"), "children.mode": ("native", "structured")}
STATUS = {0: "OK", 1: "TRUNCATED", 2: "UNSUPPORTED", 3: "INVALID_REQUEST",
          4: "ABI_MISMATCH", 5: "BUSY", 6: "FORMAT_ERROR", 7: "PANIC"}


def normalize(name):
    # Only whitespace is universally safe to normalize. Integer aliases are explicit metadata.
    return re.sub(r"\s+", "", name)


class Config:
    def __init__(self):
        self.global_options = dict(DEFAULTS)
        self.types = {}

    def resolve(self, name, overrides=None):
        result = dict(self.global_options)
        result.update(self.types.get(name, {}))
        result.update(overrides or {})
        return result

    def set(self, key, value, typename=None):
        if key not in DEFAULTS:
            raise VisualizerError("unknown option: " + key)
        if key in CHOICES:
            if value not in CHOICES[key]:
                raise VisualizerError("expected " + "/".join(CHOICES[key]))
        elif isinstance(DEFAULTS[key], bool):
            if value not in ("true", "false"):
                raise VisualizerError("expected true/false")
            value = value == "true"
        else:
            try:
                value = int(value)
            except ValueError:
                raise VisualizerError("expected a positive integer") from None
            limit = 64 if key == "children.page_size" else 16 * 1024 * 1024
            if not 1 <= value <= limit:
                raise VisualizerError(f"expected 1..{limit}")
        destination = self.global_options if typename is None else self.types.setdefault(typename, {})
        destination[key] = value


class Entry:
    def __init__(self, ident, words, text):
        self.id = ident
        self.name = text(words[0], words[1])
        self.aliases = [text(words[2], words[3]), text(words[4], words[5])]
        self.size, self.align, self.capabilities, self.shape = words[6:10]
        self.names = {normalize(n) for n in [self.name] + self.aliases if n}


class Session:
    def __init__(self, backend):
        self.backend = backend
        self.config = Config()
        self.entries = []
        self.identity = None
        self.process_key = None
        self.module = None
        self.sequence = 0
        self.active = False
        self.poisoned = False
        self.last_error = ""
        self.calls = 0
        self.elapsed = 0.0
        self.operations = {1: 0, 2: 0, 3: 0}

    def reset(self):
        self.identity = None
        self.module = None
        self.entries = []

    def discover(self):
        identity = self.backend.identity()
        process_key = self.backend.process_key()
        if process_key != self.process_key:
            self.poisoned = False
            self.process_key = process_key
        if self.identity == identity and self.module is not None:
            return
        self.reset()
        address = self.backend.module_address()
        header = struct.unpack("<8s8I9Q", self.backend.read(address, 112))
        if header[:9] != (b"DBGVIS01", 1, 112, 8, 1, 112, 88, 48, 304):
            raise VisualizerError("ABI_MISMATCH: requires v1, 64-bit little-endian target")
        keys = ("entries", "entry_count", "request", "response", "output", "capacity",
                "children", "children_capacity", "dispatcher")
        module = dict(zip(keys, header[9:]))
        if not 0 < module["entry_count"] <= 4096 or not 0 < module["capacity"] <= 16 * 1024 * 1024 or module["children_capacity"] != 128:
            raise VisualizerError("invalid module bounds")
        entries = []
        for i in range(module["entry_count"]):
            words = struct.unpack("<14Q", self.backend.read(module["entries"] + i * 112, 112))
            entry = Entry(i, words, self.read_text)
            if entry.align == 0 or entry.align & (entry.align - 1) or entry.shape not in (0, 1, 2):
                raise VisualizerError("invalid entry layout")
            entries.append(entry)
        self.entries, self.module, self.identity = entries, module, identity

    def read_text(self, address, length):
        if length > 4096:
            raise VisualizerError("metadata text exceeds limit")
        return self.backend.read(address, length).decode("utf-8") if length else ""

    def match(self, value):
        self.discover()
        if not self.backend.owns_type(value):
            raise NoMatch("type belongs to a different or unknown module")
        name, size, alignment = self.backend.value_type(value)
        matches = [entry for entry in self.entries if normalize(name) in entry.names and size == entry.size
                   and (alignment is None or alignment == entry.align)]
        if len(matches) != 1:
            raise NoMatch("unregistered or ambiguous type: " + name)
        return matches[0]

    def options(self, entry, overrides=None):
        return self.config.resolve(entry.name, overrides)

    def invoke(self, entry, value, operation, options, start=0, count=0):
        self.discover()
        if self.active:
            raise VisualizerError("BUSY: recursive formatter invocation")
        if self.poisoned:
            raise VisualizerError("previous target call was interrupted; restart the process")
        self.backend.ensure_callable(options)
        address = self.backend.value_address(value)
        if not address or address % entry.align:
            raise VisualizerError("value has no valid aligned target address")
        capacity = options["summary.buffer_bytes"]
        if not 1 <= capacity <= self.module["capacity"]:
            raise VisualizerError(f"buffer size must be 1..{self.module['capacity']}")
        mode = {"native": 0, "debug": 1, "display": 2}[options["summary.mode"]]
        if operation == 1 and not entry.capabilities & mode:
            raise VisualizerError("UNSUPPORTED: " + options["summary.mode"])
        if operation != 1 and not entry.capabilities & 4:
            raise VisualizerError("UNSUPPORTED: children")
        self.sequence += 1
        request = struct.pack("<11Q", 1, 88, self.sequence, operation, entry.id, address,
                              mode, int(options["summary.alternate"]), capacity, start, count)
        self.active = True
        began = time.monotonic()
        try:
            self.backend.write(self.module["request"], request)
            try:
                status = self.backend.call(self.module["dispatcher"], self.module["request"], options)
            except Exception:
                self.poisoned = True
                raise
            self.calls += 1
            self.operations[operation] += 1
            seq, response_status, written, total, returned, shape = struct.unpack(
                "<6Q", self.backend.read(self.module["response"], 48))
            if status == 5:
                self.poisoned = True
                raise VisualizerError("BUSY: runtime still occupied")
            if seq != self.sequence or status != response_status:
                self.poisoned = True
                raise VisualizerError("stale or inconsistent response")
            if status not in (0, 1):
                raise VisualizerError(STATUS.get(status, f"unknown status {status}"))
            if written > capacity or returned > 128 or shape != entry.shape:
                raise VisualizerError("invalid response bounds")
            text = self.backend.read(self.module["output"], written).decode("utf-8") if written else ""
            children = []
            if operation == 3:
                expected = min(count, max(0, total - start)) * (2 if shape == 2 else 1)
                if returned != expected:
                    raise VisualizerError("invalid child count")
                raw = self.backend.read(self.module["children"], returned * 304) if returned else b""
                for i in range(returned):
                    name, typename, addr, size, alignment, kind, data, length = struct.unpack_from("<64s192s6Q", raw, i * 304)
                    child = dict(name=name.split(b"\0", 1)[0].decode("utf-8"),
                                 type=typename.split(b"\0", 1)[0].decode("utf-8"),
                                 address=addr, size=size, align=alignment, kind=kind, data=data, length=length)
                    if kind not in (0, 1, 2) or alignment == 0 or alignment & (alignment - 1):
                        raise VisualizerError("invalid child descriptor")
                    if kind != 1 and (addr == 0 or addr % alignment):
                        raise VisualizerError("invalid child address")
                    children.append(child)
            return dict(text=text, truncated=status == 1, total=total, shape=shape, children=children)
        finally:
            self.active = False
            self.elapsed += time.monotonic() - began

    def summary(self, value, overrides=None):
        entry = self.match(value)
        options = self.options(entry, overrides)
        if options["summary.mode"] == "native":
            return self.backend.native(value)
        result = self.invoke(entry, value, 1, options)
        return result["text"] + (" … <已截断>" if result["truncated"] else "")

    def page(self, value, start, count, overrides=None):
        entry = self.match(value)
        options = self.options(entry, overrides)
        if start < 0 or not 1 <= count <= 64:
            raise VisualizerError("page requires start >= 0 and count in 1..64")
        return self.invoke(entry, value, 3, options, start, count)

    def diagnose(self, error):
        self.last_error = str(error)

    def command(self, text):
        args = shlex.split(text)
        if not args or args[0] == "help":
            return ("dbgvis config [--type CANONICAL_NAME] KEY VALUE\n"
                    "dbgvis print [--mode native|debug|display] [--buffer N] [--alternate] EXPR\n"
                    "dbgvis page [--start N] [--count N] EXPR\n"
                    "dbgvis types | status | refresh | reset\n"
                    "Defaults: native, execution=manual. Automatic views require execution=automatic.")
        command = args.pop(0)
        if command == "config":
            typename = None
            if len(args) >= 2 and args[0] == "--type":
                typename = args[1]; args = args[2:]
            if not args:
                return json.dumps({"global": self.config.global_options, "types": self.config.types}, ensure_ascii=False)
            if len(args) != 2:
                raise VisualizerError("config requires KEY VALUE")
            if typename is not None:
                self.discover()
                if typename not in [e.name for e in self.entries]:
                    raise VisualizerError("unknown canonical type; use dbgvis types")
            self.config.set(args[0], args[1], typename)
            self.backend.configuration_changed()
            return "configuration updated"
        if command in ("print", "page"):
            parser = argparse.ArgumentParser(prog="dbgvis " + command, add_help=False, exit_on_error=False)
            parser.add_argument("--mode", choices=("native", "debug", "display"))
            parser.add_argument("--buffer", type=int)
            parser.add_argument("--alternate", action="store_true")
            parser.add_argument("--start", type=int, default=0)
            parser.add_argument("--count", type=int)
            parser.add_argument("expression", nargs="+")
            try:
                parsed = parser.parse_args(args)
            except (argparse.ArgumentError, SystemExit) as error:
                raise VisualizerError("invalid command arguments") from error
            value = self.backend.find_value(" ".join(parsed.expression))
            overrides = {}
            if parsed.mode is not None: overrides["summary.mode"] = parsed.mode
            if parsed.buffer is not None: overrides["summary.buffer_bytes"] = parsed.buffer
            if parsed.alternate: overrides["summary.alternate"] = True
            if command == "print":
                if parsed.mode == "native":
                    return self.backend.native(value)
                entry = self.match(value)
                if parsed.mode is None and self.options(entry)["summary.mode"] == "native":
                    overrides["summary.mode"] = "debug"
                return self.summary(value, overrides)
            count = parsed.count if parsed.count is not None else self.options(self.match(value))["children.page_size"]
            result = self.page(value, parsed.start, count, overrides)
            lines = [f"items {parsed.start}..{min(parsed.start + count, result['total'])} of {result['total']}"]
            for i, child in enumerate(result["children"]):
                index = parsed.start + (i // 2 if result["shape"] == 2 else i)
                label = child["name"] or str(index)
                if result["shape"] == 2: label = f"[{index}].{label}"
                lines.append(f"{label}: {self.backend.child_text(child)}")
            return "\n".join(lines)
        if command == "types":
            self.discover()
            return "\n".join(f"{e.id}: {e.name} size={e.size} align={e.align} capabilities={e.capabilities}" for e in self.entries)
        if command == "status":
            return json.dumps(dict(error=self.last_error, poisoned=self.poisoned, calls=self.calls,
                                   operations=self.operations, elapsed_ms=round(self.elapsed * 1000, 3)), ensure_ascii=False)
        if command == "refresh":
            self.backend.configuration_changed()
            return "views invalidated (interrupted calls remain disabled)"
        if command == "reset":
            self.config = Config()
            self.backend.configuration_changed()
            return "configuration reset"
        raise VisualizerError("unknown command; use dbgvis help")
