"""Real rust-gdb assertions: no mock target memory, no named third-party printer branches."""
import gdb
import json
import sys

gdb.execute("set pagination off")
gdb.execute("set confirm off")
gdb.execute("set debuginfod enabled off")
gdb.execute("break demo::checkpoint")
gdb.execute("run")
gdb.execute("up")
session = sys.modules["dbgvis_gdb"]._session


def command(text):
    return gdb.execute("dbgvis " + text, to_string=True).strip()


def formatted(expression):
    before = session.calls
    result = command("print " + expression)
    assert session.calls == before + 1, (expression, session.calls, before)
    return result


assert session.calls == 0
baseline_map = command("print --mode native map")
baseline_app = command("print --mode native app")
assert session.calls == 0
gdb.execute("print point", to_string=True)
assert session.calls == 0, "manual view must not call target"
session.discover()
assert all(e["type"] is not None for e in session.entries)
names = {e["name"] for e in session.entries}
assert 'demo::Counter' in names
assert not any(name.startswith('demo::State<') for name in names), "nested generic type need not be a registered root"

assert formatted("map") == '{"points": [Some(Point { x: 1, y: 2 }), None]}'
assert formatted("custom") == '{(): 42}'
assert formatted("local") == 'Local { value: 17 }'
assert formatted("counter") == 'Counter { count: 23 }'
before = session.calls
try:
    command("print text")
    raise AssertionError("an unregistered Debug/Display type must not become an implicit root")
except gdb.error as error:
    assert "unregistered" in str(error)
assert session.calls == before
app = formatted("app")
assert 'points: {"app": [Some(Point { x: 5, y: 6 })]}' in app
assert 'State::Ready { value: 7 }' in app
assert 'secret' not in app
assert formatted("bytes") == 'b"hello world"'
assert formatted("ordered") == '{1: "one", 2: "two"}'
assert formatted("index_borrowed") == '{"临时字符串": 123}'
assert formatted("index_array") == '{[1, 2]: [(), ()]}'
assert formatted("external_map") == '{9: [8, 7]}'
assert formatted("--mode display address") == '127.0.0.1:8080'
assert formatted("point") == 'Point { x: 3, y: 7 }'
assert formatted("borrowed") == 'Borrowed { label: "临时字符串", value: [1, 2], marker: PhantomData }'
assert formatted("borrowed_other") == 'Borrowed { label: "临时字符串", value: [3, 4], marker: PhantomData }'
assert "byte limit" in formatted("--buffer 8 map")
assert formatted("--alternate point") == 'Point {\n  x: 3,\n  y: 7\n}'
before = session.calls
try:
    command("print --mode display bytes")
    raise AssertionError("unregistered Display was accepted")
except gdb.error as error:
    assert "UNSUPPORTED" in str(error)
assert session.calls == before, "unsupported mode must fail before target call"
gdb.execute("set may-call-functions off")
try:
    command("print point")
    raise AssertionError("disabled target calls were accepted")
except gdb.error as error:
    assert "may-call-functions" in str(error)
assert session.calls == before
gdb.execute("set may-call-functions on")

command("config format.max_nodes 2")
assert "node limit" in formatted("map")
command("reset")
command("config summary.mode auto")
command("config execution automatic")
before = session.calls
assert 'Point { x: 3, y: 7 }' in gdb.execute("print point", to_string=True)
assert session.calls == before + 1
command('config --type demo::Point summary.alternate true')
assert 'Point {\n  x: 3,\n  y: 7\n}' in gdb.execute("print point", to_string=True)
command("reset")
gdb.execute("set variable point.x = 99")
assert formatted("point") == 'Point { x: 99, y: 7 }'

gdb.execute("continue")
gdb.execute("up")
updated = formatted("app")
assert 'State::Pending' in updated and '3: "three"' in updated
command("refresh")
assert formatted("app") == updated
assert not json.loads(command("status"))["poisoned"]
print("NATIVE_MAP=" + baseline_map)
print("DBGVIS_MAP=" + formatted("map"))
print("NATIVE_APP_LENGTH=" + str(len(baseline_app)))
print("DBGVIS_APP=" + updated)
print("GDB_V2_INTEGRATION_OK")
