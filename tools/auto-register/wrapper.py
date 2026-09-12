#!/usr/bin/env python3
"""Experimental RUSTC_WORKSPACE_WRAPPER; opt in one --crate-name at a time."""
import hashlib
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[2]
DRIVER = ROOT / "target/auto-register/driver"
EXPECTED = "rustc 1.99.0-nightly (12c36e253 2026-08-10)"


def checked_output(args):
    return subprocess.check_output(args, text=True, close_fds=False).strip()


def option(args, key):
    for index, arg in enumerate(args):
        if arg == key:
            return args[index + 1]
        if arg.startswith(key + "="):
            return arg.split("=", 1)[1]
    return None


def scan_args(args, directory):
    result = []
    i = 0
    while i < len(args):
        arg = args[i]
        if arg in ("--out-dir", "-o", "--emit"):
            i += 2
            continue
        if any(arg.startswith(k + "=") for k in ("--out-dir", "--emit")):
            i += 1
            continue
        if arg == "-C" and args[i + 1].startswith("incremental="):
            i += 2
            continue
        if arg.startswith("-Cincremental="):
            i += 1
            continue
        result.append(arg)
        i += 1
    return result + ["--out-dir", str(directory), "--emit=metadata"]


def scan_directory(selected):
    """Where this session records the workspace libraries whose MIR was retained."""
    return ROOT / "target/auto-register/scan" / selected


def main():
    rustc, *args = sys.argv[1:]
    selected = os.environ.get("DBGVIS_AUTO_CRATE")
    if not selected:
        return subprocess.call([rustc, *args], close_fds=False)
    name = option(args, "--crate-name")
    crate_type = option(args, "--crate-type")
    # Scanning dependency crates is opt-in: DBGVIS_SCAN_DEPS=1 also reads the named
    # variables of the workspace libraries. It is off by default because it is not
    # free -- it keeps the libraries' MIR with `-Zalways-encode-mir`, and every local
    # it discovers becomes a registered root, so a container local whose element has
    # no formatting trait now fails the build from library code the debugging session
    # may never look at. Off, the scan stays inside the executable's own functions.
    scan_deps = os.environ.get("DBGVIS_SCAN_DEPS", "") not in ("", "0")

    if name != selected or crate_type != "bin":
        # Every workspace crate that is not the selected executable. A non-generic
        # function is codegened in its own crate, so its locals are invisible to the
        # executable's scan unless the MIR survives; keep it and record the crate name
        # for the driver. Library targets only: build scripts and proc macros are not
        # linked into the executable, so their types can never be registered there.
        if scan_deps and name and crate_type not in ("bin", "proc-macro"):
            directory = scan_directory(selected)
            directory.mkdir(parents=True, exist_ok=True)
            (directory / name).write_text("")
            if "-Zalways-encode-mir" not in args:
                args = args + ["-Zalways-encode-mir"]
        if name == selected:
            # A package holding both src/lib.rs and src/main.rs gives both targets the
            # same crate name, and Cargo builds the library first. Only the leaf
            # executable can carry registrations -- `enable!()` and
            # `.debug_gdb_scripts` both live there -- so pass the library through and
            # wait for the bin invocation. Announce it, so a crate name that never
            # reaches a bin target does not just fail silently.
            print(f"dbgvis auto: passing through {selected} (--crate-type {crate_type});"
                  " only the bin/example target is instrumented", file=sys.stderr)
        return subprocess.call([rustc, *args], close_fds=False)
    if not DRIVER.exists():
        sys.exit("Build the experiment first: cargo xtask driver")
    actual = checked_output([rustc, "--version"])
    if actual != EXPECTED:
        sys.exit(f"Unsupported driver toolchain: {actual}; expected {EXPECTED}")
    # The driver must be newer than every input that defines it: this wrapper, the
    # driver source, and the xtask gate that pins the toolchain and build flags.
    newest = max(p.stat().st_mtime_ns for p in (Path(__file__), Path(__file__).with_name("driver.rs"),
                                                ROOT / "xtask/src/autoregister.rs"))
    if DRIVER.stat().st_mtime_ns < newest:
        sys.exit("Driver is stale; run cargo xtask driver")
    sysroot = checked_output([rustc, "--print", "sysroot"])
    args += ["--sysroot", sysroot] if option(args, "--sysroot") is None else []
    digest = hashlib.sha256((str(Path.cwd()) + repr(args)).encode()).hexdigest()[:20]
    directory = ROOT / "target/auto-register/plans" / f"{selected}-{digest}"
    directory.mkdir(parents=True, exist_ok=True)
    plan = directory / "register.rs"
    env = os.environ.copy()
    env.pop("DBGVIS_INJECT", None)
    env["DBGVIS_PLAN"] = str(plan)
    env["DBGVIS_TOOL_DIR"] = str(Path(__file__).parent)
    # Names recorded while the workspace libraries were built. A name the executable
    # does not actually link matches no crate and is ignored, so a stale entry left by
    # an earlier session cannot widen the scan.
    recorded = scan_directory(selected)
    env["DBGVIS_SCAN_CRATES"] = ",".join(
        sorted(p.name for p in recorded.iterdir()) if scan_deps and recorded.is_dir() else []
    )
    with (directory / "invocations.log").open("a") as log:
        log.write(f"{time.time_ns()} {os.getpid()}\n")
    print(f"dbgvis auto: scan {selected} -> {plan}", file=sys.stderr)
    with tempfile.TemporaryDirectory(prefix="scan-", dir=directory) as scan:
        code = subprocess.call([str(DRIVER), *scan_args(args, Path(scan))], env=env, close_fds=False)
    if code:
        return code
    env["DBGVIS_INJECT"] = "1"
    print(f"dbgvis auto: compile {selected} with generated registrations (strict)", file=sys.stderr)
    return subprocess.call([str(DRIVER), *args], env=env, close_fds=False)


if __name__ == "__main__":
    sys.exit(main())
