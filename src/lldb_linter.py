import lldb

tbl = {
    "regex::regex::string::Captures": "debug_print_regex_captures",
    "indexmap::map::IndexMap<int, &str, std::hash::random::RandomState>": "debug_print_indexmap_i32_str",
    "bytes::bytes::Bytes": "debug_print_bytes",
}


def generic_summary_provider(valobj: lldb.value, internal_dict):
    typename = valobj.GetTypeName()

    if typename not in tbl:
        return "not in tbl"

    fn = tbl[typename]
    addr = valobj.GetAddress().GetLoadAddress(valobj.GetTarget())

    try:
        expr = f"{fn}({addr})"
        result = valobj.GetTarget().EvaluateExpression(expr)

        if not result.IsValid() or result.GetError().Fail():
            err = result.GetError().GetCString()
            return f"<expression failed: {err}>"

        cstr = result.GetSummary() 
        if cstr is None:
            cstr = result.GetValue()
            
        if cstr is None:
            return "<error: result is None>"

        return cstr

    except Exception as e:
        return f"<error: {e}>"

def __lldb_init_module(debugger, internal_dict):
    for rust_type in tbl.keys():
        debugger.HandleCommand(
            f'type summary add "{rust_type}" --python-function lldb_linter.generic_summary_provider'
        )
