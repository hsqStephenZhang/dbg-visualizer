import lldb
import os
import dbgvis_lldb as bridge

case = os.environ["DBGVIS_FAULT_CASE"]
session = bridge.get_session(lldb.debugger)
lldb.debugger.GetSelectedTarget().GetProcess().GetSelectedThread().SetSelectedFrame(1)
try:
    session.command("print --mode display fault")
    raise AssertionError("fault unexpectedly succeeded")
except bridge.VisualizerError as error:
    message = str(error)

if case in ("panic", "error"):
    assert message == {"panic": "PANIC", "error": "FORMAT_ERROR"}[case], message
    assert not session.poisoned
    assert session.backend.find_value("fault").GetChildMemberWithName("kind").SetValueFromCString("0")
    assert session.command("print --mode display fault") == "healthy"
else:
    assert session.poisoned, message
    if case == "hang":
        assert "timed out" in message.lower() or "timeout" in message.lower() or "interrupted" in message.lower(), message
    session.command("refresh")
    assert session.poisoned
    print("INTERRUPTED_CALL_DISABLED")
print("LLDB_FAULT_OK_" + case)
