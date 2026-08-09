// SPDX-License-Identifier: Apache-2.0
//! Unit tests for `resolution` (relocated out of src; included via #[path]).
//!
//! Resolution by repository identity (spec section 7.1): a binding selects the
//! ordered composition chain (global + matching org + matching project, in
//! section 7.2 order) and the identity check reports Verified/Advisory/Unregistered.

use super::*;
use crate::lock::ProjectBinding;
use crate::workspace::{LayerId, load_layered};
use std::path::{Path, PathBuf};
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

struct TmpWs(PathBuf);
impl Drop for TmpWs {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn tmp_ws() -> TmpWs {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let root = std::env::temp_dir().join(format!("keel-res-it-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("workspace.yaml"), WS_YAML).unwrap();
    TmpWs(root)
}

fn write_rule(dir: &Path, id: &str) {
    let rules = dir.join("rules");
    std::fs::create_dir_all(&rules).unwrap();
    std::fs::write(rules.join(format!("{id}.yaml")), rule_yaml(id)).unwrap();
}

fn write_registry(root: &Path, org: &str, body: &str) {
    let dir = root.join("organizations").join(org);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("repositories.yaml"), body).unwrap();
}

fn binding(project: &str, workspace: &str) -> ProjectBinding {
    ProjectBinding {
        project: project.to_string(),
        workspace: workspace.to_string(),
        platforms: Vec::new(),
    }
}

fn binding_with_platforms(project: &str, workspace: &str, platforms: &[&str]) -> ProjectBinding {
    ProjectBinding {
        project: project.to_string(),
        workspace: workspace.to_string(),
        platforms: platforms.iter().map(|p| p.to_string()).collect(),
    }
}

#[test]
fn resolves_global_org_and_project_in_composition_order() {
    let ws = tmp_ws();
    write_rule(&ws.0.join("global"), "global.rule");
    write_rule(&ws.0.join("organizations").join("nui"), "org.rule");
    write_rule(&ws.0.join("projects").join("con-app"), "project.rule");
    // A different org/project must NOT be pulled in.
    write_rule(&ws.0.join("projects").join("other"), "other.rule");
    write_rule(&ws.0.join("organizations").join("acme"), "acme.rule");

    let layered = load_layered(&ws.0).unwrap();
    let chain = resolve(&layered, &binding("project:nui/con-app", "org:nui"), None).unwrap();

    let picked: Vec<(LayerId, Option<String>)> = chain
        .layers(&layered)
        .map(|l| (l.id, l.name.clone()))
        .collect();
    assert_eq!(
        picked,
        vec![
            (LayerId::Global, None),
            (LayerId::Organization, Some("nui".to_string())),
            (LayerId::Project, Some("con-app".to_string())),
        ],
        "chain is global + matching org + matching project, in order"
    );
}

#[test]
fn non_org_workspace_ref_falls_back_to_project_org() {
    // A workspace ref that is not `org:<name>` shaped carries no org name, so
    // the owning org is taken from the project reference (`nui`).
    let ws = tmp_ws();
    write_rule(&ws.0.join("organizations").join("nui"), "org.rule");
    write_rule(&ws.0.join("projects").join("con-app"), "project.rule");

    let layered = load_layered(&ws.0).unwrap();
    let chain = resolve(&layered, &binding("project:nui/con-app", "local"), None).unwrap();
    let orgs: Vec<Option<String>> = chain
        .layers(&layered)
        .filter(|l| l.id == LayerId::Organization)
        .map(|l| l.name.clone())
        .collect();
    assert_eq!(orgs, vec![Some("nui".to_string())]);
}

#[test]
fn org_prefixed_workspace_ref_selects_that_org_literally() {
    // `org:local` names the org `local`; with no `organizations/local`, no org
    // layer is selected (the solo-dev case: chain is global + project).
    let ws = tmp_ws();
    write_rule(&ws.0.join("global"), "g.rule");
    write_rule(&ws.0.join("organizations").join("nui"), "org.rule");
    write_rule(&ws.0.join("projects").join("con-app"), "p.rule");

    let layered = load_layered(&ws.0).unwrap();
    let chain = resolve(&layered, &binding("project:nui/con-app", "org:local"), None).unwrap();
    let ids: Vec<LayerId> = chain.layers(&layered).map(|l| l.id).collect();
    assert_eq!(
        ids,
        vec![LayerId::Global, LayerId::Project],
        "org `local` has no directory, so only global + project apply"
    );
}

#[test]
fn flat_workspace_resolves_its_single_project_layer() {
    let ws = tmp_ws();
    write_rule(&ws.0, "flat.rule");

    let layered = load_layered(&ws.0).unwrap();
    let chain = resolve(&layered, &binding("project:me/app", "org:local"), None).unwrap();
    let ids: Vec<LayerId> = chain.layers(&layered).map(|l| l.id).collect();
    assert_eq!(
        ids,
        vec![LayerId::Project],
        "degenerate project layer applies"
    );
    assert!(chain.matched_project, "the project layer matched");
}

#[test]
fn binding_to_absent_project_is_reported_not_silent() {
    // A binding whose project has no matching layer resolves to a thin chain;
    // matched_project MUST flag it so the caller never treats "enforces
    // nothing" as a normal result (section 7.1).
    let ws = tmp_ws();
    write_rule(&ws.0.join("global"), "g.rule");

    let layered = load_layered(&ws.0).unwrap();
    let chain = resolve(&layered, &binding("project:me/ghost", "org:local"), None).unwrap();
    assert!(
        !chain.matched_project,
        "a bound project with no layer must be reported as unmatched"
    );
}

#[test]
fn registry_present_but_identity_unknown_is_advisory() {
    let ws = tmp_ws();
    write_registry(
        &ws.0,
        "nui",
        "apiVersion: keel/v1alpha1\nkind: RepositoryRegistry\nmetadata: { id: nui-repos }\n\
         spec:\n  repositories:\n    - provider: github\n      id: NuiMarkets/con-app\n      project: project:nui/con-app\n",
    );

    let layered = load_layered(&ws.0).unwrap();
    // Registry exists, but repo_identity is None → cannot verify → advisory,
    // distinct from a standalone workspace with no registry.
    let chain = resolve(&layered, &binding("project:nui/con-app", "org:nui"), None).unwrap();
    assert!(matches!(chain.identity, IdentityStatus::Advisory(_)));
}

#[test]
fn platform_team_profile_are_not_silently_included() {
    let ws = tmp_ws();
    write_rule(&ws.0.join("global"), "g.rule");
    write_rule(&ws.0.join("platforms").join("flutter"), "plat.rule");
    write_rule(&ws.0.join("teams").join("mobile"), "team.rule");
    write_rule(&ws.0.join("profiles").join("jhonatan"), "prof.rule");

    let layered = load_layered(&ws.0).unwrap();
    let chain = resolve(&layered, &binding("project:me/app", "org:local"), None).unwrap();
    let ids: Vec<LayerId> = chain.layers(&layered).map(|l| l.id).collect();
    // Only global (no matching org/project instance exists); the undocumented
    // selectors are deferred, so nothing from platform/team/profile leaks in.
    assert_eq!(ids, vec![LayerId::Global]);
}

#[test]
fn declared_platform_is_included_in_composition_order() {
    let ws = tmp_ws();
    write_rule(&ws.0.join("global"), "g.rule");
    write_rule(&ws.0.join("organizations").join("local"), "org.rule");
    write_rule(&ws.0.join("platforms").join("flutter"), "flutter.rule");
    write_rule(&ws.0.join("platforms").join("rust"), "rust.rule");
    write_rule(&ws.0.join("packages").join("common"), "pkg.rule");
    write_rule(&ws.0.join("projects").join("app"), "app.rule");

    let layered = load_layered(&ws.0).unwrap();
    let chain = resolve(
        &layered,
        &binding_with_platforms("project:local/app", "org:local", &["flutter"]),
        None,
    )
    .unwrap();

    let picked: Vec<(LayerId, Option<String>)> = chain
        .layers(&layered)
        .map(|l| (l.id, l.name.clone()))
        .collect();
    assert_eq!(
        picked,
        vec![
            (LayerId::Global, None),
            (LayerId::Organization, Some("local".to_string())),
            (LayerId::Platform, Some("flutter".to_string())),
            (LayerId::Package, Some("common".to_string())),
            (LayerId::Project, Some("app".to_string())),
        ],
        "declared platforms compose between organization and package"
    );
}

#[test]
fn identity_verified_when_registry_maps_repo_to_binding() {
    let ws = tmp_ws();
    write_rule(&ws.0.join("projects").join("con-app"), "p.rule");
    write_registry(
        &ws.0,
        "nui",
        "apiVersion: keel/v1alpha1\nkind: RepositoryRegistry\nmetadata: { id: nui-repos }\n\
         spec:\n  repositories:\n    - provider: github\n      id: NuiMarkets/con-app\n      project: project:nui/con-app\n      locked: true\n",
    );

    let layered = load_layered(&ws.0).unwrap();
    let chain = resolve(
        &layered,
        &binding("project:nui/con-app", "org:nui"),
        Some("NuiMarkets/con-app"),
    )
    .unwrap();
    assert_eq!(chain.identity, IdentityStatus::Verified);
}

#[test]
fn identity_advisory_on_registry_mismatch() {
    let ws = tmp_ws();
    write_registry(
        &ws.0,
        "nui",
        "apiVersion: keel/v1alpha1\nkind: RepositoryRegistry\nmetadata: { id: nui-repos }\n\
         spec:\n  repositories:\n    - provider: github\n      id: NuiMarkets/con-app\n      project: project:nui/con-app\n",
    );

    let layered = load_layered(&ws.0).unwrap();
    // The repo claims a DIFFERENT project than the registry maps it to.
    let chain = resolve(
        &layered,
        &binding("project:nui/other", "org:nui"),
        Some("NuiMarkets/con-app"),
    )
    .unwrap();
    assert!(
        matches!(chain.identity, IdentityStatus::Advisory(_)),
        "a registry disagreement degrades to advisory (section 13.3)"
    );
}

#[test]
fn identity_unregistered_without_registry() {
    let ws = tmp_ws();
    write_rule(&ws.0.join("projects").join("app"), "p.rule");

    let layered = load_layered(&ws.0).unwrap();
    let chain = resolve(
        &layered,
        &binding("project:me/app", "org:local"),
        Some("me/app"),
    )
    .unwrap();
    assert_eq!(chain.identity, IdentityStatus::Unregistered);
}

#[test]
fn malformed_project_reference_is_an_error() {
    let ws = tmp_ws();
    let layered = load_layered(&ws.0).unwrap();
    assert!(matches!(
        resolve(&layered, &binding("not-a-project-ref", "org:local"), None),
        Err(ResolveError::BadProjectRef(_))
    ));
}
