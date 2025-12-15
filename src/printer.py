import gdb

tbl = {
    "regex::regex::string::Captures": "debug_print_regex_captures",
    "indexmap::map::IndexMap<i32, &str, std::hash::random::RandomState>": "debug_print_indexmap_i32_str",
    "bytes::bytes::Bytes": "debug_print_bytes",
}

class GenericPrettyPrinter:
    def __init__(self, val, method):
        self.val = val
        self.method = method

    def to_string(self):
        try:
            ptr = self.val.address
            
            if ptr is None:
                return "<error: value not in memory (address is None)>"

            cstr = gdb.parse_and_eval(f"{self.method}({self.val.address})")
            cstr_val = cstr.string()
            gdb.execute("call debug_print_free(%s)" % cstr)
            
            return cstr_val
            
        except Exception as e:
            return f"<error: {e}>"
        
def lookup(val):
    lookup_tag = val.type.tag
    if lookup_tag is None:
        return None
    if lookup_tag in tbl:
        return GenericPrettyPrinter(val, tbl[lookup_tag])

    return None

gdb.current_objfile().pretty_printers.append(lookup)

