"""Load with `command script import /path/to/debugger/lldb/dbgvis_lldb.py`."""
from pathlib import Path
import lldb

_root = Path(__file__).resolve().parents[1]
exec(compile((_root / "common" / "core.py").read_text(), str(_root / "common" / "core.py"), "exec"), globals())
exec(compile((_root / "lldb" / "backend.py").read_text(), str(_root / "lldb" / "backend.py"), "exec"), globals())
