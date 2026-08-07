// SPDX-License-Identifier: Apache-2.0
//! Unit tests for the shim generator (relocated out of src; included via #[path]).

use super::*;

#[test]
fn generates_executable_shims_with_baked_paths_and_skips_missing_commands() {
    let dir = tempfile::tempdir().unwrap();
    let host_dir = dir.path().join("host");
    std::fs::create_dir_all(&host_dir).unwrap();
    let shim_bin = dir.path().join("keel-shim");
    let socket = host_dir.join("broker.sock");

    let commands = vec![
        "rm".to_string(),
        "definitely-not-a-real-command-xyz".to_string(),
    ];
    let bin_dir = generate(&host_dir, &shim_bin, &socket, &commands).unwrap();

    // `rm` exists on every Unix: its shim is generated, executable, and every
    // identity is BAKED into the script (no env-var indirection).
    let rm_shim = bin_dir.join("rm");
    let content = std::fs::read_to_string(&rm_shim).unwrap();
    assert!(content.contains(&shim_bin.display().to_string()));
    assert!(content.contains(&socket.display().to_string()));
    assert!(content.contains("--real"));
    assert!(!content.contains("$KEEL"), "identity never travels via env");
    let mode = std::fs::metadata(&rm_shim).unwrap().permissions().mode();
    assert_ne!(mode & 0o111, 0, "the shim must be executable");

    // A command that does not exist on the machine is skipped, not faked.
    assert!(!bin_dir.join("definitely-not-a-real-command-xyz").exists());

    // The dir itself is sealed.
    let dir_mode = std::fs::metadata(&bin_dir).unwrap().permissions().mode();
    assert_eq!(dir_mode & 0o777, 0o700, "shim dir is 0700");
}

#[test]
fn resolve_real_never_returns_the_shim_dir_itself() {
    let dir = tempfile::tempdir().unwrap();
    let shim_dir = dir.path().join("bin");
    std::fs::create_dir_all(&shim_dir).unwrap();
    // Fake `rm` inside the (excluded) shim dir.
    std::fs::write(shim_dir.join("rm"), "#!/bin/sh\n").unwrap();
    std::fs::set_permissions(shim_dir.join("rm"), std::fs::Permissions::from_mode(0o755)).unwrap();

    let real = resolve_real("rm", &shim_dir).expect("rm exists on the system PATH");
    assert_ne!(
        real.parent().unwrap(),
        shim_dir,
        "a shim must never exec another shim"
    );
}
