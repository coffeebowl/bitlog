//! Turns the SemVer package version back into the calendar version,
//! 26.9.0 into 26.09 and 26.9.1 into 26.09.1, see the workspace Cargo.toml.

use std::env;

fn main() {
    let part = |name| env::var(name).expect("Cargo sets the version parts");
    let (year, month, patch) = (
        part("CARGO_PKG_VERSION_MAJOR"),
        part("CARGO_PKG_VERSION_MINOR"),
        part("CARGO_PKG_VERSION_PATCH"),
    );
    let mut version = format!("{year}.{month:0>2}");
    if patch != "0" {
        version = format!("{version}.{patch}");
    }
    println!("cargo::rustc-env=KNOTBOOK_VERSION={version}");
}
