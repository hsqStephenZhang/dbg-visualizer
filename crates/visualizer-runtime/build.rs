use std::{env, fs, path::PathBuf};

// GDB executes `.debug_gdb_scripts` entries in its `__main__` namespace. The generated
// prologue instead runs gdb.py inside a dedicated `dbgvis_gdb` module so reloading the
// binary (`file`, re-run of an updated build) disposes the previous session and inherits
// its interrupted-call state. The module is published only after a successful load, so a
// broken load never shadows a working one or wedges later reloads.
const PROLOGUE: &str = "import sys, types
_old = sys.modules.get('dbgvis_gdb')
_mod = types.ModuleType('dbgvis_gdb')
_mod._previous = getattr(_old, '_session', None)
exec(SOURCE, _mod.__dict__)
sys.modules['dbgvis_gdb'] = _mod
";

/// Encode `source` as a Python string literal. The escapes used are valid in both Rust
/// and Python literals; every other byte is copied verbatim, which UTF-8 Python source
/// accepts. Do not use `{:?}`: Rust's `\u{..}` escapes are not Python syntax.
fn python_literal(source: &str) -> String {
    let mut escaped = String::from("\"");
    for c in source.chars() {
        match c {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            c => escaped.push(c),
        }
    }
    escaped.push('"');
    escaped
}

fn main() {
    println!("cargo:rerun-if-changed=gdb.py");
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let source = fs::read_to_string("gdb.py").unwrap();
    let script = out.join("dbgvis_gdb_v2.py");
    fs::write(
        &script,
        PROLOGUE.replace("SOURCE", &python_literal(&source)),
    )
    .unwrap();
    // `gdb_script_file` resolves relative to the file holding the attribute, which is the
    // generated autoload.rs inside OUT_DIR; an absolute path avoids depending on that.
    fs::write(
        out.join("autoload.rs"),
        format!(
            "#[debugger_visualizer(gdb_script_file = {:?})]\nmod dbgvis_autoload {{}}\n",
            script
        ),
    )
    .unwrap();
}
