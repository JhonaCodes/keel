// SPDX-License-Identifier: Apache-2.0
//! Shared path resolver for black-box tests of the published Keel binary.

use std::path::PathBuf;

/// The compiled `keel` binary, resolved like the other black-box tests
/// (`test/../target/<profile>/keel`), overridable by `KEEL_BIN`. Read in the
/// PARENT process, so `env_clear()` on the child never affects it.
pub fn keel_bin() -> PathBuf {
    if let Ok(p) = std::env::var("KEEL_BIN") {
        return PathBuf::from(p);
    }
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop(); // keel/
    p.push("target");
    p.push(if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    });
    p.push("keel");
    p
}
