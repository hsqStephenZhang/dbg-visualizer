"""Verify the final linker output, including LTO and feature-disabled builds."""
from pathlib import Path
import os
import subprocess

ROOT = Path(__file__).resolve().parents[2]
SYMBOLS = {"DBG_VIS_MODULE_V1", "dbgvis_dispatch_v1"}


def symbols(binary):
    text = subprocess.check_output(["nm", "--defined-only", str(binary)], text=True)
    return {line.split()[-1] for line in text.splitlines() if line.split()}


for profile in ("dev", "release", "release-lto"):
    subprocess.run(["cargo", "build", "--offline", "--profile", profile], cwd=ROOT, check=True)
    directory = "debug" if profile == "dev" else profile
    assert SYMBOLS <= symbols(ROOT / f"target/{directory}/dbg-visualizer"), profile
    print("SYMBOLS_OK_" + profile)

subprocess.run(["cargo", "build", "--offline", "--no-default-features", "--target-dir", "target/no-visualizer"], cwd=ROOT, check=True)
assert not SYMBOLS & symbols(ROOT / "target/no-visualizer/debug/dbg-visualizer")
print("FEATURE_DISABLED_OK")

subprocess.run(["cargo", "build", "--offline", "--target-dir", "target/small-buffer"], cwd=ROOT, check=True,
               env=dict(os.environ, DBGVIS_BUFFER_BYTES="8192"))
output = subprocess.check_output(["gdb", "-q", "-nx", "-batch", "-iex", "set auto-load python-scripts off",
                                 str(ROOT / "target/small-buffer/debug/dbg-visualizer"),
                                 "-ex", "set language c", "-ex", "p *(unsigned long*)((unsigned long)&DBG_VIS_MODULE_V1 + 80)"], text=True)
assert "= 8192" in output, output
print("COMPILE_TIME_CAPACITY_OK")
