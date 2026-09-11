import gdb

gdb.execute("set pagination off")
gdb.execute("break q0_anchor::checkpoint")
gdb.execute("run")
gdb.execute("up")
types = {}
for name, variable in (("MAP", "map"), ("CUSTOM", "custom"),
                       ("BORROWED", "borrowed"), ("ZST", "zst"),
                       ("BORROWED_OTHER", "borrowed_other"),
                       ("LEFT", "left"), ("RIGHT", "right")):
    symbol = (gdb.lookup_global_symbol("q0_anchor::ANCHOR_" + name)
              or gdb.lookup_static_symbol("q0_anchor::ANCHOR_" + name))
    assert symbol is not None, name
    anchor_type = symbol.type.strip_typedefs()
    field = next(f for f in anchor_type.fields() if f.name == "typed")
    target_type = field.type.target().strip_typedefs()
    value_type = gdb.parse_and_eval(variable).type.strip_typedefs()
    assert target_type == value_type, (name, str(target_type), str(value_type))
    types[name] = target_type
    print("ANCHOR", name, str(target_type))
assert types["BORROWED"] != types["BORROWED_OTHER"]
assert types["LEFT"] != types["RIGHT"]
print("GDB_CONST_GENERIC_AND_SAME_LAYOUT_DISTINCT")
print("GDB_TYPED_ANCHOR_OK")
