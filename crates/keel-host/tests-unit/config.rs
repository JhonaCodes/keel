// SPDX-License-Identifier: Apache-2.0
//! Unit tests for the global default-workspace config (relocated; #[path]).
//!
//! These mutate the process `HOME`, so they run serially under one #[test] to
//! avoid racing other tests that read the environment.

use super::*;

#[test]
fn set_then_read_default_workspace_roundtrips_and_requires_a_real_workspace() {
    let home = tempfile::tempdir().unwrap();
    let ws = tempfile::tempdir().unwrap();
    // A real workspace has a workspace.yaml.
    std::fs::write(
        ws.path().join("workspace.yaml"),
        "apiVersion: keel/v1alpha1\n",
    )
    .unwrap();

    // Isolate HOME so we touch a temp ~/.keel, not the developer's.
    let previous = std::env::var_os("HOME");
    unsafe {
        std::env::set_var("HOME", home.path());
    }

    // Nothing registered yet.
    assert!(default_workspace().is_none());

    set_default_workspace(ws.path()).unwrap();
    let got = default_workspace().expect("a default is registered");
    assert_eq!(
        got.canonicalize().unwrap(),
        ws.path().canonicalize().unwrap()
    );

    // A registered path that no longer looks like a workspace is ignored.
    let gone = tempfile::tempdir().unwrap();
    set_default_workspace(gone.path()).unwrap();
    std::fs::remove_file(gone.path().join("workspace.yaml")).ok(); // never existed
    assert!(
        default_workspace().is_none(),
        "a path without workspace.yaml is not returned"
    );

    // Restore HOME for the rest of the suite.
    unsafe {
        match previous {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
    }
}
