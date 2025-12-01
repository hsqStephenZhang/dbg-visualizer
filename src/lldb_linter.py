import lldb

debug_free_func = "debug_print_free"

tbl = {
    "regex::regex::string::Captures": "debug_print_regex_captures",
    "indexmap::map::IndexMap<int, &str, std::hash::random::RandomState>": "debug_print_indexmap_i32_str",
    "bytes::bytes::Bytes": "debug_print_bytes",
}

# scipts to run in lldb console for

console_script = """
f = lldb.frame
v = f.FindVariable("map")
print(v.GetValueType())
print(v.GetName())
target = v.GetTarget()
addr = v.GetAddress().GetLoadAddress(target)
if addr == lldb.LLDB_INVALID_ADDRESS or addr == 0:
    addr = v.AddressOf().GetValueAsUnsigned()

expr = f"debug_print_indexmap_i32_str({addr})"
result = target.EvaluateExpression(expr)
cstr = result.GetSummary()
print(cstr)
ptr_val = result.GetValueAsUnsigned()
if ptr_val != 0:
    error = lldb.SBError()
    cstr_data = target.GetProcess().ReadCStringFromMemory(ptr_val, 2048, error)

print(cstr_data)
"""


def get_object_address(valobj: lldb.SBValue):
    """
    Robustly gets the load address of the object, handling pointers, 
    references, and register-stored variables.
    """
    # 1. Handle Pointers (e.g., MyMap*)
    if valobj.TypeIsPointerType():
        # The 'value' of a pointer IS the address we need
        return valobj.GetValueAsUnsigned()

    # 2. Handle Values (e.g., MyMap)
    # Check if the object is in memory (and not a register)
    addr = valobj.GetLoadAddress()

    if addr == lldb.LLDB_INVALID_ADDRESS or addr == 0:
        addr = valobj.AddressOf().GetValueAsUnsigned()

    return addr


def free_rust_cstring(target: lldb.SBTarget, addr: int):
    """
    Calls the Rust function to free a CString allocated in Rust.
    """
    expr = f"{debug_free_func}({addr})"
    try:
        target.EvaluateExpression(expr)
    except Exception:
        pass  # Ignore errors during free

# valobj might be temperal object if user call `p xxx` instead of `v xxx`
def generic_summary_provider(valobj: lldb.SBValue, internal_dict):
    typename = valobj.GetTypeName()

    if typename not in tbl:
        return None

    fn = tbl[typename]
    target = valobj.GetTarget()
    addr = get_object_address(valobj)

    if addr == lldb.LLDB_INVALID_ADDRESS or addr == 0:
        return f"<error: variable({hex(addr)}) is in register (not memory), cannot pass address>"
    elif isinstance(addr, str):
        return addr  # return the error message from get_object_address

    try:
        expr = f"{fn}({addr})"
        result = target.EvaluateExpression(expr)
        ptr_val = result.GetValueAsUnsigned()

        if not result.IsValid() or result.GetError().Fail():
            err = result.GetError().GetCString()
            return f"<expression failed: {err}>"

        cstr = result.GetSummary()
        if cstr is None and ptr_val != 0:
            # read c string from memory
            error = lldb.SBError()
            cstr_data = target.GetProcess().ReadCStringFromMemory(ptr_val, 2048, error)
            if error.Success():
                cstr = cstr_data
            else:
                cstr = f"<error reading string memory: {error.GetCString()}>"

        free_rust_cstring(target, ptr_val)
        return cstr

    except Exception as e:
        return f"<error: {e}>"


def __lldb_init_module(debugger, internal_dict):
    for rust_type in tbl.keys():
        # -x: regex
        # -h: hide None and empty value
        # -e: expand children (which we don't use here)
        debugger.HandleCommand(
            f'type summary add "{rust_type}" -F lldb_linter.generic_summary_provider -x -h "^{rust_type}$" --category Rust' 
        )
