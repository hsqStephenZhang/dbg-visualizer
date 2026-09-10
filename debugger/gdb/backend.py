import gdb


class GdbBackend:
    def process_key(self):
        inferior = gdb.selected_inferior()
        return (inferior.num, inferior.pid)

    def identity(self):
        inferior = gdb.selected_inferior()
        return (inferior.num, inferior.pid, gdb.current_progspace().filename)

    def module_address(self):
        # no_mangle statics can be exposed only as minimal (non-DWARF) symbols by rustc.
        # Taking an address evaluates no target code. v1 supports the executable's root only.
        with gdb.with_parameter("language", "c"):
            address = int(gdb.parse_and_eval("(unsigned long)&DBG_VIS_MODULE_V1"))
        if gdb.solib_name(address) is not None:
            raise VisualizerError("v1 requires a registry in the main executable")
        return address

    def read(self, address, length):
        return bytes(gdb.selected_inferior().read_memory(address, length))

    def write(self, address, data):
        gdb.selected_inferior().write_memory(address, data)

    def value_type(self, value):
        typ = value.type.strip_typedefs()
        return str(typ), typ.sizeof, getattr(typ, "alignof", None)

    def owns_type(self, value):
        objfile = value.type.strip_typedefs().objfile
        if objfile is None: return False
        objfile = getattr(objfile, "owner", None) or objfile
        return os.path.realpath(objfile.filename) == os.path.realpath(gdb.current_progspace().filename)

    def value_address(self, value):
        if value.is_optimized_out or value.address is None:
            raise VisualizerError("value not stored in target memory")
        return int(value.address)

    def ensure_callable(self, options):
        inferior = gdb.selected_inferior()
        if inferior.pid == 0 or not inferior.threads():
            raise VisualizerError("target execution unavailable (core dump or no live process)")
        if inferior.connection is None or inferior.connection.type == "core":
            raise VisualizerError("target execution unavailable in a core dump")
        if any(thread.is_running() for thread in inferior.threads()):
            raise VisualizerError("all target threads must be stopped")
        if not gdb.parameter("may-call-functions"):
            raise VisualizerError("GDB may-call-functions is off")
        if options["execution"] == "automatic" and not self.timeout_supported():
            raise VisualizerError("automatic calls require GDB async timeout support")

    def timeout_supported(self):
        try:
            gdb.parameter("direct-call-timeout")  # None denotes unlimited, not unsupported.
            report = gdb.execute("show direct-call-timeout", to_string=True)
            async_report = gdb.execute("maintenance show target-async", to_string=True)
            return async_report.strip().endswith(" on.") and "does not support" not in report
        except Exception:
            return False

    def call(self, function, request, options):
        expression = f"((unsigned int (*)(unsigned long))0x{function:x})({request})"
        with gdb.with_parameter("language", "c"):
            if self.timeout_supported():
                with gdb.with_parameter("direct-call-timeout", max(1, math.ceil(options["execution.timeout_ms"] / 1000))):
                    with gdb.with_parameter("unwind-on-timeout", True):
                        return int(gdb.parse_and_eval(expression))
            return int(gdb.parse_and_eval(expression))

    def find_value(self, expression):
        return gdb.parse_and_eval(expression)

    def native(self, value):
        global _native_depth
        _native_depth += 1
        try:
            return value.format_string()
        finally:
            _native_depth -= 1

    def child_value(self, child):
        if child["kind"] == 1:
            with gdb.with_parameter("language", "c"):
                return gdb.Value(child["data"]).cast(gdb.lookup_type("unsigned long long"))
        names = [child["type"]]
        for entry in _session.entries:
            if normalize(child["type"]) in entry.names:
                names += [entry.name] + [name for name in entry.aliases if name]
        if child["type"] == "u8": names += ["unsigned char"]
        if child["type"] == "i32": names += ["int"]
        for name in names:
            try:
                if name.startswith(("*const ", "*mut ")):
                    pointee = gdb.lookup_type(name.split(" ", 1)[1])
                    typ = (pointee.const() if name.startswith("*const ") else pointee).pointer()
                else:
                    typ = gdb.lookup_type(name)
                if typ.sizeof != child["size"]:
                    continue
                return gdb.Value(child["address"]).cast(typ.pointer()).dereference()
            except gdb.error:
                continue
        if child["kind"] == 2:
            return self.child_text(child)
        return f"<unavailable type {child['type']}>"

    def child_text(self, child):
        if child["kind"] == 2:
            text = self.read(child["data"], min(child["length"], 4096)).decode("utf-8", errors="replace")
            return json.dumps(text, ensure_ascii=False) + (" … <已截断>" if child["length"] > 4096 else "")
        return str(self.child_value(child))

    def configuration_changed(self):
        pass  # No persistent value cache; existing GDB varobjs are refreshed by the client.


class GdbPrinter:
    def __init__(self, value, entry, options):
        self.value, self.entry, self.options = value, entry, options

    def to_string(self):
        try:
            if self.options["summary.mode"] == "native":
                return _session.backend.native(self.value)
            return _session.summary(self.value)
        except Exception as error:
            _session.diagnose(error)
            return _session.backend.native(self.value) + " <dbgvis: " + str(error) + ">"


class GdbChildrenPrinter(GdbPrinter):
    def display_hint(self):
        return {0: None, 1: "array", 2: "map"}[self.entry.shape]

    def children(self):
        try:
            total = _session.invoke(self.entry, self.value, 2, self.options)["total"]
            limit = min(total, self.options["children.max_items"])
            page_size = self.options["children.page_size"]
            for start in range(0, limit, page_size):
                result = _session.page(self.value, start, min(page_size, limit - start))
                for i, child in enumerate(result["children"]):
                    name = child["name"] or f"[{start + i}]"
                    yield name, _session.backend.child_value(child)
            if limit < total:
                yield "...", f"<{total - limit} more items; use dbgvis page>"
                if self.entry.shape == 2: yield "...", "<omitted>"
        except Exception as error:
            _session.diagnose(error)
            yield "<dbgvis error>", str(error)
            if self.entry.shape == 2: yield "<dbgvis error>", "<unavailable>"


class GdbNativeChildrenPrinter(GdbPrinter):
    def native_printer(self):
        global _native_depth
        _native_depth += 1
        try:
            return gdb.default_visualizer(self.value)
        finally:
            _native_depth -= 1

    def display_hint(self):
        printer = self.native_printer()
        return printer.display_hint() if hasattr(printer, "display_hint") else None

    def children(self):
        printer = self.native_printer()
        if hasattr(printer, "children"):
            yield from printer.children()
        else:
            for field in self.value.type.strip_typedefs().fields():
                if field.name is not None:
                    yield field.name, self.value[field.name]


_native_depth = 0
_session = Session(GdbBackend())
_events = []


def lookup(value):
    if _native_depth or _session.active:
        return None
    try:
        entry = _session.match(value)
        options = _session.options(entry)
        if options["execution"] != "automatic": return None
        summary = options["summary.mode"] != "native"
        children = options["children.mode"] == "structured" and entry.capabilities & 4
        if not summary and not children: return None
        if summary and not entry.capabilities & {"debug": 1, "display": 2}[options["summary.mode"]]:
            if not children: return None
            options["summary.mode"] = "native"
        _session.backend.ensure_callable(options)
        _session.backend.value_address(value)
        return (GdbChildrenPrinter if children else GdbNativeChildrenPrinter)(value, entry, options)
    except NoMatch:
        return None
    except Exception as error:
        _session.diagnose(error)
        return None


class DbgvisCommand(gdb.Command):
    def __init__(self):
        super().__init__("dbgvis", gdb.COMMAND_DATA)

    def invoke(self, argument, from_tty):
        try:
            gdb.write(_session.command(argument) + "\n")
        except Exception as error:
            _session.diagnose(error)
            raise gdb.GdbError("dbgvis: " + str(error))


def reset_event(event):
    if not _session.active: _session.reset()


def dispose():
    if lookup in gdb.pretty_printers: gdb.pretty_printers.remove(lookup)
    for event, callback in _events: event.disconnect(callback)


gdb.pretty_printers.insert(0, lookup)
for _event_name in ("exited", "new_objfile", "clear_objfiles", "free_objfile"):
    _event = getattr(gdb.events, _event_name, None)
    if _event is not None:
        _event.connect(reset_event)
        _events.append((_event, reset_event))
DbgvisCommand()
