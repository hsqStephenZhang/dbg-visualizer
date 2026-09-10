import lldb
import dbgvis_lldb as bridge


def check():
    debugger = lldb.debugger
    target = debugger.GetSelectedTarget()
    process = target.GetProcess()
    process.GetSelectedThread().SetSelectedFrame(1)
    session = bridge.get_session(debugger)
    backend = session.backend
    value = backend.find_value
    assert session.calls == 0
    scalar = backend.child_value({"kind": 1, "data": 2**64 - 1, "name": "computed"})
    assert scalar.GetValueAsUnsigned() == 2**64 - 1, (scalar.GetError(), str(scalar), scalar.GetType().GetName())
    native_category = debugger.CreateCategory("dbgvis-native-test")
    native_category.AddTypeSummary(lldb.SBTypeNameSpecifier("dbg_visualizer::Point", False),
                                   lldb.SBTypeSummary.CreateWithSummaryString("NATIVE_POINT"))
    native_category.SetEnabled(True)
    assert value("point").GetSummary() == "NATIVE_POINT"
    assert session.command("print bytes") == 'b"hello world"'
    assert "临时字符串" in session.command("print map")
    assert session.command("print --mode display point") == "(3, 7)"
    assert session.command("print --mode display --alternate point") == "坐标(3, 7)"
    assert session.command("print --buffer 5 bytes") == 'b"hel … <已截断>'
    assert session.command("print empty") == 'b""'
    assert session.command("print sliced") == 'b"world"'
    point = value("point")
    temporary = target.CreateValueFromData("temporary", point.GetData(), point.GetType())
    before = session.calls
    try:
        session.summary(temporary, {"summary.mode": "debug"})
        raise AssertionError("host temporary must not be passed to Rust")
    except bridge.VisualizerError as error:
        assert "target memory" in str(error), str(error)
    assert session.calls == before
    before = session.calls
    backend.native(value("bytes"))
    assert session.calls == before
    session.command("config summary.mode debug")
    backend.native(value("bytes"))
    assert session.calls == before
    session.command("config execution automatic")
    session.command("config children.mode structured")
    session.command("config children.page_size 2")
    session.command("config children.max_items 3")
    raw = value("map")
    synthetic = raw.GetSyntheticValue()
    assert synthetic.GetNumChildren() == 6, str(synthetic)
    assert synthetic.GetChildAtIndex(0).GetValueAsSigned() == 1
    assert synthetic.GetChildAtIndex(2).GetValueAsSigned() == 2
    assert synthetic.GetChildAtIndex(4).GetValueAsSigned() == 3
    assert raw.GetSummary() == '{1: "one", 2: "临时字符串", 3: "three"}'
    page = session.page(value("sliced"), 1, 2)
    assert [backend.child_value(c).GetValueAsUnsigned() for c in page["children"]] == [111, 114]
    before_pages = session.operations[3]
    page = session.page(value("large"), 100, 2)
    assert page["total"] == 10_000 and len(page["children"]) == 2
    assert session.operations[3] - before_pages == 1
    provider = bridge.SyntheticChildren(value("large"), {})
    assert provider.num_children() == 4  # three configured items plus an omission node
    omission = provider.get_child_at_index(3)
    assert "9997 more items" in (omission.GetSummary() or str(omission))
    length = value("large").GetChildMemberWithName("len")
    assert length.SetValueFromCString("2")
    assert provider.num_children() == 2
    assert length.SetValueFromCString("10000")
    assert provider.num_children() == 4
    fields = session.page(value("record"), 0, 3)["children"]
    assert [c["name"] for c in fields] == ["point", "bytes", "next"]
    assert backend.child_value(fields[2]).GetValueAsUnsigned() == backend.value_address(value("record"))
    output = lldb.SBCommandReturnObject()
    context = lldb.SBExecutionContext(process.GetSelectedThread().GetFrameAtIndex(1))
    debugger.GetCommandInterpreter().HandleCommand("frame variable -D 3 record", context, output)
    assert output.Succeeded() and len(output.GetOutput()) < 10_000, (output.GetError(), output.GetOutput())
    assert session.summary(backend.child_value(fields[0])) == 'Point { x: 4, y: 8 }'
    session.command('config --type dbg_visualizer::Point summary.mode display')
    assert value("point").GetSummary() == "(3, 7)"
    session.command("reset")
    assert value("point").GetSummary() == "NATIVE_POINT"
    debugger.DeleteCategory("dbgvis-native-test")
    before = session.calls
    backend.native(value("bytes"))
    assert session.calls == before
    process.Continue()
    process.GetSelectedThread().SetSelectedFrame(1)
    text = session.command("print map")
    assert '1: "one"' not in text and '4: "four"' in text, text
    page = session.command("page --count 3 map")
    assert '[0].key: 2' in page and '[2].key: 4' in page, page
    point_x = value("point").GetChildMemberWithName("x")
    assert point_x.SetValueFromCString("19")
    assert session.command("print --mode display point") == "(19, 7)"
    debugger.HandleCommand("command script import debugger/lldb/dbgvis_lldb.py")
    assert bridge.get_session(debugger).command("print bytes") == 'b"hello world"'
    assert process.Kill().Success()
    process = target.LaunchSimple(None, None, str(__import__('pathlib').Path.cwd()))
    process.GetSelectedThread().SetSelectedFrame(1)
    assert '1: "one"' in bridge.get_session(debugger).command("print map")
    assert process.Kill().Success()
    alternate = debugger.CreateTarget("target/debug/examples/faults")
    debugger.SetSelectedTarget(alternate)
    alternate.BreakpointCreateByName("faults::checkpoint")
    process = alternate.LaunchSimple(None, None, str(__import__('pathlib').Path.cwd()))
    process.GetSelectedThread().SetSelectedFrame(1)
    current = bridge.get_session(debugger)
    assert "faults::Fault" in current.command("types")
    assert "bytes::bytes::Bytes" not in current.command("types")
    assert current.command("print --mode display fault") == "healthy"
    print("LLDB_INTEGRATION_OK")


check()
