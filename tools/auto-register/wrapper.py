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


def main():
    rustc, *args = sys.argv[1:]
    selected = os.environ.get("DBGVIS_AUTO_CRATE")
    if not selected or option(args, "--crate-name") != selected:
        return subprocess.call([rustc, *args], close_fds=False)
    if option(args, "--crate-type") != "bin":
        sys.exit("The auto-register experiment supports only a selected bin/example, not library builds")
    if not DRIVER.exists():
        sys.exit("Build the experiment first: python3 tools/auto-register/build.py")
    actual = checked_output([rustc, "--version"])
    if actual != EXPECTED:
        sys.exit(f"Unsupported driver toolchain: {actual}; expected {EXPECTED}")
    newest = max(p.stat().st_mtime_ns for p in (Path(__file__), Path(__file__).with_name("driver.rs"), Path(__file__).with_name("build.py")))
    if DRIVER.stat().st_mtime_ns < newest:
        sys.exit("Driver is stale; run python3 tools/auto-register/build.py")
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
