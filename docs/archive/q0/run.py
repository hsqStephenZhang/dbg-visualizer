"""Reproduce Q0's implementation gates; this does not certify a working v2 runtime.

Default: stable language counterexamples. --nightly enables an isolated experiment.
--debuggers requires permission to debug child processes; never attaches to existing ones.
"""
import argparse
import os
from pathlib import Path
import signal
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[3]  # docs/archive/q0/ -> repo root
SOURCES = Path(__file__).resolve().parent


def run(args, *, expect_success=True, marker=None):
    process = subprocess.Popen(
        [str(arg) for arg in args], cwd=ROOT, stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT, text=True, start_new_session=True,
    )
    try:
        output = process.communicate(timeout=45)[0]
    except subprocess.TimeoutExpired:
        os.killpg(process.pid, signal.SIGKILL)
        output = process.communicate()[0]
        raise AssertionError(f"timeout: {args}\n{output}")
    if (process.returncode == 0) != expect_success or (marker and marker not in output):
        raise AssertionError(f"unexpected result ({process.returncode}): {args}\n{output}")
    print(output.strip() if len(output) < 400 else (marker or "COMMAND_OK"), flush=True)
    return output


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--nightly", action="store_true")
    parser.add_argument("--debuggers", action="store_true")
    options = parser.parse_args()
    run(["rustc", "+stable", "--version"])
    with tempfile.TemporaryDirectory(prefix="dbgvis-q0-") as directory:
        out = Path(directory)
        stable = ["rustc", "+stable", "--edition=2024"]
        run(stable + [SOURCES / "autoref.rs", "-o", out / "autoref"])
        run([out / "autoref"], marker="AUTOREF_GENERIC_PRIORITY_LOST")
        run(stable + ["--cfg=unbounded", SOURCES / "autoref.rs", "-o", out / "unbounded"],
            expect_success=False, marker="error[E0599]")
        run(stable + [SOURCES / "overlap.rs", "-o", out / "overlap"],
            expect_success=False, marker="error[E0119]")
        print("STABLE_COUNTEREXAMPLES_REPRODUCED", flush=True)

        if options.nightly:
            run(["rustc", "+nightly", "--version"])
            nightly = ["rustc", "+nightly", "--edition=2024"]
            run(stable + ["--crate-type=rlib", SOURCES / "dispatch.rs", "-o", out / "stable.rlib"],
                expect_success=False, marker="error[E0554]")
            for profile, flags in (("debug", []), ("lto", ["-Copt-level=3", "-Clto=fat", "-Cembed-bitcode=yes"])):
                library = out / f"libq0_{profile}.rlib"
                compiler = nightly + flags
                run(compiler + ["--crate-name=q0_dispatch", "--crate-type=rlib",
                                SOURCES / "dispatch.rs", "-o", library])
                consumer = compiler + [SOURCES / "consumer.rs", "--extern", f"q0_dispatch={library}"]
                run(consumer + ["-o", out / profile])
                run([out / profile], marker="NIGHTLY_PRIORITY_GENERIC_CROSS_CRATE_OK")
                run(consumer + ["--cfg=missing", "-o", out / f"missing_{profile}"],
                    expect_success=False, marker="error[E0080]")
                print(f"NIGHTLY_MISSING_CAPABILITY_BUILD_REJECTED_{profile}", flush=True)
                # Const validation is a code-generation check, not a trait-checking error.
                # Record this limitation rather than claiming cargo check would reject it.
                run(consumer + ["--cfg=missing", "--emit=metadata", "-o", out / f"missing_{profile}.rmeta"])
                print(f"NIGHTLY_METADATA_ONLY_DOES_NOT_REJECT_{profile}", flush=True)

        if options.debuggers:
            for profile, flags in (("debug", []), ("lto", ["-Copt-level=3", "-Clto=fat"])):
                binary = out / f"anchor_{profile}"
                run(stable + flags + ["-g", "--crate-name=q0_anchor", SOURCES / "anchor.rs", "-o", binary])
                run(["gdb", "-q", "-nx", "--batch", binary, "-ex", "set debuginfod enabled off",
                     "-x", SOURCES / "anchor_gdb.py"], marker="GDB_TYPED_ANCHOR_OK")
                lldb_output = run(["lldb", "--no-lldbinit", "--batch", binary,
                     "-o", f"command script import {SOURCES / 'anchor_lldb.py'}",
                     "-o", "q0-check"], marker="LLDB_TYPED_ANCHOR_OK")
                for line in lldb_output.splitlines():
                    if line.startswith("LLDB_CONST_GENERIC_"):
                        print(line, flush=True)
        print("Q0_PROBES_COMPLETE_NOT_V2_ACCEPTANCE", flush=True)


if __name__ == "__main__":
    main()
