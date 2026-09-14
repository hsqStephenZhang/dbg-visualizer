//! Offline consumer compile contracts.
//!
//! Port of the former `tests/compiler/run.py`. Every case is built in both debug and
//! release; the successful ones also run, and several assert their formatted text.
//! Case sources live in `tests/cases/` as real `.rs` files.

use crate::harness::{Failure, Result, Run, TempDir, root, write};
use std::path::Path;

/// rustc target spec `emit-debug-gdb-scripts` is false for Apple and MSVC targets.
const EMITS_GDB_SECTION: bool = cfg!(target_os = "linux");

struct Case {
    name: &'static str,
    succeeds: bool,
    diagnostic: &'static str,
}

#[rustfmt::skip]
const CASES: &[Case] = &[
    Case { name: "alias_generic", succeeds: true, diagnostic: "" },
    Case { name: "unformattable_generic_field", succeeds: true, diagnostic: "" },
    Case { name: "unformattable_vec_element", succeeds: true, diagnostic: "" },
    Case { name: "unformattable_nested_map", succeeds: true, diagnostic: "" },
    Case { name: "explicit_display", succeeds: false, diagnostic: "Display" },
    Case { name: "explicit_debug", succeeds: false, diagnostic: "Debug" },
    Case { name: "missing_root", succeeds: false, diagnostic: "Display" },
    Case { name: "generic_family", succeeds: false, diagnostic: "concrete type/const" },
    Case { name: "conflicting_field", succeeds: false, diagnostic: "skip cannot" },
    Case { name: "invalid_default", succeeds: false, diagnostic: "default mode must be registered" },
    Case { name: "borrowed_not_static", succeeds: false, diagnostic: "lifetime" },
    Case { name: "automatic_and_optout", succeeds: true, diagnostic: "" },
    Case { name: "empty_registry", succeeds: true, diagnostic: "" },
    Case { name: "duplicate_types_coalesced", succeeds: true, diagnostic: "" },
    Case { name: "duplicate_borrowed_types_coalesced", succeeds: true, diagnostic: "" },
    Case { name: "same_name_different_identity_rejected", succeeds: true, diagnostic: "" },
    Case { name: "scoped_types", succeeds: true, diagnostic: "" },
    Case { name: "unformattable_derive_field", succeeds: true, diagnostic: "" },
    Case { name: "generic_attribute_rejected", succeeds: false, diagnostic: "concrete type/const" },
    Case { name: "duplicate_default", succeeds: false, diagnostic: "duplicate default" },
];

/// Whether the final executable carries the inlined dbgvis bridge (prologue marker).
fn embedded_gdb_script(binary: &Path) -> Result<bool> {
    let bytes = std::fs::read(binary)
        .map_err(|error| Failure(format!("read {}: {error}", binary.display())))?;
    Ok(bytes
        .windows(b"types.ModuleType('dbgvis_gdb')".len())
        .any(|window| window == b"types.ModuleType('dbgvis_gdb')"))
}

fn facade() -> String {
    root().join("crates/dbgvis").display().to_string()
}

fn case_source(name: &str) -> Result<String> {
    let path = root().join("tests/cases").join(format!("{name}.rs"));
    std::fs::read_to_string(&path)
        .map_err(|error| Failure(format!("read {}: {error}", path.display())))
}

fn fixture(relative: &str) -> Result<String> {
    let path = root().join("tests/cases").join(relative);
    std::fs::read_to_string(&path)
        .map_err(|error| Failure(format!("read {}: {error}", path.display())))
}

pub fn run() -> Result {
    let scratch = TempDir::new("dbgvis consumer ")?;
    let project = scratch.path();
    write(
        project.join("Cargo.toml"),
        &format!(
            r#"[package]
name = "dbgvis-compile-consumer"
version = "0.0.0"
edition = "2024"
[workspace]
[dependencies]
dv = {{ package = "dbgvis", path = "{}", features = ["derive"] }}
"#,
            facade()
        ),
    )?;
    let target = root().join("target/compiler-contracts");

    for case in CASES {
        write(project.join("src/main.rs"), &case_source(case.name)?)?;
        // Debug and optimized builds must both enforce the auto fallback contract.
        for profile in [&[][..], &["--release"][..]] {
            let mut build = Run::new("cargo")
                .args(["+nightly", "build", "--offline"])
                .args(profile)
                .cwd(project)
                .env("CARGO_TARGET_DIR", &target);
            if !case.succeeds {
                build = build.expect_failure();
            }
            let output = build.output()?;
            ensure!(
                output.contains(case.diagnostic),
                "{}: missing diagnostic {:?}\n{output}",
                case.name,
                case.diagnostic
            );
            if case.succeeds {
                let folder = if profile.is_empty() {
                    "debug"
                } else {
                    "release"
                };
                Run::new(target.join(folder).join("dbgvis-compile-consumer"))
                    .timeout(10)
                    .check()?;
            }
        }
        println!("{}: OK", case.name);
    }

    check_features(project)?;
    drop(scratch);
    check_dependency_identity()?;
    println!("COMPILER_CONTRACTS_OK");
    Ok(())
}

/// Keep opt-out coverage in a consumer project; the repo examples are always enabled.
fn check_features(project: &Path) -> Result {
    for name in ["a", "b", "c"] {
        let crate_dir = project.join(name);
        write(
            crate_dir.join("src/lib.rs"),
            &fixture(&format!("feature/{name}.rs"))?,
        )?;
        write(
            crate_dir.join("Cargo.toml"),
            &format!(
                r#"[package]
name = "demo-{name}"
version = "0.0.0"
edition = "2024"
[features]
default = []
visualize = ["dep:dbgvis"]
[dependencies]
dbgvis = {{ path = "{}", optional = true, features = ["derive"] }}
"#,
                facade()
            ),
        )?;
    }
    write(
        project.join("Cargo.toml"),
        &format!(
            r#"[package]
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
dbgvis = {{ path = "{facade}", optional = true, features = ["derive"] }}
demo-a = {{ path = "{a}" }}
demo-b = {{ path = "{b}" }}
demo-c = {{ path = "{c}" }}
"#,
            facade = facade(),
            a = project.join("a").display(),
            b = project.join("b").display(),
            c = project.join("c").display(),
        ),
    )?;
    write(project.join("src/main.rs"), &fixture("feature/main.rs")?)?;

    for (enabled, release) in [(false, false), (true, false), (true, true)] {
        let target =
            root()
                .join("target/feature-contracts")
                .join(if enabled { "on" } else { "off" });
        let feature: &[&str] = if enabled {
            &["--features", "visualize"]
        } else {
            &["--no-default-features"]
        };
        let profile: &[&str] = if release { &["--release"] } else { &[] };
        let toolchain = if enabled { "+nightly" } else { "+stable" };

        Run::new("cargo")
            .arg(toolchain)
            .args(["build", "--offline"])
            .args(feature)
            .args(profile)
            .args(["--target-dir"])
            .arg(&target)
            .cwd(project)
            .check()?;
        let binary = target
            .join(if release { "release" } else { "debug" })
            .join("dbgvis-feature-consumer");
        Run::new(&binary).timeout(10).check()?;

        let tree = Run::new("cargo")
            .arg(toolchain)
            .args(["tree", "--offline"])
            .args(feature)
            .cwd(project)
            .output()?;
        let symbols = Run::new("nm").arg(&binary).output()?;

        if enabled {
            ensure!(tree.contains("linkme"), "enabled tree lacks linkme\n{tree}");
            for symbol in [
                "DBG_VIS_MODULE_V2",
                "__DBG_ANCHOR_UnusedRegistered",
                "__DBG_ANCHOR_Item",
                "__DBG_ANCHOR_State",
            ] {
                ensure!(symbols.contains(symbol), "enabled build lacks {symbol}");
            }
            ensure!(
                embedded_gdb_script(&binary)? == EMITS_GDB_SECTION,
                "debuginfo builds must embed the bridge"
            );
        } else {
            for name in ["dbgvis v", "dbgvis-runtime", "dbgvis-macros", "linkme"] {
                ensure!(
                    !tree.contains(name),
                    "disabled tree still has {name}\n{tree}"
                );
            }
            for name in [
                "DBG_VIS_MODULE",
                "dbgvis_dispatch",
                "__DBG_RUNTIME",
                "dbgvis_runtime",
                "linkme",
                "__DBG_ANCHOR",
                "__DBG_REGISTRATION",
            ] {
                ensure!(!symbols.contains(name), "disabled build still has {name}");
            }
            ensure!(
                !embedded_gdb_script(&binary)?,
                "disabled build embedded the bridge"
            );
        }
    }
    println!("TEMPORARY_CROSS_CRATE_DEBUG_LTO_OK");
    println!("FEATURE_DISABLED_OK");

    // rustc emits `.debug_gdb_scripts` only when the final crate has debuginfo. A consumer's
    // default release profile (debug = 0) keeps the Rust registry but silently loses the bridge.
    let target = root().join("target/feature-contracts/no-debuginfo");
    Run::new("cargo")
        .args([
            "+nightly",
            "build",
            "--offline",
            "--features",
            "visualize",
            "--release",
            "--target-dir",
        ])
        .arg(&target)
        .cwd(project)
        .env("CARGO_PROFILE_RELEASE_DEBUG", "0")
        .check()?;
    let binary = target.join("release/dbgvis-feature-consumer");
    Run::new(&binary).timeout(10).check()?;
    let symbols = Run::new("nm").arg(&binary).output()?;
    ensure!(
        symbols.contains("DBG_VIS_MODULE_V2"),
        "debug = 0 build lost the Rust registry"
    );
    ensure!(
        !embedded_gdb_script(&binary)?,
        "debug = 0 must not embed the bridge; the guide documents this"
    );
    println!("NO_DEBUGINFO_DROPS_GDB_SCRIPT_OK");
    Ok(())
}

/// Two versions with identical names and layouts must never share formatters.
fn check_dependency_identity() -> Result {
    let scratch = TempDir::new("dbgvis identity ")?;
    let project = scratch.path();
    for version in [1, 2] {
        let library = project.join(format!("v{version}"));
        write(
            library.join("Cargo.toml"),
            &format!(
                r#"[package]
name = "identity-library"
version = "{version}.0.0"
edition = "2024"
[lib]
path = "lib.rs"
[dependencies]
dv = {{ package = "dbgvis", path = "{}", features = ["derive"] }}
"#,
                facade()
            ),
        )?;
        write(library.join("lib.rs"), &fixture("identity/lib.rs")?)?;
    }
    write(
        project.join("Cargo.toml"),
        &format!(
            r#"[package]
name = "identity-consumer"
version = "0.0.0"
edition = "2024"
[workspace]
exclude = ["v1", "v2"]
[dependencies]
dv = {{ package = "dbgvis", path = "{}", features = ["derive"] }}
old = {{ package = "identity-library", path = "v1" }}
new = {{ package = "identity-library", path = "v2" }}
[profile.release]
lto = true
codegen-units = 1
"#,
            facade()
        ),
    )?;
    write(project.join("src/main.rs"), &fixture("identity/main.rs")?)?;

    let target = root().join("target/identity-contracts");
    for (profile, folder) in [(&[][..], "debug"), (&["--release"][..], "release")] {
        Run::new("cargo")
            .args(["+nightly", "build", "--offline"])
            .args(profile)
            .args(["--target-dir"])
            .arg(&target)
            .cwd(project)
            .check()?;
        Run::new(target.join(folder).join("identity-consumer"))
            .timeout(10)
            .check()?;
    }
    println!("CROSS_VERSION_IDENTITY_REJECTED_OK");
    Ok(())
}
