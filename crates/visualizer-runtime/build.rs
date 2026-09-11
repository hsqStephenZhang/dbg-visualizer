use std::{env, fs, path::PathBuf};
fn main() {
    println!("cargo:rerun-if-changed=gdb.py");
    println!("cargo:rerun-if-env-changed=DBGVIS_BUFFER_BYTES");
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let source = fs::read_to_string("gdb.py").unwrap();
    let mut escaped = String::from("\"");
    for c in source.chars() {
        match c {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            c => escaped.push(c),
        }
    }
    escaped.push('"');
    let script = out.join("dbgvis_gdb_v2.py");
    fs::write(&script, format!("import sys, types\n_old = sys.modules.get('dbgvis_gdb')\n_mod = types.ModuleType('dbgvis_gdb')\n_mod._previous = _old._session if _old is not None else None\nsys.modules['dbgvis_gdb'] = _mod\nexec({escaped}, _mod.__dict__)\n")).unwrap();
    fs::write(
        out.join("autoload.rs"),
        format!(
            "#[debugger_visualizer(gdb_script_file = {:?})]\nmod dbgvis_autoload {{}}\n",
            script
        ),
    )
    .unwrap();
}
