//! Licence audit, run as a test rather than remembered.
//!
//! A shipped binary statically links five hundred crates. "See Cargo.toml"
//! is not a licence notice for that, and two of those crates —
//! `option-ext` (MPL-2.0) and `webpki-roots` (CDLA-Permissive-2.0) — carry
//! licences that require the notice to travel with the distribution.
//!
//! Two checks, neither of which needs a network:
//!
//! 1. **The inventory matches the lockfile.** A dependency that appears
//!    without being recorded fails the build, so nobody can add a crate
//!    without its licence being looked at. Regenerate with
//!    `python3 packaging/collect-licenses.py`.
//! 2. **Every licence is one this project decided it can ship.** The
//!    allow-list below is a decision, written down. A crate under a licence
//!    that is not on it fails, and the failure names the crate.
//!
//! What this does not do is interpret licences. It ensures a human sees a
//! new one; it does not tell them what to think.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[derive(serde::Deserialize)]
struct Inventory {
    schema_version: u32,
    packages: Vec<Package>,
}

#[derive(serde::Deserialize)]
struct Package {
    name: String,
    version: String,
    license: Option<String>,
}

fn inventory() -> Inventory {
    let path = root().join("packaging/THIRD-PARTY-LICENSES.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    serde_json::from_str(&text).expect("THIRD-PARTY-LICENSES.json must parse")
}

/// Package name and version of everything Cargo.lock names, minus this
/// workspace's own crates — which are the entries with no `source`.
fn locked_packages() -> BTreeSet<(String, String)> {
    let path = root().join("Cargo.lock");
    let text = std::fs::read_to_string(&path).expect("Cargo.lock must exist");
    let mut packages = BTreeSet::new();
    let mut name: Option<String> = None;
    let mut version: Option<String> = None;
    let mut has_source = false;
    for line in text.lines().chain(std::iter::once("[[package]]")) {
        let line = line.trim();
        if line == "[[package]]" {
            if let (Some(n), Some(v)) = (name.take(), version.take()) {
                if has_source {
                    packages.insert((n, v));
                }
            }
            name = None;
            version = None;
            has_source = false;
        } else if let Some(rest) = line.strip_prefix("name = ") {
            name = Some(rest.trim_matches('"').to_string());
        } else if let Some(rest) = line.strip_prefix("version = ") {
            version = Some(rest.trim_matches('"').to_string());
        } else if line.starts_with("source = ") {
            has_source = true;
        }
    }
    packages
}

/// Licence expressions this project has decided it can ship.
///
/// All permissive. Two need naming in NOTICE and are named there:
/// **MPL-2.0** is file-level copyleft — shipping the crate unmodified is
/// fine, and modifying its files would oblige us to publish those changes;
/// **CDLA-Permissive-2.0** covers data (the Mozilla CA root store) and asks
/// for attribution.
///
/// Nothing here is GPL, AGPL or LGPL-only. `r-efi` offers LGPL-2.1-or-later
/// as one of three options and this project takes MIT, which is why the
/// whole expression is listed rather than the word "LGPL" being banned.
const ALLOWED: [&str; 33] = [
    "(MIT OR Apache-2.0) AND Unicode-3.0",
    "0BSD OR MIT OR Apache-2.0",
    "Apache-2.0",
    "Apache-2.0 AND ISC",
    "Apache-2.0 AND MIT",
    "Apache-2.0 OR BSL-1.0",
    "Apache-2.0 OR ISC OR MIT",
    "Apache-2.0 OR MIT",
    "Apache-2.0 WITH LLVM-exception",
    "Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT",
    "Apache-2.0/MIT",
    "BSD-2-Clause",
    "BSD-2-Clause OR Apache-2.0 OR MIT",
    "BSD-2-Clause OR MIT OR Apache-2.0",
    "BSD-3-Clause",
    "BSD-3-Clause OR Apache-2.0",
    "BSD-3-Clause OR MIT OR Apache-2.0",
    "BSL-1.0",
    "CDLA-Permissive-2.0",
    "ISC",
    "MIT",
    "MIT / Apache-2.0",
    "MIT OR Apache-2.0",
    "MIT OR Apache-2.0 OR LGPL-2.1-or-later",
    "MIT OR Apache-2.0 OR Zlib",
    "MIT OR Zlib OR Apache-2.0",
    "MIT/Apache-2.0",
    "MPL-2.0",
    "Unicode-3.0",
    "Unlicense OR MIT",
    "Unlicense/MIT",
    "Zlib",
    "Zlib OR Apache-2.0 OR MIT",
];

#[test]
fn the_inventory_describes_exactly_what_the_lockfile_locks() {
    let inventory = inventory();
    assert_eq!(inventory.schema_version, 1);
    let recorded: BTreeSet<(String, String)> = inventory
        .packages
        .iter()
        .map(|package| (package.name.clone(), package.version.clone()))
        .collect();
    let locked = locked_packages();

    let added: Vec<&(String, String)> = locked.difference(&recorded).collect();
    let removed: Vec<&(String, String)> = recorded.difference(&locked).collect();
    assert!(
        added.is_empty(),
        "these dependencies are locked but have no recorded licence — run \
         `python3 packaging/collect-licenses.py` and look at what it adds: {added:?}"
    );
    assert!(
        removed.is_empty(),
        "these packages are recorded but no longer locked — regenerate the inventory: {removed:?}"
    );
}

#[test]
fn every_dependency_ships_under_a_licence_this_project_accepts() {
    let allowed: BTreeSet<&str> = ALLOWED.into_iter().collect();
    let mut unknown: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut unlicensed = Vec::new();
    for package in inventory().packages {
        match package.license.as_deref() {
            Some(license) if allowed.contains(license) => {}
            Some(license) => unknown
                .entry(license.to_string())
                .or_default()
                .push(format!("{} {}", package.name, package.version)),
            None => unlicensed.push(format!("{} {}", package.name, package.version)),
        }
    }
    assert!(
        unlicensed.is_empty(),
        "these packages declare no licence at all and cannot be shipped without a decision: {unlicensed:?}"
    );
    assert!(
        unknown.is_empty(),
        "these licences have not been reviewed for this project. Read them, then add the \
         expression to ALLOWED if it may be shipped: {unknown:#?}"
    );
}

/// Copyleft and attribution licences oblige the distribution to carry a
/// notice. A crate under one of them must be named in NOTICE, not folded
/// into "the crates keep their own licences".
#[test]
fn licences_that_require_a_notice_are_named_in_notice() {
    let notice = std::fs::read_to_string(root().join("NOTICE")).expect("NOTICE must exist");
    let needs_naming = ["MPL-2.0", "CDLA-Permissive-2.0", "BSL-1.0"];
    let mut missing = Vec::new();
    for package in inventory().packages {
        let Some(license) = package.license.as_deref() else {
            continue;
        };
        if !needs_naming.contains(&license) {
            continue;
        }
        if !notice.contains(&package.name) {
            missing.push(format!("{} ({license})", package.name));
        }
    }
    assert!(
        missing.is_empty(),
        "these dependencies carry a licence that travels with the distribution and are not \
         named in NOTICE: {missing:?}"
    );
}

/// The lockfile parser above is small enough to be wrong quietly, so it is
/// checked against something known.
#[test]
fn the_lockfile_reader_finds_the_dependencies_it_should() {
    let locked = locked_packages();
    assert!(locked.len() > 100, "only {} packages parsed", locked.len());
    for expected in ["serde", "zip", "sha2"] {
        assert!(
            locked.iter().any(|(name, _)| name == expected),
            "{expected} missing from the parsed lockfile"
        );
    }
    // Our own crates have no `source` and must not be counted as third party.
    for ours in ["sciwhisper-core", "sciwhisper-update"] {
        assert!(
            !locked.iter().any(|(name, _)| name == ours),
            "{ours} is this workspace's own crate"
        );
    }
}
