import gdb
import json
import time
import dbgvis_gdb as bridge

gdb.newest_frame().older().select()
session = bridge._session
backend = session.backend
results = {}
for name, count, action in [
    ("short_summary", 100, lambda i: session.summary(backend.find_value("point"), {"summary.mode": "debug"})),
    ("distinct_variables", 128, lambda i: session.summary(backend.find_value(f"points[{i}]"), {"summary.mode": "debug"})),
    ("large_container_page", 10, lambda i: session.page(backend.find_value("large"), i * 64, 64)),
]:
    before = session.calls
    start = time.perf_counter()
    for i in range(count): action(i)
    results[name] = {"iterations": count, "calls": session.calls - before,
                     "elapsed_ms": round((time.perf_counter() - start) * 1000, 3)}
print("BENCHMARK_GDB=" + json.dumps(results))
