import gdb
import sys

gdb.execute("set confirm off")
gdb.execute("set debuginfod enabled off")
gdb.execute("break faults::checkpoint")
gdb.execute("run")
gdb.execute("up")
session = sys.modules["dbgvis_gdb"]._session
kind = int(gdb.parse_and_eval("fault.kind"))
abort = "/abort/" in gdb.current_progspace().filename
try:
    gdb.execute("dbgvis print fault")
    assert kind == 0
except gdb.error as error:
    message = str(error)
    if kind == 1 and gdb.selected_inferior().pid and not abort:
        assert "PANIC" in message
        assert not session.poisoned
    elif kind == 2:
        assert "FORMAT_ERROR" in message
        assert not session.poisoned
    else:
        assert session.poisoned
if kind in (1, 2) and not session.poisoned:
    gdb.execute("set variable fault.kind = 0")
    assert "healthy" in gdb.execute("dbgvis print fault", to_string=True)
elif session.poisoned:
    gdb.execute("dbgvis refresh")
    gdb.execute("dbgvis reset")
    assert session.poisoned
print("GDB_V2_FAULT_OK")
