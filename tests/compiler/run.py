"""Offline consumer compile contracts. Negative auto cases need codegen, not cargo check."""
from pathlib import Path
import os
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[2]
# rustc target spec `emit-debug-gdb-scripts` is false for Apple and MSVC targets.
EMITS_GDB_SECTION = sys.platform == "linux"


def embedded_gdb_script(binary):
    """Whether the final executable carries the inlined dbgvis bridge (prologue marker)."""
    return b"types.ModuleType('dbgvis_gdb')" in Path(binary).read_bytes()

CASES = {
    "alias_generic": (True, "", r'''
use std::{fmt, marker::PhantomData, collections::HashMap};
struct Policy;
struct Label;
impl fmt::Display for Label { fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str("label") } }
#[derive(dv::Visualize)]
struct Wrapper<'a, T, P, const N: usize> { value: T, text: &'a str, policy: PhantomData<P> }
mod first {
    #[dv::register]
    type Root<'a> = super::Wrapper<'a, std::collections::HashMap<u8, Vec<super::Label>>, super::Policy, 9>;
    #[cfg(any())] #[dv::register] type Disabled = Missing;
}
mod second { dv::register_type!(u32; display); }
fn main() {
    dv::enable!();
    assert_eq!(dv::VIS_TYPES.len(), 2);
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
dv::register_type!(Root<Missing>);
#[dv::main] fn main() {}
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
dv::register_type!(std::collections::HashMap<u8, Vec<Missing>>);
#[dv::main] fn main() {}
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
dv::register_type!(DebugOnly; display);
#[dv::main] fn main() {}
'''),
    "generic_family": (False, "concrete type/const", r'''
#[dv::register] type Root<T> = Vec<T>;
fn main() {}
'''),
    "conflicting_field": (False, "skip cannot", r'''
#[derive(dv::Visualize)] struct Root { #[dbgvis(skip, via = "debug")] value: u8 }
fn main() {}
'''),
    "invalid_default": (False, "default mode must be registered", r'''
dv::register_type!(u8; debug, default = "display");
fn main() {}
'''),
    "borrowed_not_static": (False, "lifetime", r'''
struct Borrowed<'a>(&'a str);
impl std::fmt::Display for Borrowed<'static> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(self.0) }
}
#[dv::register(display)] type Root<'a> = Borrowed<'a>;
#[dv::main] fn main() {}
'''),
    "automatic_and_optout": (True, "", r'''
#[derive(dv::Visualize)] struct Automatic { x: u32 }
#[derive(dv::Visualize)] #[dbgvis(no_register)] struct Nested { x: u32 }
#[derive(dv::Visualize)] struct Generic<T>(T);
#[dv::register] #[derive(Debug)] struct DebugOnly(u32);
#[derive(Debug)] struct PlainDebug;
#[dv::register(display)] struct DisplayOnly;
impl std::fmt::Display for DisplayOnly {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("display") }
}
dv::register_type!(Generic<Nested>);
#[dv::main] fn main() {
    assert_eq!(dv::VIS_TYPES.len(), 4);
    let mut buf = [0; 128];
    assert_eq!(dv::format_into(&Generic(Nested { x: 3 }), &mut buf, Default::default()).text,
        "Generic(Nested { x: 3 })");
}
'''),
    "empty_registry": (True, "", r'''
#[derive(dv::Visualize)] struct Generic<T>(T);
#[derive(dv::Visualize)] struct Borrowed<'a>(&'a str);
#[derive(dv::Visualize)] #[dbgvis(no_register)] struct Nested { x: u32 }
#[dv::main] fn main() { assert!(dv::VIS_TYPES.is_empty()); }
'''),
    "duplicate_types_coalesced": (True, "", r'''
mod a { dv::register_type!(u32); }
mod b { #[dv::register(display)] type Root = u32; }
fn main() { dv::enable!(); assert_eq!(dv::collected_roots().len(), 1); }
'''),
    "duplicate_borrowed_types_coalesced": (True, "", r'''
mod a { #[dv::register(debug)] type Root<'a> = &'a str; }
mod b { #[dv::register(display)] type Root<'b> = &'b str; }
fn main() { dv::enable!(); assert_eq!(dv::collected_roots().len(), 1); }
'''),
    "same_name_different_identity_rejected": (True, "", r'''
fn main() {
    { #[derive(dv::Visualize)] struct Same(u32); }
    { #[derive(dv::Visualize)] struct Same(f32); }
    let error = std::panic::catch_unwind(dv::collected_roots).err().expect("collision accepted");
    let message = error.downcast_ref::<String>().map(String::as_str)
        .or_else(|| error.downcast_ref::<&str>().copied()).unwrap();
    assert!(message.contains("different concrete type identities"), "{message}");
}
'''),
    "scoped_types": (True, "", r'''
mod a { #[derive(dv::Visualize)] pub struct Same(u32); }
mod b { #[derive(dv::Visualize)] pub struct Same(u32); }
#[dv::main] fn main() {
    #[derive(dv::Visualize)] struct Local(u32);
    assert_eq!(dv::VIS_TYPES.len(), 3);
}
'''),
    "auto_missing_concrete": (False, "no Visualize, Debug or Display", r'''
struct Missing;
#[derive(dv::Visualize)] struct Root { value: Missing }
#[dv::main] fn main() {}
'''),
    "generic_attribute_rejected": (False, "concrete type/const", r'''
#[dv::register] #[derive(Debug)] struct Root<T>(T);
fn main() {}
'''),
    "duplicate_default": (False, "duplicate default", r'''
dv::register_type!(u32; auto, default = "auto", default = "auto");
fn main() {}
'''),
}


def check_features(project):
    """Keep opt-out coverage in a consumer project; the demo is always enabled."""
    sources = {
        "a": r'''
#[cfg_attr(feature = "visualize", derive(dbgvis::Visualize))]
pub struct Record<T, Policy> { value: T, policy: std::marker::PhantomData<Policy> }
impl<T, P> Record<T, P> {
    pub fn new(value: T) -> Self { Self { value, policy: std::marker::PhantomData } }
}
''',
        "b": r'''
#[cfg_attr(feature = "visualize", derive(dbgvis::Visualize))]
pub struct Item { count: u32 }
impl Item { pub fn new(count: u32) -> Self { Self { count } } }
#[cfg_attr(feature = "visualize", derive(dbgvis::Visualize))]
pub struct UnusedRegistered { pub value: u32 }
''',
        "c": r'''
#[cfg_attr(feature = "visualize", derive(dbgvis::Visualize))]
pub enum State { Pending }
pub struct Label;
impl std::fmt::Display for Label {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Display-only")
    }
}
''',
    }
    for name, source in sources.items():
        crate = project / name
        (crate / "src").mkdir(parents=True)
        (crate / "src/lib.rs").write_text(source)
        (crate / "Cargo.toml").write_text(f'''[package]
name = "demo-{name}"
version = "0.0.0"
edition = "2024"
[features]
default = []
visualize = ["dep:dbgvis"]
[dependencies]
dbgvis = {{ path = "{ROOT / 'crates/dbgvis'}", optional = true, features = ["derive"] }}
''')
    (project / "Cargo.toml").write_text(f'''[package]
name = "dbgvis-feature-consumer"
version = "0.0.0"
edition = "2024"
[workspace]
members = ["a", "b", "c"]
[profile.release]
lto = true
codegen-units = 1
debug = 2
[features]
default = []
visualize = ["dep:dbgvis", "demo-a/visualize", "demo-b/visualize", "demo-c/visualize"]
[dependencies]
dbgvis = {{ path = "{ROOT / 'crates/dbgvis'}", optional = true, features = ["derive"] }}
demo-a = {{ path = "{project / 'a'}" }}
demo-b = {{ path = "{project / 'b'}" }}
demo-c = {{ path = "{project / 'c'}" }}
''')
    (project / "src/main.rs").write_text(r'''
struct NoTraits;
#[cfg_attr(feature = "visualize", derive(dbgvis::Visualize))]
struct App {
    a: demo_a::Record<demo_b::Item, NoTraits>,
    c: demo_c::State,
    label: demo_c::Label,
    #[cfg_attr(feature = "visualize", dbgvis(skip))]
    skipped: NoTraits,
}
fn main() {
    #[cfg(feature = "visualize")]
    dbgvis::enable!();
    let app = App { a: demo_a::Record::new(demo_b::Item::new(8)),
        c: demo_c::State::Pending, label: demo_c::Label, skipped: NoTraits };
    #[cfg(feature = "visualize")]
    {
        let mut buffer = [0; 512];
        let text = dbgvis::format_into(&app, &mut buffer, Default::default()).text;
        assert!(text.contains("count: 8") && text.contains("State::Pending"));
        assert!(!text.contains("skipped"));
        assert!(text.contains("label: Display-only"));
        assert_eq!(dbgvis::VIS_TYPES.len(), 4); // App, Item, UnusedRegistered, State
    }
    std::hint::black_box((&app.a, &app.c, &app.label, &app.skipped));
}
''')
    for enabled, release in ((False, False), (True, False), (True, True)):
        target = ROOT / "target/feature-contracts" / ("on" if enabled else "off")
        feature = ["--features", "visualize"] if enabled else ["--no-default-features"]
        profile = ["--release"] if release else []
        toolchain = "+nightly" if enabled else "+stable"
        subprocess.run(["cargo", toolchain, "build", "--offline", *feature, *profile, "--target-dir", target],
                       cwd=project, check=True, timeout=120)
        binary = target / ("release" if release else "debug") / "dbgvis-feature-consumer"
        subprocess.run([binary], check=True, timeout=10)
        tree = subprocess.check_output(["cargo", toolchain, "tree", "--offline", *feature], cwd=project, text=True)
        symbols = subprocess.check_output(["nm", binary], text=True)
        if enabled:
            assert "linkme" in tree and "DBG_VIS_MODULE_V2" in symbols
            assert "__DBG_ANCHOR_UnusedRegistered" in symbols
            assert "__DBG_ANCHOR_Item" in symbols and "__DBG_ANCHOR_State" in symbols
            assert embedded_gdb_script(binary) == EMITS_GDB_SECTION, "debuginfo builds must embed the bridge"
        else:
            assert all(name not in tree for name in ("dbgvis v", "visualizer-runtime", "dbgvis-macros", "linkme")), tree
            assert all(name not in symbols for name in ("DBG_VIS_MODULE", "dbgvis_dispatch", "__DBG_RUNTIME", "visualizer_runtime", "linkme", "__DBG_ANCHOR", "__DBG_REGISTRATION"))
            assert not embedded_gdb_script(binary)
    print("TEMPORARY_CROSS_CRATE_DEBUG_LTO_OK")
    print("FEATURE_DISABLED_OK")
    # rustc emits `.debug_gdb_scripts` only when the final crate has debuginfo. A consumer's
    # default release profile (debug = 0) keeps the Rust registry but silently loses the bridge.
    target = ROOT / "target/feature-contracts/no-debuginfo"
    subprocess.run(["cargo", "+nightly", "build", "--offline", "--features", "visualize", "--release", "--target-dir", target],
                   cwd=project, check=True, timeout=120, env=dict(os.environ, CARGO_PROFILE_RELEASE_DEBUG="0"))
    binary = target / "release/dbgvis-feature-consumer"
    subprocess.run([binary], check=True, timeout=10)
    assert "DBG_VIS_MODULE_V2" in subprocess.check_output(["nm", binary], text=True)
    assert not embedded_gdb_script(binary), "debug = 0 must not embed the bridge; the guide documents this"
    print("NO_DEBUGINFO_DROPS_GDB_SCRIPT_OK")


def check_dependency_identity():
    """Two versions with identical names and layouts must never share formatters."""
    with tempfile.TemporaryDirectory(prefix="dbgvis identity ") as directory:
        project = Path(directory).resolve()
        (project / "src").mkdir()
        for version in (1, 2):
            library = project / f"v{version}"
            library.mkdir()
            (library / "Cargo.toml").write_text(f'''
[package]
name = "identity-library"
version = "{version}.0.0"
edition = "2024"
[lib]
path = "lib.rs"
[dependencies]
dv = {{ package = "dbgvis", path = "{ROOT / 'crates/dbgvis'}", features = ["derive"] }}
''')
            (library / "lib.rs").write_text('''
#[derive(dv::Visualize)]
pub struct Shared(pub u32);
''')
        (project / "Cargo.toml").write_text(f'''
[package]
name = "identity-consumer"
version = "0.0.0"
edition = "2024"
[workspace]
exclude = ["v1", "v2"]
[dependencies]
dv = {{ package = "dbgvis", path = "{ROOT / 'crates/dbgvis'}", features = ["derive"] }}
old = {{ package = "identity-library", path = "v1" }}
new = {{ package = "identity-library", path = "v2" }}
[profile.release]
lto = true
codegen-units = 1
''')
        (project / "src/main.rs").write_text('''
fn main() {
    assert_eq!(std::any::type_name::<old::Shared>(), std::any::type_name::<new::Shared>());
    assert_eq!(size_of::<old::Shared>(), size_of::<new::Shared>());
    assert_eq!(align_of::<old::Shared>(), align_of::<new::Shared>());
    assert_ne!(std::any::TypeId::of::<old::Shared>(), std::any::TypeId::of::<new::Shared>());
    assert_eq!(dv::VIS_TYPES.len(), 2);
    let error = std::panic::catch_unwind(|| { dv::enable!(); }).err().expect("collision accepted");
    let message = error.downcast_ref::<String>().map(String::as_str)
        .or_else(|| error.downcast_ref::<&str>().copied()).unwrap();
    assert!(message.contains("different concrete type identities"), "{message}");
}
''')
        target = ROOT / "target/identity-contracts"
        for profile, folder in (([], "debug"), (["--release"], "release")):
            subprocess.run(["cargo", "+nightly", "build", "--offline", *profile, "--target-dir", target],
                           cwd=project, check=True, timeout=120)
            subprocess.run([target / folder / "identity-consumer"], check=True, timeout=10)
    print("CROSS_VERSION_IDENTITY_REJECTED_OK", flush=True)


def main():
    with tempfile.TemporaryDirectory(prefix="dbgvis consumer ") as directory:
        project = Path(directory).resolve()  # cargo canonicalizes path deps (macOS /var symlink)
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
        check_features(project)
    check_dependency_identity()
    print("COMPILER_CONTRACTS_OK")


if __name__ == "__main__":
    main()
