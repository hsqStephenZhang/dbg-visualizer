"""Automatic roots in the Debug-based demo, including borrowed third-party values."""
import gdb
import sys

gdb.execute("set pagination off")
gdb.execute("set confirm off")
gdb.execute("set debuginfod enabled off")
gdb.execute("break demo::checkpoint")
gdb.execute("run")
gdb.execute("up")
session = sys.modules["dbgvis_gdb"]._session


def formatted(expression):
    before = session.calls
    result = gdb.execute("dbgvis p " + expression, to_string=True).strip()
    assert session.calls == before + 1
    return result


for expression, expected in (
    ("map", '{"points": [Some(Point { x: 1, y: 2 }), None]}'),
    ("custom", '{(): 42}'),
    ("ordered", '{1: "one", 2: "two"}'),
    ("index_borrowed", '{"临时字符串": 123}'),
    ("index_array", '{[1, 2]: [(), ()]}'),
    ("external_map", '{9: [8, 7]}'),
    ("bytes", 'b"hello world"'),
    ("counter", 'Counter { count: 23 }'),
    ("local", 'Local { value: 17 }'),
    ("borrowed", 'Borrowed { label: "临时字符串", value: [1, 2], marker: PhantomData<demo::Marker> }'),
    ("borrowed_other", 'Borrowed { label: "临时字符串", value: [3, 4], marker: PhantomData<demo::Marker> }'),
):
    actual = formatted(expression)
    assert actual == expected, (expression, actual)
app = formatted("app")
assert 'points: {"app": [Some(Point { x: 5, y: 6 })]}' in app
assert 'state: Ready { value: 7 }' in app and 'secret: Marker' in app
gdb.execute("continue")
gdb.execute("up")
app = formatted("app")
assert 'state: Pending' in app and '3: "three"' in app
session.discover()
assert all(entry["type"] is not None for entry in session.entries)
gdb.execute("continue")
assert gdb.selected_inferior().pid == 0
print("AUTO_DEMO_GDB_OK")
