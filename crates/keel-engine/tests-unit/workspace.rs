// SPDX-License-Identifier: Apache-2.0
//! Unit tests for `workspace` (relocated out of src; included via #[path]).
//!
//! Focus: the layered loader (spec section 8.5) reads each composition layer
//! present, tags it with its authority position, and orders them by the fixed
//! chain of section 7.2 — while a flat single-project workspace still loads.

use super::*;
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

const WS_YAML: &str =
    "apiVersion: keel/v1alpha1\nkind: Workspace\nmetadata: { id: it }\nspec: {}\n";

fn rule_yaml(id: &str) -> String {
    format!(
        "apiVersion: keel/v1alpha1\nkind: Rule\n\
         metadata: {{ id: {id}, author: t, adrRef: \"adr:ADR-1\", reviewAfter: P6M }}\n\
         spec:\n  on: [file.edited]\n  enforcement: {{ valid: {{ decision: allow }} }}\n"
    )
}

/// A throwaway workspace root under the OS temp dir.
struct TmpWs(PathBuf);

impl Drop for TmpWs {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn tmp_ws() -> TmpWs {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let root = std::env::temp_dir().join(format!("keel-ws-it-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("workspace.yaml"), WS_YAML).unwrap();
    TmpWs(root)
}

/// Writes `rules/<file>.yaml` under `dir`, creating `rules/` as needed.
fn write_rule(dir: &Path, file: &str, id: &str) {
    let rules = dir.join("rules");
    std::fs::create_dir_all(&rules).unwrap();
    std::fs::write(rules.join(format!("{file}.yaml")), rule_yaml(id)).unwrap();
}

#[test]
fn layer_order_is_the_fixed_composition_chain() {
    // section 7.2: global is the highest authority, profile the lowest.
    // Package (invariant 3) sits below the base layers and above the project.
    assert!(LayerId::Global < LayerId::Organization);
    assert!(LayerId::Organization < LayerId::Platform);
    assert!(LayerId::Platform < LayerId::Package);
    assert!(LayerId::Package < LayerId::Project);
    assert!(LayerId::Project < LayerId::Team);
    assert!(LayerId::Team < LayerId::Profile);
    assert_eq!(LayerId::CHAIN.len(), 7);
}

#[test]
fn packages_compose_as_a_technology_layer_between_platform_and_project() {
    // invariant 3: a `packages/<tech>/` bundle loads like any namespaced layer
    // and composes below the project — the SDK wiring that lets technology
    // content (Flutter, Rust) actually reach the model, not sit inert.
    let ws = tmp_ws();
    write_rule(&ws.0.join("global"), "g", "global.no-drop");
    write_rule(
        &ws.0.join("packages").join("flutter"),
        "f",
        "flutter.widget-classes",
    );
    write_rule(&ws.0.join("projects").join("demo"), "p", "demo.local");

    let layered = load_layered(&ws.0).expect("layered load");
    let ids: Vec<LayerId> = layered.layers.iter().map(|l| l.id).collect();
    assert_eq!(
        ids,
        vec![LayerId::Global, LayerId::Package, LayerId::Project],
        "package composes between global/base and the project"
    );
    let pkg = layered
        .layers
        .iter()
        .find(|l| l.id == LayerId::Package)
        .expect("package layer present");
    assert_eq!(
        pkg.name.as_deref(),
        Some("flutter"),
        "namespaced by technology"
    );
    assert_eq!(pkg.files.rules[0].metadata.id, "flutter.widget-classes");
}

#[test]
fn layered_loads_global_and_named_project() {
    let ws = tmp_ws();
    write_rule(&ws.0.join("global"), "g", "global.no-drop");
    write_rule(&ws.0.join("projects").join("demo"), "p", "demo.local");

    let layered = load_layered(&ws.0).expect("layered load");
    assert_eq!(layered.layers.len(), 2, "global + one project");

    let global = &layered.layers[0];
    assert_eq!(global.id, LayerId::Global);
    assert_eq!(global.name, None, "global is unnamed");
    assert_eq!(global.files.rules[0].metadata.id, "global.no-drop");

    let project = &layered.layers[1];
    assert_eq!(project.id, LayerId::Project);
    assert_eq!(project.name.as_deref(), Some("demo"));
    assert_eq!(project.files.rules[0].metadata.id, "demo.local");
}

#[test]
fn layers_are_ordered_global_before_project_regardless_of_creation() {
    let ws = tmp_ws();
    // Create the project layer FIRST; ordering must still put global first.
    write_rule(&ws.0.join("projects").join("z-app"), "p", "z.rule");
    write_rule(&ws.0.join("global"), "g", "g.rule");

    let layered = load_layered(&ws.0).unwrap();
    let ids: Vec<LayerId> = layered.layers.iter().map(|l| l.id).collect();
    assert_eq!(ids, vec![LayerId::Global, LayerId::Project]);
}

#[test]
fn multiple_project_instances_are_sorted_by_name() {
    let ws = tmp_ws();
    write_rule(&ws.0.join("projects").join("beta"), "b", "beta.rule");
    write_rule(&ws.0.join("projects").join("alpha"), "a", "alpha.rule");

    let layered = load_layered(&ws.0).unwrap();
    let names: Vec<Option<&str>> = layered.layers.iter().map(|l| l.name.as_deref()).collect();
    assert_eq!(names, vec![Some("alpha"), Some("beta")]);
}

#[test]
fn absent_layers_are_skipped() {
    let ws = tmp_ws();
    write_rule(&ws.0.join("global"), "g", "only.global");

    let layered = load_layered(&ws.0).unwrap();
    assert_eq!(layered.layers.len(), 1);
    assert_eq!(layered.layers[0].id, LayerId::Global);
}

#[test]
fn flat_workspace_loads_as_single_degenerate_project() {
    // A pre-existing flat workspace (rules/ under the root, no layer dirs) must
    // keep working: it composes as one Project layer.
    let ws = tmp_ws();
    write_rule(&ws.0, "r", "flat.rule");

    let layered = load_layered(&ws.0).unwrap();
    assert_eq!(layered.layers.len(), 1);
    assert_eq!(layered.layers[0].id, LayerId::Project);
    assert_eq!(layered.layers[0].name, None);
    assert_eq!(layered.layers[0].files.rules[0].metadata.id, "flat.rule");
}

#[test]
fn mixed_layout_is_rejected_not_silently_dropped() {
    // Root components AND a layer dir: ambiguous. Must error rather than
    // silently ignore the root rules/ (the target failure mode).
    let ws = tmp_ws();
    write_rule(&ws.0, "r", "root.rule");
    write_rule(&ws.0.join("global"), "g", "global.rule");

    assert!(
        matches!(load_layered(&ws.0), Err(WorkspaceError::MixedLayout(_))),
        "a workspace mixing root components with layer dirs must be rejected"
    );
}

#[test]
fn org_native_components_with_real_content_is_rejected() {
    // section 8.5 org-native `components/` is not loadable yet: REAL authored
    // content there is rejected loudly instead of read as empty.
    let ws = tmp_ws();
    let policies = ws.0.join("organizations/my-company/components/policies");
    std::fs::create_dir_all(&policies).unwrap();
    std::fs::write(policies.join("p.yaml"), "apiVersion: keel/v1alpha1\n").unwrap();

    assert!(
        matches!(
            load_layered(&ws.0),
            Err(WorkspaceError::UnsupportedLayerLayout { .. })
        ),
        "authored content under components/ must not be silently read as empty"
    );
}

#[test]
fn org_native_components_docs_only_is_tolerated() {
    // The `keel init` scaffold ships components/ with only a README (+ .example)
    // — a documented placeholder with nothing to drop. It must NOT error.
    let ws = tmp_ws();
    let comp = ws.0.join("organizations/my-company/components/policies");
    std::fs::create_dir_all(&comp).unwrap();
    std::fs::write(comp.join("README.md"), "org-scale, deferred\n").unwrap();
    std::fs::write(comp.join("policy.yaml.example"), "# template\n").unwrap();

    let layered = load_layered(&ws.0).expect("docs-only components/ is tolerated");
    // The org layer loads (empty of rules); no error.
    assert!(layered.layers.iter().any(|l| l.id == LayerId::Organization));
}

#[test]
fn flat_fallback_triggers_on_any_component_dir_not_just_rules() {
    // A flat workspace with tools/ but no rules/ still loads as one Project
    // layer (the fallback keys on any component subdir).
    let ws = tmp_ws();
    let tools = ws.0.join("tools");
    std::fs::create_dir_all(&tools).unwrap();
    std::fs::write(
        tools.join("t.yaml"),
        "apiVersion: keel/v1alpha1\nkind: Tool\nmetadata: { id: t.x }\nspec: { command: [\"true\"], output: exit-code }\n",
    )
    .unwrap();

    let layered = load_layered(&ws.0).unwrap();
    assert_eq!(layered.layers.len(), 1);
    assert_eq!(layered.layers[0].id, LayerId::Project);
    assert_eq!(layered.layers[0].files.tools[0].metadata.id, "t.x");
}

#[test]
fn load_layered_requires_a_workspace_file() {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let root = std::env::temp_dir().join(format!("keel-ws-nows-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let guard = TmpWs(root.clone());
    assert!(
        matches!(
            load_layered(&guard.0),
            Err(WorkspaceError::NotAWorkspace(_))
        ),
        "a workspace root must carry workspace.yaml"
    );
}
