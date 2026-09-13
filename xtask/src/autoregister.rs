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
///
/// `scan` pins `DBGVIS_SCAN_DEPS` so a gate never inherits it from the developer's
/// shell: `None` removes it, which is what an ordinary build sees and what Cargo's
/// dep-info records differently from an explicit "0".
pub fn environment(run: Run, crate_name: &str, target: &Path, scan: Option<&str>) -> Run {
    let run = run
        .env("RUSTC_WORKSPACE_WRAPPER", tools().join("wrapper.py"))
        .env("DBGVIS_AUTO_CRATE", crate_name)
        .env("CARGO_TARGET_DIR", target);
    match scan {
        Some(mode) => run.env("DBGVIS_SCAN_DEPS", mode),
        None => run.env_remove("DBGVIS_SCAN_DEPS"),
    }
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
            Some("0"),
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
    drop(scratch);
    same_name_lib_and_bin()?;
    dependency_reexports()?;
    println!("AUTO_COMPILER_CONTRACTS_OK");
    Ok(())
}

/// A package holding both `src/lib.rs` and `src/main.rs` gives both targets the
/// package's crate name, and Cargo builds the library first. The wrapper must let
/// that invocation through and instrument the bin that follows, rather than failing
/// the build -- which is what it used to do, making the commonest Cargo layout
/// unusable. Every other fixture here sidesteps it with a distinct `[[bin]] name`.
fn same_name_lib_and_bin() -> Result {
    let scratch = TempDir::new("dbgvis lib and bin ")?;
    let project = scratch.path();
    let facade = root().join("crates/dbgvis").display().to_string();
    write(
        project.join("Cargo.toml"),
        &format!(
            r#"
[package]
name = "both_targets"
version = "0.0.0"
edition = "2024"
[workspace]
[dependencies]
dv = {{ package = "dbgvis", path = "{facade}", features = ["derive"] }}
[profile.dev]
debug = 2
"#
        ),
    )?;
    write(
        project.join("src/lib.rs"),
        r#"
#[derive(Debug)] pub struct Shared { pub count: u32 }
pub fn make() -> Shared { Shared { count: 5 } }
// `LibOnly` is never a named variable in the executable, so it is reachable only
// by scanning this crate's own functions.
#[derive(Debug)] pub struct LibOnly { pub tag: u8 }
pub fn inside() -> u8 {
    let hidden = LibOnly { tag: 3 };
    std::hint::black_box(&hidden);
    hidden.tag
}
// A trait implementation is not a module child, and unlike an inherent impl it is
// not reachable from the implementing type either, so it needs its own enumeration.
#[derive(Debug)] pub struct TraitLocal { pub tag: u8 }
pub struct Shown;
impl std::fmt::Display for Shown {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let via_trait = TraitLocal { tag: 4 };
        std::hint::black_box(&via_trait);
        f.write_str("shown")
    }
}
// An early-bound lifetime parameter still leaves exactly one instantiation.
#[derive(Debug)] pub struct BorrowedLocal { pub tag: u8 }
pub struct Borrowed<'a>(pub &'a str);
impl<'a> Borrowed<'a> {
    pub fn peek(&self) {
        let via_lifetime = BorrowedLocal { tag: 5 };
        std::hint::black_box(&via_lifetime);
    }
}
"#,
    )?;
    write(
        project.join("src/main.rs"),
        r#"
fn main() {
    dv::enable!();
    let shared = both_targets::make();
    let items = vec![both_targets::make()];
    let tag = both_targets::inside();
    let shown = format!("{}", both_targets::Shown);
    let text = String::from("borrowed");
    both_targets::Borrowed(&text).peek();
    std::hint::black_box((&shared, &items, &tag, &shown));
    println!("BOTH_TARGETS_EXECUTED");
}
"#,
    )?;
    let target = root().join("target/auto-register/both-targets");
    let build = |mode: Option<&str>| {
        environment(
            Run::new("cargo")
                .args(["build", "--offline"])
                .cwd(project)
                .timeout(600),
            "both_targets",
            &target,
            mode,
        )
    };
    let mut previous_invocations = None;
    // Use one target directory throughout: separate targets hide missing Cargo
    // invalidation, both when retaining MIR and when removing registrations.
    // Unset is what an ordinary build sees, and Cargo's dep-info records it
    // differently from an explicit "0", so both transitions have to be walked.
    for mode in [None, Some("1"), Some("0"), None] {
        let output = build(mode).output()?;
        ensure!(
            output.contains("passing through both_targets (--crate-type lib)"),
            "the library build must be announced and passed through\n{output}"
        );
        let (path, generated, report) = plan(&output)?;
        for needle in [
            "::both_targets::r#Shared",
            "r#Vec<::both_targets::r#Shared>",
        ] {
            ensure!(
                generated.contains(needle),
                "plan lacks {needle}\n{generated}\n{report}"
            );
        }
        for library_only in [
            "::both_targets::r#LibOnly",
            // Reached only through the crate's trait implementations.
            "::both_targets::r#TraitLocal",
            // Reached only if lifetime parameters do not disqualify an instantiation.
            "::both_targets::r#BorrowedLocal",
        ] {
            ensure!(
                generated.contains(library_only) == (mode == Some("1")),
                "{library_only} must follow DBGVIS_SCAN_DEPS={mode:?}\n{generated}\n{report}"
            );
        }
        let log = path.with_file_name("invocations.log");
        let mut invocations = read(&log)?;
        ensure!(
            previous_invocations.as_ref() != Some(&invocations),
            "switching scan mode must really invoke the wrapper (not replay stderr)"
        );
        let before = modified(&path)?;
        build(mode).output()?;
        if read(&log)? != invocations {
            invocations = read(&log)?;
            build(mode).output()?;
        }
        ensure!(
            read(&log)? == invocations,
            "unchanged mode must converge to fresh"
        );
        ensure!(
            modified(&path)? == before,
            "unchanged mode rewrote the plan"
        );
        previous_invocations = Some(invocations);
        Run::new(target.join("debug/both_targets"))
            .marker("BOTH_TARGETS_EXECUTED")
            .timeout(30)
            .check()?;
    }
    println!("AUTO_LIB_AND_BIN_OK");
    println!("AUTO_SCAN_MODE_INVALIDATION_OK");
    Ok(())
}

fn dependency_reexports() -> Result {
    let scratch = TempDir::new("dbgvis auto reexports ")?;
    let project = scratch.path();
    let facade = root().join("crates/dbgvis").display().to_string();
    write(
        project.join("Cargo.toml"),
        &format!(
            r#"
[package]
name = "scan_reexports"
version = "0.0.0"
edition = "2024"
[workspace]
exclude = ["foreign"]
[dependencies]
dv = {{ package = "dbgvis", path = "{facade}", features = ["derive"] }}
foreign = {{ path = "foreign" }}
"#
        ),
    )?;
    write(
        project.join("foreign/Cargo.toml"),
        "[package]\nname = \"foreign\"\nversion = \"0.0.0\"\nedition = \"2024\"\n",
    )?;
    for (path, source) in [
        ("src/main.rs", "reexport_main.rs"),
        ("src/lib.rs", "reexport_lib.rs"),
        ("foreign/src/lib.rs", "reexport_foreign.rs"),
    ] {
        write(project.join(path), &fixture(source)?)?;
    }
    let target = root().join("target/auto-register/reexports");
    let output = environment(
        Run::new("cargo")
            .args(["build", "--offline"])
            .cwd(project)
            .timeout(600),
        "scan_reexports",
        &target,
        Some("1"),
    )
    .output()?;
    let (_, generated, report) = plan(&output)?;
    for expected in [
        "SelectedFunctionLocal",
        "SelectedMethodLocal",
        "ForeignUsed",
    ] {
        ensure!(
            generated.contains(expected),
            "lost {expected}\n{generated}\n{report}"
        );
    }
    for excluded in [
        "ForeignFunctionLocal",
        "ForeignStructLocal",
        "ForeignEnumLocal",
        "ForeignUnionLocal",
        "ForeignModuleLocal",
    ] {
        ensure!(
            !report.contains(excluded) && !generated.contains(excluded),
            "re-export escaped the selected crates: {excluded}\n{generated}\n{report}"
        );
    }
    Run::new(target.join("debug/scan_reexports"))
        .marker("REEXPORTS_EXECUTED")
        .timeout(30)
        .check()?;
    println!("AUTO_DEPENDENCY_REEXPORTS_OK");
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
                Some("0"),
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
