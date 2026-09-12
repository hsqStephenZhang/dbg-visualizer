"""Run actual GDB v2 scenarios in disposable process groups with external timeouts."""
import argparse
import os
from pathlib import Path
import signal
import subprocess
import shutil
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[2]


def run(args, marker=None, success=True):
    process = subprocess.Popen([str(a) for a in args], cwd=ROOT, stdout=subprocess.PIPE,
                               stderr=subprocess.STDOUT, text=True, start_new_session=True)
    try:
        output = process.communicate(timeout=45)[0]
    except subprocess.TimeoutExpired:
        os.killpg(process.pid, signal.SIGKILL)
        output = process.communicate()[0]
        raise AssertionError(f"TIMEOUT {args}\n{output}")
    if (process.returncode == 0) != success or (marker and marker not in output):
        raise AssertionError(f"FAILED {args}\n{output}")
    print(output, flush=True)
    return output


def debugger(binary, script, args=()):
    return ["rust-gdb", "-q", "-nx", "--batch", "-iex", f"add-auto-load-safe-path {binary}",
            "-x", ROOT / "tests/integration" / script, "--args", binary, *args]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--faults", action="store_true")
    parser.add_argument("--lto", action="store_true")
    parser.add_argument("--relocated", action="store_true")
    options = parser.parse_args()
    if sys.platform != "linux":
        # Apple/MSVC rustc targets never emit `.debug_gdb_scripts`, and GDB cannot ptrace here.
        print("GDB_V2_SUITE_SKIPPED: live GDB auto-load scenarios require Linux", flush=True)
        return
    run(["cargo", "build", "--offline", "--example", "explicit"])
    run(debugger(ROOT / "target/debug/examples/explicit", "gdb_checks.py"), "GDB_V2_INTEGRATION_OK")
    if options.lto:
        run(["cargo", "build", "--offline", "--example", "explicit", "--profile", "release-lto"])
        run(debugger(ROOT / "target/release-lto/examples/explicit", "gdb_checks.py"), "GDB_V2_INTEGRATION_OK")
    if options.relocated:
        with tempfile.TemporaryDirectory(prefix="dbgvis relocated ") as directory:
            binary = Path(directory) / "explicit binary"
            shutil.copy2(ROOT / "target/debug/examples/explicit", binary)
            run(debugger(binary, "gdb_checks.py"), "GDB_V2_INTEGRATION_OK")
    if options.faults:
        run(["cargo", "build", "--offline", "--example", "faults"])
        for kind in ("panic", "error", "hang", "exit"):
            run(debugger(ROOT / "target/debug/examples/faults", "fault_gdb.py", [kind]), "GDB_V2_FAULT_OK")
        run(["cargo", "build", "--offline", "--example", "faults", "--profile", "abort"])
        run(debugger(ROOT / "target/abort/examples/faults", "fault_gdb.py", ["panic"]), "GDB_V2_FAULT_OK")
    print("GDB_V2_SUITE_OK")


if __name__ == "__main__":
    main()
