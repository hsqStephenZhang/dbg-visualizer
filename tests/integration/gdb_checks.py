import gdb
import sys


def check():
    import dbgvis_gdb as bridge
    session = bridge._session
    gdb.newest_frame().older().select()
    backend = session.backend
    value = backend.find_value
    assert session.calls == 0
    assert int(backend.child_value({"kind": 1, "data": 2**64 - 1})) == 2**64 - 1
    class ExistingPrinter:
        def __init__(self, value): self.value = value
        def to_string(self): return "NATIVE_POINT"
        def children(self): yield "original-x", self.value["x"]
    def existing(value):
        return ExistingPrinter(value) if str(value.type) == "dbg_visualizer::Point" else None
    gdb.pretty_printers.append(existing)
    assert "NATIVE_POINT" in gdb.execute("print point", to_string=True)
    assert session.command("print bytes") == 'b"hello world"'
    assert "临时字符串" in session.command("print map")
    assert session.command("print --mode display point") == "(3, 7)"
    assert session.command("print --mode display --alternate point") == "坐标(3, 7)"
    assert session.command("print --buffer 5 bytes") == 'b"hel … <已截断>'
    assert session.command("print --mode display --buffer 2 --alternate point") == ' … <已截断>'
    assert session.command("print empty") == 'b""'
    assert session.command("print sliced") == 'b"world"'
    point = value("point")
    temporary = gdb.Value(backend.read(int(point.address), point.type.sizeof), point.type)
    before = session.calls
    try:
        session.summary(temporary, {"summary.mode": "debug"})
        raise AssertionError("host temporary must not be passed to Rust")
    except bridge.VisualizerError as error:
        assert "target memory" in str(error), str(error)
    assert session.calls == before
    for expression, expected in [("print --mode display bytes", "UNSUPPORTED"),
                                  ("print --buffer 999999 bytes", "buffer size"),
                                  ("page --start -1 bytes", "start >= 0")]:
        try:
            session.command(expression)
            raise AssertionError("expected error: " + expression)
        except bridge.VisualizerError as error:
            assert expected in str(error), str(error)
    before = session.calls
    gdb.execute("print bytes", to_string=True)
    assert session.calls == before, "native must not execute target code"
    session.command("config summary.mode debug")
    gdb.execute("print bytes", to_string=True)
    assert session.calls == before, "manual view must not execute target code"
    session.command("config execution automatic")
    assert 'b"hello world"' in gdb.execute("print bytes", to_string=True)
    original_children = gdb.execute("print point", to_string=True)
    assert "Point { x: 3, y: 7 }" in original_children and "original-x" in original_children, original_children
    session.command("config children.mode structured")
    session.command("config children.page_size 2")
    session.command("config children.max_items 3")
    rendered = gdb.execute("print map", to_string=True)
    assert '[1] = "one"' in rendered and '[3] = "three"' in rendered, rendered
    result = session.page(value("sliced"), 1, 2)
    assert result["total"] == 5
    assert [int(backend.child_value(c)) for c in result["children"]] == [111, 114]
    before_pages = session.operations[3]
    result = session.page(value("large"), 100, 2)
    assert len(result["children"]) == 2 and result["total"] == 10_000
    assert session.operations[3] - before_pages == 1
    fields = session.page(value("record"), 0, 3)["children"]
    assert [c["name"] for c in fields] == ["point", "bytes", "next"]
    assert int(backend.child_value(fields[2])) == int(value("record").address)
    with gdb.with_parameter("print max-depth", 3):
        assert len(gdb.execute("print record", to_string=True)) < 10_000
    assert session.summary(backend.child_value(fields[0])) == 'Point { x: 4, y: 8 }'
    session.command('config --type dbg_visualizer::Point summary.mode display')
    assert "(3, 7)" in gdb.execute("print point", to_string=True)
    session.command("config summary.mode native")
    session.command("config children.mode native")
    before = session.calls
    gdb.execute("print bytes", to_string=True)
    assert session.calls == before
    session.command('config --type dbg_visualizer::Point summary.mode native')
    assert "NATIVE_POINT" in gdb.execute("print point", to_string=True)
    gdb.pretty_printers.remove(existing)
    # Restore fresh values after the program modifies and reallocates its containers.
    gdb.execute("continue", to_string=True)
    gdb.newest_frame().older().select()
    text = session.command("print map")
    assert '1: "one"' not in text and '4: "four"' in text, text
    page = session.command("page --count 3 map")
    assert '[0].key: 2' in page and '[2].key: 4' in page, page
    # Same-stop explicit writes are observed without stale value/summary caching.
    gdb.execute("set var point.x = 19")
    assert session.command("print --mode display point") == "(19, 7)"
    # Reload the source adapter, preserving exactly one registered printer/command.
    gdb.execute("source debugger/gdb/dbgvis_gdb.py")
    import dbgvis_gdb as reloaded
    assert reloaded._session.command("print bytes") == 'b"hello world"'
    assert sum(getattr(p, "__module__", None) == "dbgvis_gdb" for p in gdb.pretty_printers) == 1
    with gdb.with_parameter("confirm", False):
        gdb.execute("run", to_string=True)
        gdb.newest_frame().older().select()
        assert '1: "one"' in reloaded._session.command("print map")
        gdb.execute("kill", to_string=True)
        gdb.execute("file target/debug/examples/faults", to_string=True)
        gdb.execute("break faults::checkpoint", to_string=True)
        gdb.execute("run", to_string=True)
        gdb.newest_frame().older().select()
        import dbgvis_gdb as alternate
        assert "faults::Fault" in alternate._session.command("types")
        assert "bytes::bytes::Bytes" not in alternate._session.command("types")
        assert alternate._session.command("print --mode display fault") == "healthy"
    print("GDB_INTEGRATION_OK")


try:
    check()
except BaseException:
    import traceback
    traceback.print_exc()
    gdb.execute("quit 1")
