"""Reproducible compiler + optional real GDB gates. Fixtures live outside crates/."""
import argparse
import os
from pathlib import Path
import re
import signal
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
TOOLS = ROOT / "tools/auto-register"


def run(args, *, cwd=ROOT, env=None, ok=True, marker=None):
    process = subprocess.Popen([str(a) for a in args], cwd=cwd, env=env,
                               stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                               text=True, start_new_session=True)
    try:
        output = process.communicate(timeout=90)[0]
    except subprocess.TimeoutExpired:
        os.killpg(process.pid, signal.SIGKILL)
        output = process.communicate()[0]
        raise AssertionError(f"TIMEOUT {args}\n{output}")
    print(output, end="", flush=True)
    assert (process.returncode == 0) == ok, (args, process.returncode, output)
    assert marker is None or marker in output, (marker, output)
    return output


def environment(crate, target):
    env = os.environ.copy()
    env.pop("RUSTC_WRAPPER", None)
    env["RUSTC_WORKSPACE_WRAPPER"] = str(TOOLS / "wrapper.py")
    env["DBGVIS_AUTO_CRATE"] = crate
    env["CARGO_TARGET_DIR"] = str(target)
    return env


def plan(output):
    path = Path(re.search(r"dbgvis auto: scan .* -> (.+)", output)[1])
    return path, path.read_text(), path.with_suffix(".tsv").read_text()


POSITIVE = r'''
#[derive(dv::Visualize)] struct Point { x: u32 }
#[dv::register(display)] struct Explicit;
impl std::fmt::Display for Explicit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("explicit") }
}
struct NoFormat;
#[derive(Debug)] struct Borrowed<'a>(&'a str);
#[derive(Debug)] struct Packet<T, const N: usize> { values: [T; N] }
mod hidden {
    #[derive(Debug)] struct Secret;
    #[inline(never)] pub fn work() { let secret = Secret; std::hint::black_box(secret); }
}
#[inline(never)] fn generic<T>(value: T) {
    let values = vec![value];
    std::hint::black_box(values);
}
fn main() {
    dv::enable!();
    let point = Point { x: 1 };
    let explicit = Explicit;
    let missing = NoFormat;
    let borrowed = Borrowed("text");
    #[derive(Debug)] struct Local;
    let local = Local;
    let map = std::collections::HashMap::from([(1u64, vec![Some(Point { x: 3 })])]);
    let ordered = idx::IndexMap::from([(1u32, "one".to_string())]);
    let packet = Packet { values: [2u8; 17] };
    generic(9u16);
    hidden::work();
    std::hint::black_box((point, explicit, missing, borrowed.0, local, map, ordered, packet.values));
    println!("CONTRACT_EXECUTED");
}
'''

NEGATIVE = r'''
struct NoFormat;
fn main() {
    dv::enable!();
    let missing: Vec<NoFormat> = Vec::new();
    std::hint::black_box(missing);
}
'''


def contracts():
    with tempfile.TemporaryDirectory(prefix="dbgvis auto contracts ") as temporary:
        project = Path(temporary)
        (project / "src").mkdir()
        (project / "Cargo.toml").write_text(f'''
[package]
name = "auto_contract"
version = "0.0.0"
edition = "2024"
[workspace]
[dependencies]
dv = {{ package = "dbgvis", path = "{ROOT / 'crates/dbgvis'}", features = ["derive"] }}
idx = {{ package = "indexmap", version = "=2.14.0" }}
[profile.dev]
debug = 2
[profile.release]
debug = 2
lto = true
codegen-units = 1
''')
        source = project / "src/main.rs"
        source.write_text(POSITIVE)
        env = environment("auto_contract", ROOT / "target/auto-register/contracts")
        for profile in ([], ["--release"]):
            output = run(["cargo", "build", "--offline", *profile], cwd=project, env=env)
            path, generated, report = plan(output)
            assert "::dv::register_type!" in generated
            assert "::idx::r#IndexMap" in generated
            assert "Vec<u16>" in generated, "must substitute generic function arguments"
            assert "Packet<u8, 17>" in generated
            assert "register_type!(crate::r#Point);" not in generated
            assert "register_type!(crate::r#Explicit);" not in generated
            assert "EXISTING\tlocal typed anchor\tPoint" in report
            assert "EXISTING\tlocal typed anchor\tExplicit" in report
            assert "SKIP\tno formatting trait\tNoFormat" in report
            assert "SKIP\ttype is not accessible/nameable from root\t" in report
            assert "SKIP\ttype is not accessible/nameable from root\tmain::Local" in report
            assert "SKIP\ttype is not accessible/nameable from root\thidden::Secret" in report
            assert "borrowed type: lifetime proof not implemented" in report
            binary = Path(env["CARGO_TARGET_DIR"]) / ("release" if profile else "debug") / "auto_contract"
            run([binary], marker="CONTRACT_EXECUTED")
            before = path.stat().st_mtime_ns
            log = path.with_name("invocations.log")
            invocations = log.read_text()
            again = run(["cargo", "build", "--offline", *profile], cwd=project, env=env)
            # A newly discovered include is newer than Cargo's first invocation stamp.
            # Permit one warm-up rebuild, but never an endless rebuild loop.
            if log.read_text() != invocations:
                invocations = log.read_text()
                again = run(["cargo", "build", "--offline", *profile], cwd=project, env=env)
            # Cargo may replay cached wrapper stderr even without invoking rustc.
            assert log.read_text() == invocations, "unchanged build must converge to fresh"
            assert path.stat().st_mtime_ns == before
        source.write_text(POSITIVE.replace('println!("CONTRACT_EXECUTED");',
            'let added = vec![17u128]; std::hint::black_box(added); println!("CONTRACT_EXECUTED");'))
        output = run(["cargo", "build", "--offline"], cwd=project, env=env)
        assert "Vec<u128>" in plan(output)[1], "source changes must invalidate the discovery plan"
        # Baseline builds, automatic strict registration fails during monomorphization.
        source.write_text(NEGATIVE)
        baseline = env.copy()
        baseline.pop("RUSTC_WORKSPACE_WRAPPER")
        baseline.pop("DBGVIS_AUTO_CRATE")
        run(["cargo", "build", "--offline"], cwd=project, env=baseline)
        run([Path(env["CARGO_TARGET_DIR"]) / "debug/auto_contract"])
        output = run(["cargo", "build", "--offline"], cwd=project, env=env, ok=False,
                     marker="no Visualize, Debug or Display implementation")
        assert "dbgvis auto: compile" in output, "scan succeeds; generated entry causes failure"
        _, generated, _ = plan(output)
        assert "Vec<crate::r#NoFormat>" in generated
        assert "Vec<u128>" not in generated, "old generated entries must not feed back into scanning"
        print("AUTO_STRICT_FAILURE_CONFIRMED", flush=True)
        # Known boundary: an upstream root is not visible in the local-anchor scan.
        library = project / "upstream"
        library.mkdir()
        (library / "Cargo.toml").write_text(f'''
[package]
name = "upstream"
version = "0.0.0"
edition = "2024"
[lib]
path = "lib.rs"
[dependencies]
dv = {{ package = "dbgvis", path = "{ROOT / 'crates/dbgvis'}", features = ["derive"] }}
''')
        (library / "lib.rs").write_text('dv::register_type!(u64);')
        manifest = project / "Cargo.toml"
        manifest.write_text(manifest.read_text() + '\n[dependencies.upstream]\npath = "upstream"\n')
        source.write_text('use upstream as _; fn main() { dv::enable!(); let value = 7u64; std::hint::black_box(value); println!("CROSS_CRATE_DEDUP_OK"); }')
        run(["cargo", "build", "--offline"], cwd=project, env=baseline)
        binary = Path(env["CARGO_TARGET_DIR"]) / "debug/auto_contract"
        run([binary])
        run(["cargo", "build", "--offline"], cwd=project, env=env)
        run([binary], marker="CROSS_CRATE_DEDUP_OK")
        print("AUTO_CROSS_CRATE_DEDUP_OK", flush=True)
    print("AUTO_COMPILER_CONTRACTS_OK", flush=True)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--gdb", action="store_true")
    args = parser.parse_args()
    run(["python3", TOOLS / "build.py"])
    contracts()
    env = environment("auto_poc", ROOT / "target/auto-register/demo")
    for profile, folder in (([], "debug"), (["--profile", "release-lto"], "release-lto")):
        run(["cargo", "build", "--offline", "--example", "auto_poc", *profile], env=env)
        binary = Path(env["CARGO_TARGET_DIR"]) / folder / "examples/auto_poc"
        run([binary])
        if args.gdb:
            run(["rust-gdb", "-q", "-nx", "--batch", "-iex", f"add-auto-load-safe-path {binary}",
                 "-x", TOOLS / "gdb_checks.py", "--args", binary], marker="AUTO_GDB_OK")
    print("AUTO_POC_SUITE_OK", flush=True)


if __name__ == "__main__":
    main()
