"""GDB assertions for examples/auto_poc.rs, including exact target-call counts."""
import gdb
import sys

gdb.execute("set pagination off")
gdb.execute("set confirm off")
gdb.execute("set debuginfod enabled off")
gdb.execute("break auto_poc::checkpoint")
gdb.execute("run")
gdb.execute("up")
session = sys.modules["dbgvis_gdb"]._session
for expression, expected in (
    ("map", "{7: [Some(Point { x: 42 })]}"),
    ("ordered", '{1: "one"}'),
    ("debug_only", "DebugOnly { count: 9 }"),
):
    before = session.calls
    actual = gdb.execute("dbgvis print " + expression, to_string=True).strip()
    assert actual == expected, (expression, actual)
    assert session.calls == before + 1
session.discover()
assert all(entry["type"] is not None for entry in session.entries)
names = [entry["name"] for entry in session.entries]
assert len(names) == len(set(names)), names
assert names.count("auto_poc::Point") == 1
gdb.execute("continue")
assert gdb.selected_inferior().pid == 0
print("AUTO_GDB_OK")
