//! `cargo dbgvis` -- install and manage the experimental auto-registration driver so a
//! project can use it without cloning the dbgvis repository.
//!
//! The driver, its `RUSTC_WORKSPACE_WRAPPER`, and its dependency-tracking source are
//! embedded here at build time, so an installed `cargo-dbgvis` binary is self-contained.
//! `setup` writes them into a home directory (`DBGVIS_HOME`, else `<cache>/dbgvis`) and
//! compiles the driver there; the wrapper resolves everything relative to that home when
//! `DBGVIS_HOME` is set. Auto-registration is a strict, experimental backend -- the core
//! visualizer needs none of this and works from a plain `dbgvis` dependency.

use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, ExitCode};

// Embedded at build time from the driver's own sources.
const WRAPPER_PY: &str = include_str!("../../../tools/auto-register/wrapper.py");
const DRIVER_RS: &str = include_str!("../../../tools/auto-register/driver.rs");
const TRACKED_RS: &str = include_str!("../../../tools/auto-register/tracked.rs");

/// The driver links `rustc_private`, which has no stable API, so it is pinned to the
/// exact compiler it was written against.
const EXPECTED_RUSTC: &str = "rustc 1.99.0-nightly (12c36e253 2026-08-10)";

const USAGE: &str = "\
cargo dbgvis <command>

  setup     compile the driver into DBGVIS_HOME and write its wrapper
  doctor    check the toolchain, rustc-dev, and the installed driver
  home      print DBGVIS_HOME
  config    print a .cargo/config.toml snippet for a project (needs BIN)
  uninstall remove DBGVIS_HOME (the driver, wrapper, and scan plans)
  help      this message

DBGVIS_HOME defaults to $XDG_CACHE_HOME/dbgvis (or ~/.cache/dbgvis).
The core dbgvis visualizer does not need any of this -- add the `dbgvis`
dependency, `enable!()`, and register roots by hand; the driver only removes
the hand-written `register_type!` lines.";

fn home() -> PathBuf {
    if let Some(dir) = env::var_os("DBGVIS_HOME") {
        return PathBuf::from(dir);
    }
    let cache = env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env::var_os("HOME").expect("HOME must be set")).join(".cache")
        });
    cache.join("dbgvis")
}

fn rustc_version() -> Result<String, String> {
    let out = Command::new("rustc")
        .arg("--version")
        .output()
        .map_err(|error| format!("cannot run rustc: {error}"))?;
    if !out.status.success() {
        return Err("rustc --version failed".into());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// Whether the active toolchain carries rustc-dev (the `rustc_*` crates the driver
/// links). Absence is the most common setup failure, so name it precisely.
fn has_rustc_dev() -> bool {
    let Ok(out) = Command::new("rustc").args(["--print", "sysroot"]).output() else {
        return false;
    };
    if !out.status.success() {
        return false;
    }
    let sysroot = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim().to_owned());
    // librustc_driver ships only with the rustc-dev component.
    let libdir = sysroot.join("lib/rustlib").join(host_triple()).join("lib");
    fs::read_dir(&libdir)
        .map(|entries| {
            entries.flatten().any(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("librustc_driver")
            })
        })
        .unwrap_or(false)
}

fn host_triple() -> String {
    // `rustc -vV` reports the host triple; used to find the rustc-dev lib dir.
    Command::new("rustc")
        .arg("-vV")
        .output()
        .ok()
        .and_then(|out| {
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .find_map(|line| line.strip_prefix("host: ").map(str::to_owned))
        })
        .unwrap_or_default()
}

fn setup() -> Result<(), String> {
    let version = rustc_version()?;
    if version != EXPECTED_RUSTC {
        return Err(format!(
            "unsupported toolchain: {version}\n\
             the driver requires exactly {EXPECTED_RUSTC} with rustc-dev.\n\
             install it, e.g. `rustup toolchain install nightly-2026-08-10 --component rustc-dev`,\n\
             and run `cargo dbgvis setup` under that toolchain (rustup override or +toolchain)."
        ));
    }
    if !has_rustc_dev() {
        return Err("rustc-dev is not installed for the active toolchain\n\
                    add it: `rustup component add rustc-dev`"
            .into());
    }
    let home = home();
    fs::create_dir_all(&home).map_err(|error| format!("create {}: {error}", home.display()))?;
    let write = |name: &str, contents: &str| -> Result<PathBuf, String> {
        let path = home.join(name);
        fs::write(&path, contents).map_err(|error| format!("write {}: {error}", path.display()))?;
        Ok(path)
    };
    let wrapper = write("wrapper.py", WRAPPER_PY)?;
    write("driver.rs", DRIVER_RS)?;
    write("tracked.rs", TRACKED_RS)?;
    make_executable(&wrapper)?;

    // Compile the driver next to its sources.
    let driver = home.join("driver");
    let status = Command::new("rustc")
        .arg(home.join("driver.rs"))
        .args(["--edition=2024", "-C", "rpath=yes", "-D", "warnings", "-o"])
        .arg(&driver)
        .status()
        .map_err(|error| format!("compile driver: {error}"))?;
    if !status.success() {
        return Err("compiling the driver failed".into());
    }

    println!("dbgvis: installed to {}", home.display());
    println!("\nAdd to your project's .cargo/config.toml:\n");
    print!("{}", config_snippet(&home, "<your-bin-name>"));
    Ok(())
}

#[cfg(unix)]
fn make_executable(path: &std::path::Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = fs::metadata(path)
        .map_err(|error| format!("stat {}: {error}", path.display()))?
        .permissions();
    perms.set_mode(perms.mode() | 0o111);
    fs::set_permissions(path, perms).map_err(|error| format!("chmod {}: {error}", path.display()))
}
#[cfg(not(unix))]
fn make_executable(_path: &std::path::Path) -> Result<(), String> {
    Ok(())
}

fn config_snippet(home: &std::path::Path, bin: &str) -> String {
    format!(
        "[build]\n\
         rustc-workspace-wrapper = {wrapper:?}\n\n\
         [env]\n\
         DBGVIS_HOME = {home:?}\n\
         DBGVIS_AUTO_CRATE = {bin:?}\n",
        wrapper = home.join("wrapper.py").display().to_string(),
        home = home.display().to_string(),
    )
}

fn doctor() -> Result<(), String> {
    let mut ok = true;
    let check = |label: &str, good: bool, detail: &str| {
        println!(
            "  [{}] {label}{}",
            if good { "ok" } else { "!!" },
            if detail.is_empty() {
                String::new()
            } else {
                format!(" -- {detail}")
            }
        );
        good
    };
    match rustc_version() {
        Ok(version) => {
            ok &= check("toolchain", version == EXPECTED_RUSTC, &version);
            if version != EXPECTED_RUSTC {
                println!("      expected {EXPECTED_RUSTC}");
            }
        }
        Err(error) => ok &= check("toolchain", false, &error),
    }
    ok &= check("rustc-dev", has_rustc_dev(), "librustc_driver in sysroot");
    let home = home();
    let driver = home.join("driver");
    ok &= check("driver", driver.exists(), &driver.display().to_string());
    if driver.exists() {
        let wrapper = home.join("wrapper.py");
        let fresh = [wrapper.as_path(), &home.join("driver.rs")]
            .iter()
            .filter_map(|p| fs::metadata(p).and_then(|m| m.modified()).ok())
            .all(|source| {
                fs::metadata(&driver)
                    .and_then(|m| m.modified())
                    .map(|built| built >= source)
                    .unwrap_or(false)
            });
        ok &= check("driver up to date", fresh, "newer than its sources");
    }
    if ok {
        println!("\ndbgvis: ready. `cargo dbgvis config <bin>` prints the project config.");
        Ok(())
    } else {
        Err("some checks failed; run `cargo dbgvis setup`".into())
    }
}

fn run(args: &[OsString]) -> Result<(), String> {
    let command = args.first().and_then(|a| a.to_str()).unwrap_or("help");
    match command {
        "setup" => setup(),
        "doctor" => doctor(),
        "home" => {
            println!("{}", home().display());
            Ok(())
        }
        "config" => {
            let bin = args
                .get(1)
                .and_then(|a| a.to_str())
                .ok_or("usage: cargo dbgvis config <bin-name>")?;
            print!("{}", config_snippet(&home(), bin));
            Ok(())
        }
        "uninstall" => {
            let home = home();
            // Only remove a directory that looks like ours, never an arbitrary path a
            // stray DBGVIS_HOME might point at.
            if !home.join("driver.rs").exists() && !home.join("wrapper.py").exists() {
                return Err(format!(
                    "{} is not a dbgvis install (no driver.rs/wrapper.py); refusing to remove it",
                    home.display()
                ));
            }
            fs::remove_dir_all(&home)
                .map_err(|error| format!("remove {}: {error}", home.display()))?;
            println!("dbgvis: removed {}", home.display());
            Ok(())
        }
        "help" | "--help" | "-h" => {
            println!("{USAGE}");
            Ok(())
        }
        other => Err(format!("unknown command `{other}`\n\n{USAGE}")),
    }
}

fn main() -> ExitCode {
    // Invoked as `cargo dbgvis ...`, Cargo runs `cargo-dbgvis dbgvis ...`; drop that.
    let mut args: Vec<OsString> = env::args_os().skip(1).collect();
    if args.first().map(|a| a == "dbgvis").unwrap_or(false) {
        args.remove(0);
    }
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("cargo-dbgvis: {error}");
            ExitCode::FAILURE
        }
    }
}
