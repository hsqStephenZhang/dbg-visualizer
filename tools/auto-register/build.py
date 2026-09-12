"""Build only against the compiler revision audited by this experiment."""
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[2]
EXPECTED = "rustc 1.99.0-nightly (12c36e253 2026-08-10)"


def main():
    actual = subprocess.check_output(["rustc", "--version"], text=True).strip()
    if actual != EXPECTED:
        raise SystemExit(f"Unsupported driver toolchain: {actual}; expected {EXPECTED} with rustc-dev")
    output = ROOT / "target/auto-register/driver"
    output.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run(["rustc", ROOT / "tools/auto-register/driver.rs", "--edition=2024",
                    "-C", "rpath=yes", "-D", "warnings", "-o", output], check=True)
    print(f"AUTO_DRIVER_BUILT {actual}")


if __name__ == "__main__":
    main()
