"""Optional source-tree loader. Cargo embeds an independent bundle into the binary."""
import sys
import types
from pathlib import Path

# GDB `source` shares globals with an enclosing sourced Python script; __file__ can belong
# to that caller. The executing code object's filename still identifies this loader.
root = Path(sys._getframe().f_code.co_filename).resolve().parents[1]
old = sys.modules.get("dbgvis_gdb")
if old is not None:
    old.dispose()
module = types.ModuleType("dbgvis_gdb")
sys.modules["dbgvis_gdb"] = module
for relative in ("common/core.py", "gdb/backend.py"):
    path = root / relative
    exec(compile(path.read_text(), str(path), "exec"), module.__dict__)
