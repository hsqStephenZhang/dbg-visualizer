"""Run from any directory: python3 tests/integration/run.py. Requires local ptrace access."""
import argparse
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[2]


def run(command, marker=None, timeout=45, env=None, cwd=ROOT):
    process = subprocess.Popen(command, cwd=cwd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                               text=True, env=env, start_new_session=True)
    try:
        output, _ = process.communicate(timeout=timeout)
    except subprocess.TimeoutExpired:
        # Only this freshly created test process group is terminated, including its inferior.
        os.killpg(process.pid, signal.SIGKILL)
        output, _ = process.communicate()
        print(output)
        raise
    if process.returncode or (marker and marker not in output):
        print(output)
        raise RuntimeError(f"failed: {command}")
    print(marker or "PASS " + " ".join(command[:3]))
    return output


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--backend", choices=("gdb", "lldb", "all"), default="all")
    parser.add_argument("--faults", action="store_true")
    parser.add_argument("--core", action="store_true")
    parser.add_argument("--benchmark", action="store_true")
    args = parser.parse_args()
    run(["cargo", "build", "--offline", "--examples", "--bin", "dbg-visualizer"])
    if args.backend in ("gdb", "all"):
        run(["gdb", "-q", "-nx", "-batch", "-iex", f"add-auto-load-safe-path {ROOT}",
             "target/debug/dbg-visualizer", "-ex", "set debuginfod enabled off",
             "-ex", "break dbg_visualizer::checkpoint", "-ex", "run",
             "-ex", "source tests/integration/gdb_checks.py"], "GDB_INTEGRATION_OK")
        with tempfile.TemporaryDirectory(prefix="dbgvis-relocated-") as temporary:
            binary = Path(temporary) / "standalone"
            shutil.copy2(ROOT / "target/debug/dbg-visualizer", binary)
            run(["gdb", "-q", "-nx", "-batch", "-iex", f"add-auto-load-safe-path {binary}",
                 str(binary), "-ex", "set debuginfod enabled off", "-ex", "break dbg_visualizer::checkpoint",
                 "-ex", "run", "-ex", "up", "-ex", "dbgvis print bytes"], 'b"hello world"', cwd=temporary)
            print("GDB_STANDALONE_BUNDLE_OK")
    if args.backend in ("lldb", "all"):
        run(["lldb", "--no-lldbinit", "--batch", "target/debug/dbg-visualizer",
             "-o", "command script import debugger/lldb/dbgvis_lldb.py", "-o", "dbgvis-install-hook",
             "-o", "breakpoint set -n dbg_visualizer::checkpoint", "-o", "run",
             "-o", "script import runpy; runpy.run_path('tests/integration/lldb_checks.py')"], "LLDB_INTEGRATION_OK")
    if args.faults:
        run(["cargo", "build", "--offline", "--profile", "abort", "--example", "faults"])
        for case in ("panic", "error", "hang", "exit", "abort"):
            env = dict(os.environ, DBGVIS_FAULT_CASE=case)
            mode = "panic" if case == "abort" else case
            binary = "target/abort/examples/faults" if case == "abort" else "target/debug/examples/faults"
            if args.backend in ("gdb", "all"):
                run(["gdb", "-q", "-nx", "-batch", "-iex", f"add-auto-load-safe-path {ROOT}", binary,
                     "-ex", "set debuginfod enabled off", "-ex", "break faults::checkpoint",
                     "-ex", "set args " + mode, "-ex", "run", "-ex", "source tests/integration/fault_gdb.py"],
                    "GDB_FAULT_OK_" + case, env=env)
            if args.backend in ("lldb", "all"):
                run(["lldb", "--no-lldbinit", "--batch", binary,
                     "-o", "command script import debugger/lldb/dbgvis_lldb.py",
                     "-o", "breakpoint set -n faults::checkpoint", "-o", "settings set target.run-args " + mode,
                     "-o", "run", "-o", "script import runpy; runpy.run_path('tests/integration/fault_lldb.py')"],
                    "LLDB_FAULT_OK_" + case, env=env)
    if args.core:
        # TemporaryDirectory owns only these freshly generated artifacts, and removes them on exit.
        with tempfile.TemporaryDirectory(prefix="dbgvis-core-") as temporary:
            core = str(Path(temporary) / "example.core")
            run(["gdb", "-q", "-nx", "-batch", "-iex", "set auto-load python-scripts off",
                 "target/debug/dbg-visualizer", "-ex", "set debuginfod enabled off",
                 "-ex", "break dbg_visualizer::checkpoint", "-ex", "run", "-ex", "generate-core-file " + core])
            if args.backend in ("gdb", "all"):
                output = run(["gdb", "-q", "-nx", "-batch", "-iex", f"add-auto-load-safe-path {ROOT}",
                              "target/debug/dbg-visualizer", "--core", core, "-ex", "up",
                              "-ex", "dbgvis print --mode native bytes", "-ex", "dbgvis print bytes",
                              "-ex", "dbgvis status"])
                assert "core dump" in output and '"calls": 0' in output, output
            if args.backend in ("lldb", "all"):
                output = run(["lldb", "--no-lldbinit", "--batch", "target/debug/dbg-visualizer", "--core", core,
                              "-o", "command script import debugger/lldb/dbgvis_lldb.py", "-o", "up",
                              "-o", "dbgvis print --mode native bytes",
                              "-o", "script s=dbgvis_lldb.get_session(lldb.debugger); exec(\"try:\\n s.command('print bytes')\\nexcept Exception as e:\\n print(e)\"); print(s.command('status'))"])
                assert "core dump" in output and '"calls": 0' in output, output
            print("CORE_FALLBACK_OK")
    if args.benchmark:
        commands = {
            "gdb": ["gdb", "-q", "-nx", "-batch", "-iex", f"add-auto-load-safe-path {ROOT}",
                    "target/debug/dbg-visualizer", "-ex", "set debuginfod enabled off", "-ex", "break dbg_visualizer::checkpoint",
                    "-ex", "run", "-ex", "source tests/integration/benchmark_gdb.py"],
            "lldb": ["lldb", "--no-lldbinit", "--batch", "target/debug/dbg-visualizer",
                     "-o", "command script import debugger/lldb/dbgvis_lldb.py", "-o", "breakpoint set -n dbg_visualizer::checkpoint",
                     "-o", "run", "-o", "script import runpy; runpy.run_path('tests/integration/benchmark_lldb.py')"],
        }
        for name, command in commands.items():
            if args.backend not in (name, "all"): continue
            prefix = "BENCHMARK_" + name.upper() + "="
            output = run(command, prefix)
            print(next(line for line in output.splitlines() if line.startswith(prefix)))


if __name__ == "__main__":
    main()
