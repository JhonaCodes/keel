// SPDX-License-Identifier: Apache-2.0
//! Unit tests for the Seatbelt profile generator (macOS only).

use super::seatbelt::profile;
use keel_engine::snapshot::CompiledContainment;
use std::path::Path;

fn containment() -> CompiledContainment {
    CompiledContainment {
        deny_unlink: vec!["**/*.md".to_string()],
        deny_write_outside: true,
        deny_network: true,
    }
}

#[test]
fn profile_denies_unlink_of_the_glob_and_confines_writes_and_network() {
    let p = profile(&containment(), Path::new("/work/space"));
    assert!(p.starts_with("(version 1)"));
    assert!(p.contains("(allow default)"));
    // The .md glob becomes a file-write-unlink deny.
    assert!(
        p.contains("file-write-unlink") && p.contains("md$"),
        "profile: {p}"
    );
    // Writes confined to the workspace subpath.
    assert!(p.contains("(deny file-write*)"));
    assert!(p.contains("(subpath \"/work/space\")"));
    // Network denied.
    assert!(p.contains("(deny network*)"));
}

#[test]
fn empty_containment_only_confines_what_is_declared() {
    let only_unlink = CompiledContainment {
        deny_unlink: vec!["*.env".to_string()],
        deny_write_outside: false,
        deny_network: false,
    };
    let p = profile(&only_unlink, Path::new("/w"));
    assert!(p.contains("file-write-unlink"));
    // Nothing beyond what was declared: no write-confinement, no network deny.
    assert!(!p.contains("(deny file-write*)"), "profile: {p}");
    assert!(!p.contains("network"), "profile: {p}");
}
