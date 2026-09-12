//! Module/function/type visibility contracts for the auto-registration driver.
//!
//! Port of the former `tools/auto-register/visibility.py`. Each case becomes its own
//! generated `.rs` module so scopes never bleed into one another. The matrix answers
//! one question: which discovered concrete types can the driver *name* from the crate
//! root? The answer turns out to depend only on the type's own visibility.

use crate::harness::{Result, Run, TempDir, contains_word, root, write};
use std::fmt::Write as _;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Plain,
    Nested,
    Method,
    Generic,
    Root,
    Local,
}

struct Case {
    name: String,
    module: &'static str,
    function: &'static str,
    visibility: &'static str,
    inner: &'static str,
    accessible: bool,
    kind: Kind,
    reexport: bool,
    derive: bool,
    index: usize,
    typ: String,
}

impl Case {
    fn new(name: &str, kind: Kind, accessible: bool) -> Self {
        Self {
            name: name.to_owned(),
            module: "",
            function: "",
            visibility: "pub(crate)",
            inner: "",
            accessible,
            kind,
            reexport: false,
            derive: false,
            index: 0,
            typ: String::new(),
        }
    }
}

fn cases() -> Vec<Case> {
    let mut result = Vec::new();
    for module_public in [false, true] {
        for function_public in [false, true] {
            for (type_vis, spelling) in [
                ("private", ""),
                ("super", "pub(super)"),
                ("crate", "pub(crate)"),
                ("public", "pub"),
            ] {
                let name = format!(
                    "{}_{}_{type_vis}",
                    if module_public { "public" } else { "private" },
                    if function_public { "public" } else { "private" },
                );
                let mut case = Case::new(&name, Kind::Plain, type_vis != "private");
                case.module = if module_public { "pub" } else { "" };
                case.function = if function_public { "pub" } else { "" };
                case.visibility = spelling;
                result.push(case);
            }
        }
    }

    let mut nested_private = Case::new("nested_private", Kind::Nested, false);
    nested_private.inner = "";
    result.push(nested_private);

    // pub(super) on inner is declared in a root child: it grants the root access.
    let mut nested_super = Case::new("nested_super", Kind::Nested, true);
    nested_super.inner = "pub(super)";
    result.push(nested_super);

    // pub(super) on the type inside inner only reaches the outer module.
    let mut nested_type_super = Case::new("nested_type_super", Kind::Nested, false);
    nested_type_super.inner = "pub(crate)";
    nested_type_super.visibility = "pub(super)";
    result.push(nested_type_super);

    let mut nested_crate = Case::new("nested_crate", Kind::Nested, true);
    nested_crate.inner = "pub(crate)";
    result.push(nested_crate);

    let mut nested_reexport = Case::new("nested_reexport", Kind::Nested, true);
    nested_reexport.reexport = true;
    result.push(nested_reexport);

    result.push(Case::new("private_method", Kind::Method, true));
    result.push(Case::new("private_generic", Kind::Generic, true));
    result.push(Case::new("root_private", Kind::Root, true));
    result.push(Case::new("function_local", Kind::Local, false));

    // Existing derive registration works in the local scope even though the
    // driver cannot spell the type in its root-level generated module.
    let mut local_derive = Case::new("function_local_derive", Kind::Local, false);
    local_derive.derive = true;
    result.push(local_derive);

    let mut private_derive = Case::new("private_type_derive", Kind::Plain, false);
    private_derive.visibility = "";
    private_derive.derive = true;
    result.push(private_derive);

    for (index, case) in result.iter_mut().enumerate() {
        case.index = index;
        case.typ = format!(
            "Value{}",
            case.name
                .split('_')
                .map(|word| {
                    let mut chars = word.chars();
                    match chars.next() {
                        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                        None => String::new(),
                    }
                })
                .collect::<String>()
        );
    }
    result
}

fn source(case: &Case) -> String {
    let typ = &case.typ;
    let derive = if case.derive {
        "dv::Visualize"
    } else {
        "Debug"
    };
    let mut declaration = format!("#[derive({derive})] {} struct {typ}(u32);", case.visibility);
    let mut body = format!(
        r#"
    let value = {typ}(7);
    let values = vec![{typ}(8)];
    std::hint::black_box((&value, &values));
    crate::checkpoint({});
    std::hint::black_box((&value, &values));
"#,
        case.index
    );
    if case.kind == Kind::Local {
        declaration = declaration.replace("pub(crate) ", "");
        body = format!("{declaration}{body}");
        declaration = String::new();
    }

    let (worker, call) = match case.kind {
        Kind::Generic => {
            let body = body
                .replace(&format!("let value = {typ}(7);"), "let value = input;")
                .replace(
                    &format!("let values = vec![{typ}(8)];"),
                    "let values = vec![other];",
                );
            (
                format!("#[inline(never)] fn work<T>(input: T, other: T) {{{body}}}"),
                format!("work({typ}(7), {typ}(8));"),
            )
        }
        Kind::Method => (
            format!("struct Worker; impl Worker {{ #[inline(never)] fn work(&self) {{{body}}} }}"),
            "Worker.work();".to_owned(),
        ),
        _ => (
            format!("#[inline(never)] {} fn work() {{{body}}}", case.function),
            "work();".to_owned(),
        ),
    };

    let mut contents = format!("{declaration}\n{worker}\npub(crate) fn run() {{ {call} }}\n");
    if case.kind == Kind::Root {
        return contents
            .replace("pub(crate) struct", "struct")
            .replace("fn run()", "fn root_run()");
    }
    if case.kind == Kind::Nested {
        contents = format!(
            "{} mod inner {{\n{contents}\n}}\npub(crate) fn run() {{ inner::run(); }}\n",
            case.inner
        );
        if case.reexport {
            let _ = writeln!(contents, "pub(crate) use inner::{typ};");
        }
    }
    contents
}

/// GDB-side assertions. Generated per run because the case table is the input.
fn gdb_checks(cases: &[Case]) -> String {
    let mut rows = String::new();
    for case in cases {
        let _ = writeln!(
            rows,
            "    ({}, {:?}, {}, {}),",
            case.index,
            case.typ,
            if case.accessible { "True" } else { "False" },
            if case.derive { "True" } else { "False" },
        );
    }
    format!(
        r#"import gdb
import sys
gdb.execute("set pagination off")
gdb.execute("set confirm off")
gdb.execute("set debuginfod enabled off")
gdb.execute("break visibility_bin::checkpoint")
gdb.execute("run")
session = sys.modules["dbgvis_gdb"]._session
CASES = [
{rows}]
for index, typ, accessible, derive in CASES:
    assert int(gdb.parse_and_eval("case")) == index, (index, typ)
    gdb.execute("up")
    for expression in ("value", "values"):
        before = session.calls
        # Derive only registers the scalar, not Vec<PrivateOrLocal>.
        printable = accessible or (derive and expression == "value")
        if printable:
            actual = gdb.execute("dbgvis p " + expression, to_string=True).strip()
            expected = typ + ("(7)" if expression == "value" else "(8)")
            if expression == "values":
                expected = "[" + expected + "]"
            assert actual == expected, (typ, expression, actual)
            assert session.calls == before + 1, typ
        else:
            try:
                gdb.execute("dbgvis p " + expression, to_string=True)
                raise AssertionError(("unexpected registration", typ, expression))
            except gdb.error as error:
                assert "unregistered or ambiguous type" in str(error), (typ, str(error))
            assert session.calls == before, typ
    gdb.execute("continue")
assert gdb.selected_inferior().pid == 0
print("VISIBILITY_GDB_OK")
"#
    )
}

pub fn run(gdb: bool) -> Result {
    crate::autoregister::build_driver()?;
    contracts(gdb)
}

pub fn contracts(gdb: bool) -> Result {
    let matrix = cases();
    let scratch = TempDir::new("dbgvis visibility ")?;
    let project = scratch.path();
    let facade = root().join("crates/dbgvis").display().to_string();
    write(
        project.join("Cargo.toml"),
        &format!(
            r#"
[package]
name = "visibility_contract"
version = "0.0.0"
edition = "2024"
[workspace]
[[bin]]
name = "visibility_bin"
path = "src/main.rs"
[dependencies]
dv = {{ package = "dbgvis", path = "{facade}", features = ["derive"] }}
[profile.dev]
debug = 2
[profile.release]
debug = 2
lto = true
codegen-units = 1
"#
        ),
    )?;

    let mut declarations = String::new();
    let mut calls = String::new();
    for case in &matrix {
        if case.kind == Kind::Root {
            declarations.push_str(&source(case));
            calls.push_str("root_run();\n");
        } else {
            write(
                project.join("src").join(format!("{}.rs", case.name)),
                &source(case),
            )?;
            let _ = writeln!(declarations, "{} mod {};", case.module, case.name);
            let _ = writeln!(calls, "{}::run();", case.name);
        }
    }

    // Same Cargo package, but a separate crate. Neither this private helper
    // nor its public type should contribute candidates to the bin scan.
    write(
        project.join("src/lib.rs"),
        r#"
#[derive(Debug)] pub struct LibraryOnly(pub u64);
#[inline(never)] fn private_work() {
    let value = LibraryOnly(123);
    std::hint::black_box(&value);
}
pub fn run() { private_work(); }
"#,
    )?;
    write(
        project.join("src/unused.rs"),
        r#"
#[derive(Debug)] pub(crate) struct Uncalled(pub u64);
#[inline(never)] fn never_called() {
    let value = Uncalled(1);
    std::hint::black_box(&value);
}
"#,
    )?;
    write(
        project.join("src/main.rs"),
        &format!(
            r#"#![allow(dead_code, unused_imports)]
{declarations}
mod unused;
#[inline(never)] fn checkpoint(case: u32) {{ std::hint::black_box(case); }}
fn main() {{
    dv::enable!();
{calls}
    visibility_contract::run();
    println!("VISIBILITY_EXECUTED");
}}
"#
        ),
    )?;
    let script = project.join("checks.py");
    write(&script, &gdb_checks(&matrix))?;

    // Distinct bin name lets the wrapper select it without intercepting the lib.
    let target = root().join("target/auto-register/visibility");
    for (flags, folder) in [(&[][..], "debug"), (&["--release"][..], "release")] {
        let output = crate::autoregister::environment(
            Run::new("cargo")
                .args(["build", "--offline", "--bin", "visibility_bin"])
                .args(flags)
                .cwd(project)
                .timeout(600),
            "visibility_bin",
            &target,
        )
        .output()?;
        let (_, generated, report) = crate::autoregister::plan(&output)?;
        let rows: Vec<Vec<&str>> = report
            .lines()
            .map(|line| line.splitn(3, '\t').collect())
            .collect();

        for case in &matrix {
            let typ = &case.typ;
            let scalar: Vec<&Vec<&str>> = rows
                .iter()
                .filter(|row| row.len() == 3 && row[2].rsplit("::").next() == Some(typ.as_str()))
                .collect();
            let vector: Vec<&Vec<&str>> = rows
                .iter()
                .filter(|row| {
                    row.len() == 3 && row[2].contains("Vec<") && contains_word(row[2], typ)
                })
                .collect();
            let expected = if case.accessible { "CANDIDATE" } else { "SKIP" };
            for found in [&scalar, &vector] {
                ensure!(
                    found.len() == 1 && found[0][0] == expected,
                    "{}: expected one {expected} row, got {found:?}\n{report}",
                    case.name
                );
                if expected == "SKIP" {
                    ensure!(
                        found[0][1] == "type is not accessible/nameable from root",
                        "{}: wrong skip reason {:?}",
                        case.name,
                        found[0][1]
                    );
                }
            }
            ensure!(
                contains_word(&generated, typ) == case.accessible,
                "{}: generated source presence does not match accessibility",
                case.name
            );
            println!("VISIBILITY {folder} {}: {expected}", case.name);
        }
        for absent in ["LibraryOnly", "Uncalled"] {
            ensure!(
                !report.contains(absent) && !generated.contains(absent),
                "{absent} must not be scanned"
            );
        }

        let binary = target.join(folder).join("visibility_bin");
        Run::new(&binary)
            .marker("VISIBILITY_EXECUTED")
            .timeout(60)
            .check()?;
        if gdb {
            Run::new("rust-gdb")
                .args(["-q", "-nx", "--batch", "-iex"])
                .arg(format!("add-auto-load-safe-path {}", binary.display()))
                .arg("-x")
                .arg(&script)
                .arg("--args")
                .arg(&binary)
                .marker("VISIBILITY_GDB_OK")
                .timeout(300)
                .check()?;
        }
    }
    println!(
        "VISIBILITY_CONTRACTS_OK: {} cases, debug + LTO",
        matrix.len()
    );
    Ok(())
}
