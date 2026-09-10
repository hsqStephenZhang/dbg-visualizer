import lldb
import json
import time
import dbgvis_lldb as bridge

debugger = lldb.debugger
debugger.GetSelectedTarget().GetProcess().GetSelectedThread().SetSelectedFrame(1)
session = bridge.get_session(debugger)
backend = session.backend
results = {}
for name, count, action in [
    ("short_summary", 100, lambda i: session.summary(backend.find_value("point"), {"summary.mode": "debug"})),
    ("distinct_variables", 128, lambda i: session.summary(backend.find_value("points").GetChildAtIndex(i), {"summary.mode": "debug"})),
    ("large_container_page", 10, lambda i: session.page(backend.find_value("large"), i * 64, 64)),
]:
    before = session.calls
    start = time.perf_counter()
    for i in range(count): action(i)
    results[name] = {"iterations": count, "calls": session.calls - before,
                     "elapsed_ms": round((time.perf_counter() - start) * 1000, 3)}
# Check that requesting adjacent actual synthetic children reuses the fetched page.
session.command("config execution automatic")
session.command("config children.mode structured")
provider = bridge.SyntheticChildren(backend.find_value("large"), {})
before_pages = session.operations[3]
for i in range(64): assert provider.get_child_at_index(i).GetValueAsUnsigned() == ord('x')
assert session.operations[3] - before_pages == 1, session.operations
results["synthetic_first_page"] = {"children": 64, "page_calls": session.operations[3] - before_pages}
print("BENCHMARK_LLDB=" + json.dumps(results))
