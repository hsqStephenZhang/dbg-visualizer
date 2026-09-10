use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=debugger/common/core.py");
    println!("cargo:rerun-if-changed=debugger/gdb/backend.py");
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo OUT_DIR"));
    if env::var_os("CARGO_FEATURE_VISUALIZER").is_some() {
        let common = fs::read_to_string("debugger/common/core.py").expect("common debugger source");
        let backend = fs::read_to_string("debugger/gdb/backend.py").expect("GDB debugger source");
        let source = format!("{common}\n{backend}\n");
        let script = format!(
            "import sys, types\n_old = sys.modules.get('dbgvis_gdb')\nif _old is not None:\n    _old.dispose()\n_mod = types.ModuleType('dbgvis_gdb')\nsys.modules['dbgvis_gdb'] = _mod\nexec({}, _mod.__dict__)\n",
            python_string(&source)
        );
        let script_path = out.join("dbgvis_gdb.py");
        fs::write(&script_path, script).unwrap();
        fs::write(
            out.join("autoload.rs"),
            format!(
                "#[debugger_visualizer(gdb_script_file = {:?})]\nmod dbgvis_autoload {{}}\n",
                script_path
            ),
        )
        .unwrap();
    } else {
        fs::write(out.join("autoload.rs"), "").unwrap();
    }
}

fn python_string(text: &str) -> String {
    let mut result = String::from("\"");
    for c in text.chars() {
        match c {
            '\\' => result.push_str("\\\\"),
            '"' => result.push_str("\\\""),
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            c => result.push(c),
        }
    }
    result.push('"');
    result
}
