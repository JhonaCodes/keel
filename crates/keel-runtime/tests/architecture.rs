use std::path::PathBuf;

fn runtime_source(name: &str) -> String {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(root.join("src").join(name)).unwrap()
}

#[test]
fn governed_runtime_does_not_import_authoring_dsl() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for entry in std::fs::read_dir(root.join("src")).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|value| value.to_str()) == Some("rs") {
            let source = std::fs::read_to_string(&path).unwrap();
            assert!(
                !source.contains("keel_dsl"),
                "runtime source imports authoring DSL: {}",
                path.display()
            );
        }
    }
}

#[test]
fn runtime_evidence_store_exposes_no_update_or_delete_sql() {
    let source = runtime_source("store.rs").to_ascii_uppercase();
    assert!(!source.contains("UPDATE "));
    assert!(!source.contains("DELETE "));
}
