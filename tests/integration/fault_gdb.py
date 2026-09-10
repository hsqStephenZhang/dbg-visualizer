import gdb
import os
import dbgvis_gdb as bridge

case = os.environ["DBGVIS_FAULT_CASE"]
session = bridge._session
gdb.newest_frame().older().select()
try:
    session.command("print --mode display fault")
    raise AssertionError("fault unexpectedly succeeded")
except bridge.VisualizerError as error:
    message = str(error)
except gdb.error as error:
    message = str(error)

if case in ("panic", "error"):
    assert message == {"panic": "PANIC", "error": "FORMAT_ERROR"}[case], message
    assert not session.poisoned
    gdb.execute("set var fault.kind = 0")
    assert session.command("print --mode display fault") == "healthy"
else:
    assert session.poisoned, message
    if case == "hang":
        assert "timed out" in message.lower() or "timeout" in message.lower(), message
    session.command("refresh")
    assert session.poisoned
    print("INTERRUPTED_CALL_DISABLED")
print("GDB_FAULT_OK_" + case)
