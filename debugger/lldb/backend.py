import lldb


class LldbBackend:
    def __init__(self, debugger):
        self.debugger = debugger
        self.native_depth = 0
        self.registered = None
        self.revision = 0
        self.type_catalog = {}
        self.type_catalog_identity = None

    @property
    def target(self):
        return self.debugger.GetSelectedTarget()

    def process_key(self):
        return self.target.GetProcess().GetUniqueID()

    def identity(self):
        target = self.target
        if not target.IsValid():
            raise VisualizerError("no selected target")
        process = target.GetProcess()
        return (str(target.GetExecutable()), process.GetUniqueID(),
                tuple(target.GetModuleAtIndex(i).GetUUIDString() for i in range(target.GetNumModules())))

    def module_address(self):
        target = self.target
        if target.GetAddressByteSize() != 8 or target.GetByteOrder() != lldb.eByteOrderLittle:
            raise VisualizerError("v1 requires a 64-bit little-endian target")
        symbol = target.GetModuleAtIndex(0).FindSymbol("DBG_VIS_MODULE_V1", lldb.eSymbolTypeData)
        if not symbol.IsValid():
            raise VisualizerError("root DBG_VIS_MODULE_V1 not found")
        address = symbol.GetStartAddress().GetLoadAddress(target)
        if address == lldb.LLDB_INVALID_ADDRESS:
            raise VisualizerError("module is not loaded; stop the process first")
        return address

    def read(self, address, length):
        error = lldb.SBError()
        data = self.target.GetProcess().ReadMemory(address, length, error)
        if error.Fail() or len(data) != length:
            raise VisualizerError("memory read failed: " + str(error))
        return data

    def write(self, address, data):
        error = lldb.SBError()
        size = self.target.GetProcess().WriteMemory(address, data, error)
        if error.Fail() or size != len(data):
            raise VisualizerError("memory write failed: " + str(error))

    def value_type(self, value):
        typ = value.GetType().GetCanonicalType()
        alignment = typ.GetByteAlign() if hasattr(typ, "GetByteAlign") else None
        return typ.GetName(), typ.GetByteSize(), alignment or None

    def owns_type(self, value):
        # SBValue's compiler type can lose its SBModule association. Compare its canonical
        # type identity against the root module's DWARF catalog instead of names alone.
        typ = value.GetType().GetCanonicalType()
        names = [typ.GetName()]
        for entry in get_session(self.debugger).entries:
            if normalize(typ.GetName()) in entry.names:
                names += [entry.name] + [name for name in entry.aliases if name]
        for name in names:
            if any(candidate.GetCanonicalType() == typ for candidate in self.root_types(name)):
                return True
        return False

    def root_types(self, name):
        root = self.target.GetModuleAtIndex(0)
        candidates = root.FindTypes(name)
        if candidates.GetSize():
            return [candidates.GetTypeAtIndex(i) for i in range(candidates.GetSize())]
        # LLDB's name lookup cannot find some Rust generic DWARF names even though GetTypes
        # exposes their exact compiler type identity. Build a module-scoped fallback index once.
        identity = (str(root.GetFileSpec()), root.GetUUIDString())
        if self.type_catalog_identity != identity:
            self.type_catalog = {}
            types = root.GetTypes()
            for i in range(types.GetSize()):
                typ = types.GetTypeAtIndex(i)
                if typ.GetName():
                    self.type_catalog.setdefault(normalize(typ.GetName()), []).append(typ)
            self.type_catalog_identity = identity
        return self.type_catalog.get(normalize(name), [])

    def value_address(self, value):
        value = value.GetNonSyntheticValue()
        address = value.GetLoadAddress()
        if not value.IsValid() or value.GetError().Fail() or address in (0, lldb.LLDB_INVALID_ADDRESS):
            raise VisualizerError("value not stored in target memory")
        return address

    def ensure_callable(self, options):
        process = self.target.GetProcess()
        if not process.IsValid() or process.GetState() != lldb.eStateStopped:
            raise VisualizerError("target execution requires a stopped live process")
        plugin = process.GetPluginName() or ""
        if "core" in plugin.lower():
            raise VisualizerError("target execution unavailable in a core dump")
        if not hasattr(lldb.SBExpressionOptions, "SetTimeoutInMicroSeconds"):
            raise VisualizerError("this LLDB lacks expression timeouts")

    def call(self, function, request, options):
        expression_options = lldb.SBExpressionOptions()
        expression_options.SetLanguage(lldb.eLanguageTypeC)
        expression_options.SetTimeoutInMicroSeconds(options["execution.timeout_ms"] * 1000)
        expression_options.SetOneThreadTimeoutInMicroSeconds(options["execution.timeout_ms"] * 1000)
        expression_options.SetTryAllThreads(False)
        expression_options.SetStopOthers(True)
        expression_options.SetIgnoreBreakpoints(True)
        expression_options.SetUnwindOnError(True)
        expression_options.SetSuppressPersistentResult(True)
        result = self.target.EvaluateExpression(
            f"((unsigned int (*)(unsigned long))0x{function:x})({request})", expression_options)
        if not result.IsValid() or result.GetError().Fail():
            raise VisualizerError("expression failed: " + str(result.GetError()))
        return result.GetValueAsUnsigned()

    def find_value(self, expression):
        frame = self.target.GetProcess().GetSelectedThread().GetSelectedFrame()
        if not frame.IsValid():
            raise VisualizerError("select a frame with local variables")
        # FindVariable/GetValueForVariablePath preserve lvalue addresses, unlike copied expressions.
        value = frame.FindVariable(expression)
        if not value.IsValid():
            value = frame.GetValueForVariablePath(expression)
        if not value.IsValid() or value.GetError().Fail():
            raise VisualizerError("variable path not found: " + expression)
        return value.GetNonSyntheticValue()

    def native(self, value):
        category = self.debugger.GetCategory("dbgvis")
        was_enabled = category.IsValid() and category.GetEnabled()
        self.native_depth += 1
        try:
            if was_enabled: category.SetEnabled(False)
            return value.GetSummary() or value.GetValue() or str(value)
        finally:
            if was_enabled: category.SetEnabled(True)
            self.native_depth -= 1

    def child_value(self, child, name=None):
        name = name or child["name"] or "item"
        if child["kind"] == 1:
            data = lldb.SBData()
            error = lldb.SBError()
            data.SetDataWithOwnership(error, struct.pack("<Q", child["data"]), lldb.eByteOrderLittle, 8)
            if error.Fail(): raise VisualizerError(str(error))
            return self.target.CreateValueFromData(name, data, self.target.GetBasicType(lldb.eBasicTypeUnsignedLongLong))
        names = [child["type"]]
        for entry in get_session(self.debugger).entries:
            if normalize(child["type"]) in entry.names:
                names += [entry.name] + [name for name in entry.aliases if name]
        matches = []
        for typename in names:
            matches += [typ for typ in self.root_types(typename) if typ.GetByteSize() == child["size"]]
        if not matches and child["type"].startswith(("*const ", "*mut ")):
            for pointee in self.root_types(child["type"].split(" ", 1)[1]):
                pointer = pointee.GetPointerType()
                if pointer.GetByteSize() == child["size"]: matches.append(pointer)
        # Explicit target-sized primitive aliases only.
        if not matches and child["type"] in ("i32", "u8"):
            basic = lldb.eBasicTypeInt if child["type"] == "i32" else lldb.eBasicTypeUnsignedChar
            typ = self.target.GetBasicType(basic)
            if typ.GetByteSize() == child["size"]: matches = [typ]
        if matches:
            canonical_names = {typ.GetCanonicalType().GetName() for typ in matches}
            if len(canonical_names) == 1:
                return self.target.CreateValueFromAddress(name, lldb.SBAddress(child["address"], self.target), matches[0])
        return self.text_value(name, self.child_text(child, fallback=True))

    def text_value(self, name, text):
        data = lldb.SBData()
        error = lldb.SBError()
        raw = text.encode("utf-8") + b"\0"
        data.SetDataWithOwnership(error, raw, lldb.eByteOrderLittle, 8)
        if error.Fail(): raise VisualizerError(str(error))
        typ = self.target.GetBasicType(lldb.eBasicTypeChar).GetArrayType(len(raw))
        return self.target.CreateValueFromData(name, data, typ)

    def child_text(self, child, fallback=False):
        if child["kind"] == 2:
            text = self.read(child["data"], min(child["length"], 4096)).decode("utf-8", errors="replace")
            return json.dumps(text, ensure_ascii=False) + (" … <已截断>" if child["length"] > 4096 else "")
        if fallback: return f"<unavailable type {child['type']}>"
        return self.native(self.child_value(child))

    def configuration_changed(self):
        self.revision += 1
        self.registered = None
        self.register_views()

    def register_views(self):
        session = get_session(self.debugger)
        try:
            session.discover()
        except Exception:
            # Loading before target creation is supported; a stop hook will retry.
            return
        signature = (session.identity, json.dumps(session.config.global_options, sort_keys=True),
                     json.dumps(session.config.types, sort_keys=True))
        if self.registered == signature: return
        self.debugger.DeleteCategory("dbgvis")
        category = self.debugger.CreateCategory("dbgvis")
        for entry in session.entries:
            options = session.options(entry)
            if options["execution"] != "automatic": continue
            names = set([entry.name] + entry.aliases) - {""}
            for name in names:
                spec = lldb.SBTypeNameSpecifier(name, False)
                mode = options["summary.mode"]
                if mode != "native" and entry.capabilities & {"debug": 1, "display": 2}[mode]:
                    category.AddTypeSummary(spec, lldb.SBTypeSummary.CreateWithFunctionName(__name__ + ".summary"))
                if options["children.mode"] == "structured" and entry.capabilities & 4:
                    category.AddTypeSynthetic(spec, lldb.SBTypeSynthetic.CreateWithClassName(__name__ + ".SyntheticChildren"))
        category.SetEnabled(True)
        self.registered = signature


_sessions = {}


def get_session(debugger):
    key = debugger.GetID()
    if key not in _sessions:
        _sessions[key] = Session(LldbBackend(debugger))
    return _sessions[key]


def summary(value, internal_dict, options=None):
    session = get_session(value.GetTarget().GetDebugger())
    if session.active or session.backend.native_depth: return None
    try:
        return session.summary(value.GetNonSyntheticValue())
    except Exception as error:
        session.diagnose(error)
        # An error in a registered summary uses the non-synthetic raw fields, never recurses.
        raw = value.GetNonSyntheticValue()
        fields = []
        for i in range(min(raw.GetNumChildren(), 10)):
            child = raw.GetChildAtIndex(i)
            fields.append(f"{child.GetName()}={child.GetValue() or '<fields>'}")
        return "{" + ", ".join(fields) + "} <dbgvis: " + str(error) + ">"


class SyntheticChildren:
    def __init__(self, value, internal_dict):
        self.value = value.GetNonSyntheticValue()
        self.session = get_session(value.GetTarget().GetDebugger())
        self.pages = {}
        self.stamp = None
        self.total = 0
        self.entry = None
        self.error = None

    def epoch(self):
        process = self.value.GetProcess()
        return (process.GetUniqueID(), process.GetStopID(True), self.session.calls, self.session.backend.revision)

    def update(self):
        if self.session.active: return False
        if self.stamp != self.epoch():
            self.pages.clear()
            self.stamp = None
            self.entry = None
            self.error = None
        return False

    def prepare(self):
        if self.session.active: raise VisualizerError("BUSY: synthetic reentry")
        epoch = self.epoch()
        # Re-read the root header to detect same-stop debugger writes. Returned child values
        # are constructed anew from live addresses. Custom global-dependent adapters require
        # explicit refresh after writes that cannot be detected through the object or stop ID.
        fingerprint = None
        if self.entry is not None and self.entry.size <= 4096:
            fingerprint = self.session.backend.read(self.session.backend.value_address(self.value), self.entry.size)
        if self.stamp == epoch and fingerprint is not None and fingerprint == getattr(self, "fingerprint", None):
            return
        self.pages.clear()
        self.entry = self.session.match(self.value)
        self.options = self.session.options(self.entry)
        if self.options["execution"] != "automatic" or self.options["children.mode"] != "structured":
            raise VisualizerError("structured view disabled")
        result = self.session.invoke(self.entry, self.value, 2, self.options)
        self.total = result["total"]
        self.limit = min(self.total, self.options["children.max_items"])
        self.multiplier = 2 if self.entry.shape == 2 else 1
        self.fingerprint = self.session.backend.read(self.session.backend.value_address(self.value), self.entry.size) if self.entry.size <= 4096 else None
        self.stamp = self.epoch()

    def num_children(self, max_count=None):
        try:
            self.prepare()
            count = self.limit * self.multiplier + int(self.total > self.limit)
        except Exception as error:
            self.session.diagnose(error)
            self.error = str(error)
            count = self.value.GetNumChildren()
        return min(count, max_count) if max_count is not None else count

    def has_children(self):
        return self.num_children() > 0

    def get_child_index(self, name):
        try:
            self.prepare()
            match = re.fullmatch(r"\[(\d+)\](?:\.(key|value))?", name)
            if match:
                return int(match[1]) * self.multiplier + int(match[2] == "value")
            # Fields are few; their labels are part of a bounded page, queried only on demand.
            for index in range(self.limit):
                if self.get_child_at_index(index).GetName() == name: return index
        except Exception as error:
            self.session.diagnose(error)
        return lldb.UINT32_MAX

    def get_child_at_index(self, index):
        try:
            self.prepare()
            if index == self.limit * self.multiplier and self.total > self.limit:
                return self.session.backend.text_value("...", f"{self.total - self.limit} more items; use dbgvis page")
            if not 0 <= index < self.limit * self.multiplier: return None
            logical = index // self.multiplier
            page_size = self.options["children.page_size"]
            start = logical // page_size * page_size
            if start not in self.pages:
                result = self.session.page(self.value, start, min(page_size, self.limit - start))
                self.pages[start] = result["children"]
                self.stamp = self.epoch()
            child = self.pages[start][index - start * self.multiplier]
            if self.entry.shape == 2: name = f"[{logical}].{child['name']}"
            else: name = child["name"] or f"[{logical}]"
            return self.session.backend.child_value(child, name)
        except Exception as error:
            self.session.diagnose(error)
            return self.value.GetChildAtIndex(index)


def command(debugger, text, result, internal_dict):
    session = get_session(debugger)
    try:
        result.AppendMessage(session.command(text))
    except Exception as error:
        session.diagnose(error)
        result.SetError("dbgvis: " + str(error))


class RefreshHook:
    def __init__(self, target, extra_args, internal_dict):
        self.debugger = target.GetDebugger()

    def handle_stop(self, execution_context, stream):
        get_session(self.debugger).backend.register_views()
        return True


def install_hook(debugger, text, result, internal_dict):
    # Explicit preRun command: target exists here, whereas initCommands can precede creation.
    target = debugger.GetSelectedTarget()
    if not target.IsValid():
        result.SetError("create a target before dbgvis-install-hook")
        return
    session = get_session(debugger)
    identity = str(target.GetExecutable())
    installed = getattr(session.backend, "hook_targets", set())
    if identity not in installed:
        response = lldb.SBCommandReturnObject()
        debugger.GetCommandInterpreter().HandleCommand("target stop-hook add -P " + __name__ + ".RefreshHook", response)
        if not response.Succeeded():
            result.SetError(response.GetError())
            return
        installed.add(identity)
        session.backend.hook_targets = installed
    result.AppendMessage("dbgvis stop hook installed")


def __lldb_init_module(debugger, internal_dict):
    debugger.HandleCommand("command script add -o -f " + __name__ + ".command dbgvis")
    debugger.HandleCommand("command script add -o -f " + __name__ + ".install_hook dbgvis-install-hook")
    get_session(debugger).backend.configuration_changed()
