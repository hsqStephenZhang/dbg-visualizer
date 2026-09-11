"""Load the generated `.debug_gdb_scripts` prologue like GDB does, twice, against a stub gdb.

Checks the source escaping round-trips, the module is published, a reload disposes the
previous session while inheriting its state, a broken or foreign `dbgvis_gdb` module is
tolerated, and a failed load leaves the previously working module in place.
"""
import sys
import types


class Event:
    def __init__(self):
        self.handlers = []

    def connect(self, handler):
        self.handlers.append(handler)

    def disconnect(self, handler):
        self.handlers.remove(handler)


class Command:
    registered = {}

    def __init__(self, name, command_class):
        Command.registered[name] = self


gdb = types.ModuleType("gdb")
gdb.pretty_printers = []
gdb.events = types.SimpleNamespace(new_objfile=Event(), clear_objfiles=Event(), exited=Event(), cont=Event())
gdb.COMMAND_DATA = 0
gdb.Command = Command
gdb.GdbError = type("GdbError", (Exception,), {})
sys.modules["gdb"] = gdb

code = compile(open(sys.argv[1], encoding="utf-8").read(), sys.argv[1], "exec")
main_globals = {"__name__": "__main__"}  # GDB runs every section script in its __main__


def load():
    exec(code, main_globals)
    return sys.modules["dbgvis_gdb"]


def handlers():
    return [len(event.handlers) for event in vars(gdb.events).values()]


# Fresh load publishes the module, registers one printer, four event handlers and the command.
module = load()
first = module._session
assert "GDB-only v2 bridge" in module.__doc__, "docstring escaping did not round-trip"
assert gdb.pretty_printers == [first.lookup] and handlers() == [1, 1, 1, 1]
assert "dbgvis" in Command.registered
assert "\n" in first.command("help") and "dbgvis types" in first.command("help"), "newline escaping"
assert first.command("config summary.mode auto") == "summary.mode=auto"

# Reload: the previous session is disposed, its state inherited, nothing registered twice.
first.poisoned, first.calls, first.last_error = True, 3, "interrupted"
second = load()._session
assert second is not first
assert (second.poisoned, second.calls, second.last_error) == (True, 3, "interrupted")
assert second.config["summary.mode"] == "auto"
assert gdb.pretty_printers == [second.lookup] and handlers() == [1, 1, 1, 1]

# A broken previous module (no _session) must not wedge every later load.
second.dispose()
sys.modules["dbgvis_gdb"] = types.ModuleType("dbgvis_gdb")
third = load()._session
assert third.poisoned is False and third.calls == 0
assert gdb.pretty_printers == [third.lookup] and handlers() == [1, 1, 1, 1]

# A failed load must not replace the working module or leak registrations.
gdb.Command = None
try:
    load()
    raise AssertionError("load unexpectedly succeeded without gdb.Command")
except TypeError:
    pass
gdb.Command = Command
assert sys.modules["dbgvis_gdb"]._session is third
assert gdb.pretty_printers == [third.lookup] and handlers() == [1, 1, 1, 1]
fourth = load()._session
assert fourth is not third and gdb.pretty_printers == [fourth.lookup] and handlers() == [1, 1, 1, 1]
print("PROLOGUE_OK")
