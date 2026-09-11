"""Offline consumer compile contracts. Negative auto cases need codegen, not cargo check."""
from pathlib import Path
import os
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
CASES = {
    "alias_generic": (True, "", r'''
use std::{fmt, marker::PhantomData, collections::HashMap};
struct Policy;
struct Label;
impl fmt::Display for Label { fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str("label") } }
#[derive(dv::Visualize)]
struct Wrapper<'a, T, P, const N: usize> { value: T, text: &'a str, policy: PhantomData<P> }
#[dv::visualizers] mod first {
    type Root<'a> = super::Wrapper<'a, std::collections::HashMap<u8, Vec<super::Label>>, super::Policy, 9>;
    #[cfg(any())] type Disabled = Missing;
}
#[dv::visualizers] mod second { #[dbgvis(display)] type Root = u32; }
fn main() {
    dv::enable!(first, second);
    let text = String::from("borrowed");
    let value = Wrapper::<_, Policy, 9> { value: HashMap::from([(1, vec![Label])]), text: &text, policy: PhantomData };
    let mut buffer = [0; 256];
    assert_eq!(dv::format_into(&value, &mut buffer, Default::default()).text,
        "Wrapper { value: {1: [label]}, text: \"borrowed\", policy: PhantomData }");
}
'''),
    "missing_field": (False, "no Visualize, Debug or Display", r'''
struct Missing;
#[derive(dv::Visualize)] struct Root<T> { value: T }
#[dv::visualizers] mod registry { type Root = super::Root<super::Missing>; }
#[dv::main(registry = registry)] fn main() {}
'''),
    "missing_empty_vec": (False, "no Visualize, Debug or Display", r'''
struct Missing;
fn main() {
    let mut buffer = [0; 64];
    let empty: Vec<Missing> = Vec::new();
    std::hint::black_box(dv::format_into(&empty, &mut buffer, Default::default()));
}
'''),
    "missing_nested_map": (False, "no Visualize, Debug or Display", r'''
struct Missing;
#[dv::visualizers] mod registry { type Root = std::collections::HashMap<u8, Vec<super::Missing>>; }
#[dv::main(registry = registry)] fn main() {}
'''),
    "explicit_display": (False, "Display", r'''
#[derive(Debug)] struct DebugOnly;
#[derive(dv::Visualize)] struct Root { #[dbgvis(via = "display")] value: DebugOnly }
fn main() {}
'''),
    "explicit_debug": (False, "Debug", r'''
struct NoDebug;
#[derive(dv::Visualize)] struct Root { #[dbgvis(via = "debug")] value: NoDebug }
fn main() {}
'''),
    "missing_root": (False, "Display", r'''
#[derive(Debug)] struct DebugOnly;
#[dv::visualizers] mod registry { #[dbgvis(display)] type Root = super::DebugOnly; }
#[dv::main(registry = registry)] fn main() {}
'''),
    "generic_family": (False, "concrete type/const", r'''
#[dv::visualizers] mod registry { type Root<T> = Vec<T>; }
fn main() {}
'''),
    "conflicting_field": (False, "skip cannot", r'''
#[derive(dv::Visualize)] struct Root { #[dbgvis(skip, via = "debug")] value: u8 }
fn main() {}
'''),
    "invalid_default": (False, "default mode must be registered", r'''
#[dv::visualizers] mod registry { #[dbgvis(debug, default = "display")] type Root = u8; }
fn main() {}
'''),
    "borrowed_not_static": (False, "lifetime", r'''
struct Borrowed<'a>(&'a str);
impl std::fmt::Display for Borrowed<'static> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(self.0) }
}
#[dv::visualizers] mod registry { #[dbgvis(display)] type Root<'a> = super::Borrowed<'a>; }
#[dv::main(registry = registry)] fn main() {}
'''),
}


def main():
    with tempfile.TemporaryDirectory(prefix="dbgvis consumer ") as directory:
        project = Path(directory)
        (project / "src").mkdir()
        (project / "Cargo.toml").write_text(f'''[package]
name = "dbgvis-compile-consumer"
version = "0.0.0"
edition = "2024"
[workspace]
[dependencies]
dv = {{ package = "dbgvis", path = "{ROOT / 'crates/dbgvis'}", features = ["derive"] }}
''')
        environment = dict(os.environ, CARGO_TARGET_DIR=str(ROOT / "target/compiler-contracts"))
        for name, (success, diagnostic, source) in CASES.items():
            (project / "src/main.rs").write_text(source)
            # Debug and optimized builds must both enforce the auto fallback contract.
            for profile in ([], ["--release"]):
                result = subprocess.run(["cargo", "+nightly", "build", "--offline", *profile], cwd=project,
                                        env=environment, text=True, stdout=subprocess.PIPE,
                                        stderr=subprocess.STDOUT, timeout=120)
                assert (result.returncode == 0) == success, (name, profile, result.stdout)
                assert diagnostic in result.stdout, (name, result.stdout)
                if success:
                    binary = Path(environment["CARGO_TARGET_DIR"]) / ("release" if profile else "debug") / "dbgvis-compile-consumer"
                    subprocess.run([binary], check=True, timeout=10)
            print(name + ": OK", flush=True)
    disabled = ROOT / "target/disabled"
    subprocess.run(["cargo", "+stable", "build", "--offline", "-p", "dbg-visualizer", "--no-default-features",
                    "--target-dir", disabled], cwd=ROOT, check=True, timeout=120)
    tree = subprocess.check_output(["cargo", "+stable", "tree", "--offline", "-p", "dbg-visualizer", "--no-default-features"], cwd=ROOT, text=True)
    symbols = subprocess.check_output(["nm", disabled / "debug/dbg-visualizer"], text=True)
    assert all(name not in tree for name in ("dbgvis", "visualizer-runtime", "dbgvis-macros")), tree
    assert all(name not in symbols for name in ("DBG_VIS_MODULE", "dbgvis_dispatch", "__DBG_RUNTIME", "visualizer_runtime"))
    print("FEATURE_DISABLED_OK")
    print("COMPILER_CONTRACTS_OK")


if __name__ == "__main__":
    main()
