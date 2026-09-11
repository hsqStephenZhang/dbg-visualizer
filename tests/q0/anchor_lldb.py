import lldb


def check(debugger, command, result, internal_dict):
    target = debugger.GetSelectedTarget()
    target.BreakpointCreateByName("q0_anchor::checkpoint")
    debugger.HandleCommand("settings set target.disable-aslr false")
    debugger.HandleCommand("run")
    process = target.GetProcess()
    assert process.GetState() == lldb.eStateStopped
    frame = process.GetSelectedThread().GetFrameAtIndex(1)
    types = {}
    rejected = []
    for name, variable in (("MAP", "map"), ("CUSTOM", "custom"),
                           ("BORROWED", "borrowed"), ("ZST", "zst"),
                           ("BORROWED_OTHER", "borrowed_other"),
                           ("LEFT", "left"), ("RIGHT", "right")):
        anchor = target.FindFirstGlobalVariable("q0_anchor::ANCHOR_" + name)
        assert anchor.IsValid(), name
        target_type = anchor.GetChildMemberWithName("typed").GetType().GetPointeeType().GetCanonicalType()
        value = frame.FindVariable(variable)
        assert value.IsValid(), variable
        value_type = value.GetType().GetCanonicalType()
        if not target_type.IsValid() or not value_type.IsValid() or target_type != value_type:
            # Diagnostic probe: retaining the counterexample is intentional. It is NOT
            # evidence this registration works. A real dispatcher must reject it.
            assert name in ("BORROWED", "BORROWED_OTHER"), name
            rejected.append(name)
            print("LLDB_CONST_GENERIC_UNAVAILABLE_MUST_REJECT", name,
                  repr(target_type.GetName()), repr(value_type.GetName()))
            continue
        types[name] = target_type
        print("ANCHOR", name, target_type.GetName())
    assert types["LEFT"] != types["RIGHT"]
    if rejected:
        print("LLDB_CONST_GENERIC_REGISTRATIONS_REJECTED", rejected)
    elif types["BORROWED"] == types["BORROWED_OTHER"]:
        matches = [name for name, typ in types.items() if typ == types["BORROWED"]]
        assert set(matches) == {"BORROWED", "BORROWED_OTHER"}, matches
        print("LLDB_CONST_GENERIC_AMBIGUOUS_MUST_REJECT", matches)
    else:
        print("LLDB_CONST_GENERIC_DISTINCT")
    print("LLDB_TYPED_ANCHOR_OK")


def __lldb_init_module(debugger, internal_dict):
    debugger.HandleCommand("command script add -f anchor_lldb.check q0-check")
