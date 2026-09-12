//! Experimental two-pass automatic registration driver.
//!
//! Port of the former `tools/auto-register/test.py`. The driver itself
//! (`tools/auto-register/driver.rs`) and the `RUSTC_WORKSPACE_WRAPPER` shim
//! (`tools/auto-register/wrapper.py`) stay where they are: the wrapper is invoked
//! by cargo for every rustc call, not by this gate.

use crate::harness::{Failure, Result, Run, TempDir, root, write};
use crate::visibility;
use std::path::{Path, PathBuf};

/// Only the compiler revision audited by this experiment can build the driver.
const EXPECTED_RUSTC: &str = "rustc 1.99.0-nightly (12c36e253 2026-08-10)";

pub fn tools() -> PathBuf {
    root().join("tools/auto-register")
}

/// Environment for a wrapper-driven cargo build.
pub fn environment(run: Run, crate_name: &str, target: &Path) -> Run {
    run.env("RUSTC_WORKSPACE_WRAPPER", tools().join("wrapper.py"))
        .env("DBGVIS_AUTO_CRATE", crate_name)
        .env("CARGO_TARGET_DIR", target)
}

/// Locate the plan the driver wrote, returning (path, generated source, tsv report).
pub fn plan(output: &str) -> Result<(PathBuf, String, String)> {
    let marker = "dbgvis auto: scan ";
    let line = output
        .lines()
        .find(|line| line.contains(marker) && line.contains(" -> "))
        .ok_or_else(|| Failure(format!("no scan plan in output\n{output}")))?;
    let path = PathBuf::from(
        line.rsplit_once(" -> ")
            .expect("checked above")
            .1
            .trim()
            .to_owned(),
    );
    let generated = std::fs::read_to_string(&path)
        .map_err(|error| Failure(format!("read {}: {error}", path.display())))?;
    let report_path = path.with_extension("tsv");
    let report = std::fs::read_to_string(&report_path)
        .map_err(|error| Failure(format!("read {}: {error}", report_path.display())))?;
    Ok((path, generated, report))
}

pub fn build_driver() -> Result {
    let actual = Run::new("rustc").args(["--version"]).output()?;
    let actual = actual.trim();
    ensure!(
        actual == EXPECTED_RUSTC,
        "Unsupported driver toolchain: {actual}; expected {EXPECTED_RUSTC} with rustc-dev"
    );
    let output = root().join("target/auto-register/driver");
    std::fs::create_dir_all(output.parent().expect("has parent"))
        .map_err(|error| Failure(format!("mkdir driver target: {error}")))?;
    Run::new("rustc")
        .arg(tools().join("driver.rs"))
        .args(["--edition=2024", "-C", "rpath=yes", "-D", "warnings", "-o"])
        .arg(&output)
        .timeout(600)
        .check()?;
    println!("AUTO_DRIVER_BUILT {actual}");
    Ok(())
}

fn fixture(name: &str) -> Result<String> {
    let path = root().join("tests/cases/auto").join(name);
    std::fs::read_to_string(&path)
        .map_err(|error| Failure(format!("read {}: {error}", path.display())))
}

fn contracts() -> Result {
    let scratch = TempDir::new("dbgvis auto contracts ")?;
    let project = scratch.path();
    let facade = root().join("crates/dbgvis").display().to_string();
    write(
        project.join("Cargo.toml"),
        &format!(
            r#"
[package]
name = "auto_contract"
version = "0.0.0"
edition = "2024"
[workspace]
[dependencies]
dv = {{ package = "dbgvis", path = "{facade}", features = ["derive"] }}
idx = {{ package = "indexmap", version = "=2.14.0" }}
[profile.dev]
debug = 2
[profile.release]
debug = 2
lto = true
codegen-units = 1
"#
        ),
    )?;
    let source = project.join("src/main.rs");
    let positive = fixture("positive.rs")?;
    write(&source, &positive)?;
    let target = root().join("target/auto-register/contracts");
    let build = |profile: &[&str]| {
        environment(
            Run::new("cargo")
                .args(["build", "--offline"])
                .args(profile)
                .cwd(project)
                .timeout(600),
            "auto_contract",
            &target,
        )
    };

    for profile in [&[][..], &["--release"][..]] {
        let output = build(profile).output()?;
        let (path, generated, report) = plan(&output)?;
        for needle in [
            "::dv::register_type!",
            "::idx::r#IndexMap",
            "Packet<u8, 17>",
            "type __DbgvisAuto_",
        ] {
            ensure!(
                generated.contains(needle),
                "generated plan lacks {needle}\n{generated}"
            );
        }
        ensure!(
            generated.contains("Vec<u16>"),
            "must substitute generic function arguments\n{generated}"
        );
        for absent in [
            "register_type!(crate::r#Point);",
            "register_type!(crate::r#Explicit);",
        ] {
            ensure!(!generated.contains(absent), "plan re-registered {absent}");
        }
        for needle in [
            "EXISTING\tlocal typed anchor\tPoint",
            "EXISTING\tlocal typed anchor\tExplicit",
            "SKIP\tno formatting trait\tNoFormat",
            "SKIP\ttype is not accessible/nameable from root\t",
            "SKIP\ttype is not accessible/nameable from root\tmain::Local",
            "SKIP\ttype is not accessible/nameable from root\thidden::Secret",
            "Borrowed<'a",
        ] {
            ensure!(report.contains(needle), "report lacks {needle:?}\n{report}");
        }
        let folder = if profile.is_empty() {
            "debug"
        } else {
            "release"
        };
        Run::new(target.join(folder).join("auto_contract"))
            .marker("CONTRACT_EXECUTED")
            .timeout(30)
            .check()?;

        let before = modified(&path)?;
        let log = path.with_file_name("invocations.log");
        let mut invocations = read(&log)?;
        build(profile).output()?;
        // A newly discovered include is newer than Cargo's first invocation stamp.
        // Permit one warm-up rebuild, but never an endless rebuild loop.
        if read(&log)? != invocations {
            invocations = read(&log)?;
            build(profile).output()?;
        }
        // Cargo may replay cached wrapper stderr even without invoking rustc.
        ensure!(
            read(&log)? == invocations,
            "unchanged build must converge to fresh"
        );
        ensure!(
            modified(&path)? == before,
            "unchanged build rewrote the plan"
        );
    }

    write(
        &source,
        &positive.replace(
            r#"println!("CONTRACT_EXECUTED");"#,
            r#"let added = vec![17u128]; std::hint::black_box(added); println!("CONTRACT_EXECUTED");"#,
        ),
    )?;
    let output = build(&[]).output()?;
    ensure!(
        plan(&output)?.1.contains("Vec<u128>"),
        "source changes must invalidate the discovery plan"
    );

    // Baseline builds, automatic strict registration fails during monomorphization.
    write(&source, &fixture("negative.rs")?)?;
    let baseline = || {
        Run::new("cargo")
            .args(["build", "--offline"])
            .cwd(project)
            .env("CARGO_TARGET_DIR", &target)
            .timeout(600)
    };
    baseline().check()?;
    Run::new(target.join("debug/auto_contract"))
        .timeout(30)
        .check()?;
    let output = build(&[])
        .expect_failure()
        .marker("no Visualize, Debug or Display implementation")
        .output()?;
    ensure!(
        output.contains("dbgvis auto: compile"),
        "scan succeeds; generated entry causes failure"
    );
    let generated = plan(&output)?.1;
    ensure!(
        generated.contains("Vec<crate::r#NoFormat>"),
        "plan lost the failing entry\n{generated}"
    );
    ensure!(
        !generated.contains("Vec<u128>"),
        "old generated entries must not feed back into scanning"
    );
    println!("AUTO_STRICT_FAILURE_CONFIRMED");

    // Known boundary: an upstream root is not visible in the local-anchor scan.
    let library = project.join("upstream");
    write(
        library.join("Cargo.toml"),
        &format!(
            r#"
[package]
name = "upstream"
version = "0.0.0"
edition = "2024"
[lib]
path = "lib.rs"
[dependencies]
dv = {{ package = "dbgvis", path = "{facade}", features = ["derive"] }}
"#
        ),
    )?;
    write(library.join("lib.rs"), "dv::register_type!(u64);")?;
    let manifest = project.join("Cargo.toml");
    let existing = read(&manifest)?;
    write(
        &manifest,
        &format!("{existing}\n[dependencies.upstream]\npath = \"upstream\"\n"),
    )?;
    write(
        &source,
        r#"use upstream as _; fn main() { dv::enable!(); let value = 7u64; std::hint::black_box(value); println!("CROSS_CRATE_DEDUP_OK"); }"#,
    )?;
    baseline().check()?;
    let binary = target.join("debug/auto_contract");
    Run::new(&binary).timeout(30).check()?;
    build(&[]).check()?;
    Run::new(&binary)
        .marker("CROSS_CRATE_DEDUP_OK")
        .timeout(30)
        .check()?;
    println!("AUTO_CROSS_CRATE_DEDUP_OK");
    println!("AUTO_COMPILER_CONTRACTS_OK");
    Ok(())
}

fn read(path: &Path) -> Result<String> {
    std::fs::read_to_string(path)
        .map_err(|error| Failure(format!("read {}: {error}", path.display())))
}

fn modified(path: &Path) -> Result<std::time::SystemTime> {
    std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .map_err(|error| Failure(format!("stat {}: {error}", path.display())))
}

pub fn run(gdb: bool) -> Result {
    build_driver()?;
    contracts()?;
    visibility::contracts(gdb)?;

    let target = root().join("target/auto-register/examples");
    for (example, script, marker) in [
        ("auto_poc", "auto_poc.py", "AUTO_GDB_OK"),
        ("demo", "demo.py", "AUTO_DEMO_GDB_OK"),
    ] {
        for (profile, folder) in [
            (&[][..], "debug"),
            (&["--profile", "release-lto"][..], "release-lto"),
        ] {
            environment(
                Run::new("cargo")
                    .args(["build", "--offline", "--example", example])
                    .args(profile)
                    .timeout(600),
                example,
                &target,
            )
            .check()?;
            let binary = target.join(folder).join("examples").join(example);
            Run::new(&binary).timeout(30).check()?;
            if gdb {
                Run::new("rust-gdb")
                    .args(["-q", "-nx", "--batch", "-iex"])
                    .arg(format!("add-auto-load-safe-path {}", binary.display()))
                    .arg("-x")
                    .arg(root().join("tests/inferior").join(script))
                    .arg("--args")
                    .arg(&binary)
                    .marker(marker)
                    .timeout(120)
                    .check()?;
            }
        }
    }
    println!("AUTO_POC_SUITE_OK");
    Ok(())
}
