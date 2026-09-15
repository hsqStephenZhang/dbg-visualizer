#!/usr/bin/env python3
"""Experimental RUSTC_WORKSPACE_WRAPPER; opt in one --crate-name at a time."""
import hashlib
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import time

EXPECTED = "rustc 1.99.0-nightly (12c36e253 2026-08-10)"


def _toolchain_help(actual):
    """Why `actual` is not the pinned compiler, and the command that fixes it.

    Separates "a nightly, but the wrong build" (a date mismatch) from "not a
    nightly at all" (a channel mismatch); both need the exact pinned build.
    """
    date = EXPECTED.rsplit(" ", 1)[-1].rstrip(")")
    toolchain = f"nightly-{date}"
    lead = ("this is a nightly build, but not the exact one the driver was built against"
            if "-nightly" in actual
            else "this is not a nightly toolchain; the driver needs a pinned nightly")
    return (f"dbgvis auto: unsupported toolchain -- {lead}.\n"
            f"  expected {EXPECTED}\n"
            f"  got      {actual}\n"
            f"  rustup toolchain install {toolchain} --component rustc-dev\n"
            f"  cargo +{toolchain} dbgvis setup")


def _install():
    """Where the driver, its sources and the scan plans live.

    Installed (`cargo dbgvis setup`): a self-contained DBGVIS_HOME holds the wrapper,
    driver source and the compiled driver together. In the dbgvis repository itself,
    with DBGVIS_HOME unset, fall back to the checkout layout so `cargo xtask` keeps
    working. Returns (driver binary, tool dir with the sources, plans dir).
    """
    home = os.environ.get("DBGVIS_HOME")
    if home:
        home = Path(home)
        return home / "driver", home, home / "plans"
    repo = Path(__file__).resolve().parents[2]
    return (repo / "target/auto-register/driver",
            Path(__file__).resolve().parent,
            repo / "target/auto-register/plans")


DRIVER, TOOL_DIR, PLANS = _install()
# The driver must be newer than every input that defines it: this wrapper and the
# driver source. A vendored install may not ship a source next to the wrapper; the
# staleness check below skips what is absent.
TRACKED = (Path(__file__).resolve(), TOOL_DIR / "driver.rs")


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


def is_probe(args):
    """Whether Cargo is only asking rustc about itself rather than compiling."""
    # The target-info probe does carry a crate name (`--crate-name ___`), so the
    # presence of a name proves nothing. `--print` is the reliable signal: Cargo
    # never passes it for a real compilation, and a probe must reach plain rustc
    # untouched -- otherwise a stale driver fails the build as an unreadable
    # "failed to run `rustc` to learn about target-specific information".
    return any(arg == "--print" or arg.startswith("--print=") for arg in args)


def scan_directory(args, selected):
    """Where this build records the workspace libraries whose MIR was retained.

    Inside Cargo's own target directory, never this repository's: `cargo clean`
    then drops the markers together with the libraries they describe, and two
    projects that happen to share a `DBGVIS_AUTO_CRATE` value cannot collide --
    a name recorded by one project must not make the other scan a same-named
    registry crate. Returns None when the target directory cannot be located,
    which disables dependency scanning rather than guessing.
    """
    out = option(args, "--out-dir")
    if not out:
        return None
    for directory in [Path(out), *Path(out).parents]:
        # Cargo writes CACHEDIR.TAG at the root of every target directory.
        if (directory / "CACHEDIR.TAG").exists():
            return directory / "dbgvis-scan" / selected
    return None


def main():
    rustc, *args = sys.argv[1:]
    selected = os.environ.get("DBGVIS_AUTO_CRATE")
    if not selected:
        return subprocess.call([rustc, *args], close_fds=False)
    name = option(args, "--crate-name")
    crate_type = option(args, "--crate-type")
    # Probes must not require the driver or have their output altered; only actual
    # workspace compilations go through the tracking driver.
    if not name or is_probe(args):
        return subprocess.call([rustc, *args], close_fds=False)
    if not DRIVER.exists():
        sys.exit("driver not built: run `cargo dbgvis setup` (or `cargo xtask driver` in the dbgvis repo)")
    actual = checked_output([rustc, "--version"])
    if actual != EXPECTED:
        sys.exit(_toolchain_help(actual))
    # Track these inputs in every compilation, including passthrough libraries.
    # A vendored wrapper may not ship the whole checkout: skip what is absent
    # rather than raising a traceback on every rustc invocation.
    tracked = [p for p in TRACKED if p.exists()]
    newest = max(p.stat().st_mtime_ns for p in tracked)
    if DRIVER.stat().st_mtime_ns < newest:
        sys.exit("driver is stale: run `cargo dbgvis setup` (or `cargo xtask driver` in the dbgvis repo)")
    sysroot = checked_output([rustc, "--print", "sysroot"])
    args += ["--sysroot", sysroot] if option(args, "--sysroot") is None else []
    env = os.environ.copy()
    env.pop("DBGVIS_INJECT", None)
    env.pop("DBGVIS_PASSTHROUGH", None)
    env["DBGVIS_TOOL_DIR"] = str(TOOL_DIR)
    # Scanning dependency crates is opt-in: DBGVIS_SCAN_DEPS=1 also reads the named
    # variables of the workspace libraries. It is off by default because it is not
    # free -- it keeps the libraries' MIR with `-Zalways-encode-mir`, and every local
    # it discovers becomes a registered root, so a container local whose element has
    # no formatting trait now fails the build from library code the debugging session
    # may never look at. Off, the scan stays inside the executable's own functions.
    scan_deps = os.environ.get("DBGVIS_SCAN_DEPS", "") not in ("", "0")
    # DBGVIS_SCAN_CRATES narrows or widens *which* crates: comma-separated regular
    # expressions, each matched against a whole crate name (the `_` form, so `regex`
    # selects exactly that crate and `regex.*` also takes `regex_syntax`). Setting it
    # implies scanning on. Unset, DBGVIS_SCAN_DEPS=1 scans every workspace library.
    # Only workspace libraries can have their MIR kept; a matched registry crate still
    # yields its `#[inline]`/generic functions, which carry MIR anyway.
    patterns = [p.strip() for p in os.environ.get("DBGVIS_SCAN_CRATES", "").split(",") if p.strip()]
    try:
        compiled = [re.compile(p) for p in patterns]
    except re.error as error:
        sys.exit(f"dbgvis auto: invalid DBGVIS_SCAN_CRATES pattern `{error.pattern}`: {error}")
    scan_deps = scan_deps or bool(compiled)

    if name != selected or crate_type != "bin":
        # Every workspace crate that is not the selected executable. A non-generic
        # function is codegened in its own crate, so its locals are invisible to the
        # executable's scan unless the MIR survives; keep it and record the crate name
        # for the driver. Library targets only: build scripts and proc macros are not
        # linked into the executable, so their types can never be registered there.
        if scan_deps and crate_type not in ("bin", "proc-macro"):
            if compiled:
                keep = any(p.fullmatch(name) for p in compiled)
            else:
                recorded = scan_directory(args, selected)
                keep = recorded is not None
                if keep:
                    recorded.mkdir(parents=True, exist_ok=True)
                    (recorded / name).write_text("")
            if keep and "-Zalways-encode-mir" not in args:
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
        env["DBGVIS_PASSTHROUGH"] = "1"
        return subprocess.call([str(DRIVER), *args], env=env, close_fds=False)
    digest = hashlib.sha256((str(Path.cwd()) + repr(args)).encode()).hexdigest()[:20]
    directory = PLANS / f"{selected}-{digest}"
    directory.mkdir(parents=True, exist_ok=True)
    plan = directory / "register.rs"
    env["DBGVIS_PLAN"] = str(plan)
    # The driver always receives regular expressions, through DBGVIS_SCAN_PATTERNS
    # rather than the user's DBGVIS_SCAN_CRATES: Cargo validates a recorded env-dep
    # against its own environment, so a variable the wrapper rewrites for rustc would
    # never match and every build would be dirty. In pattern mode they are the user's
    # own; otherwise they are the names recorded while this target directory's
    # workspace libraries were built, escaped. A name the executable does not link
    # matches no crate and is ignored.
    if compiled:
        env["DBGVIS_SCAN_PATTERNS"] = ",".join(patterns)
    else:
        recorded = scan_directory(args, selected) if scan_deps else None
        env["DBGVIS_SCAN_PATTERNS"] = ",".join(
            sorted(re.escape(p.name) for p in recorded.iterdir())
            if recorded is not None and recorded.is_dir()
            else []
        )
        if scan_deps and recorded is None:
            print("dbgvis auto: cannot locate Cargo's target directory;"
                  " dependency scanning is disabled for this build", file=sys.stderr)
    # Lets the driver refuse to scan the standard library whatever the patterns say.
    env["DBGVIS_SYSROOT"] = sysroot
    with (directory / "invocations.log").open("a") as log:
        log.write(f"{time.time_ns()} {os.getpid()}\n")
    print(f"dbgvis auto: scan {selected} -> {plan}", file=sys.stderr)
    with tempfile.TemporaryDirectory(prefix="scan-", dir=directory) as scan:
        code = subprocess.call([str(DRIVER), *scan_args(args, Path(scan))], env=env, close_fds=False)
    if code:
        return code
    # Indirect dependencies the scan selected are not in the executable's extern
    # prelude; the driver listed them with their rlibs so the generated `::name::...`
    # paths resolve. They are linked already -- this only names them.
    externs = plan.with_suffix(".externs")
    if externs.exists():
        for line in externs.read_text().splitlines():
            if "=" in line:
                args = args + ["--extern", line]
    env["DBGVIS_INJECT"] = "1"
    print(f"dbgvis auto: compile {selected} with generated registrations", file=sys.stderr)
    return subprocess.call([str(DRIVER), *args], env=env, close_fds=False)


if __name__ == "__main__":
    sys.exit(main())
