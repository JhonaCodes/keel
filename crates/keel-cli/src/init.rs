// SPDX-License-Identifier: Apache-2.0
//! Scaffold content for `keel init` — the full spec section 8.5 workspace tree.
//!
//! Each directory ships a README (what it is + how to author it, as context for
//! a human OR an LLM) and a base `.example` template (the loader ignores
//! `.example`, so nothing fires until it is renamed + edited). The text is
//! HONEST about status: ACTIVE now, PARSED-not-yet-composed, org-scale/deferred,
//! or engine-generated — nothing is presented as working when it is not.

pub const WORKSPACE_YAML: &str = "\
apiVersion: keel/v1alpha1
kind: Workspace
metadata: { id: my-workspace }
spec:
  description: Keel workspace — layered composition (section 7 / section 8.5)
";

pub const WORKSPACE_README: &str = r#"# Keel workspace

This is a **Keel workspace**: the versioned source of your rules and their
composition (spec section 8.4). A repository binds to a project here
(`.keel/project.yaml`) and `keel compile` composes the layers that apply to it
into one immutable snapshot.

## Layers (composition order, section 7.2 — highest authority first)

```
global/          rules that apply to EVERY project            [ACTIVE]
organizations/   per-organization config + policies           [repositories.yaml ACTIVE; components org-scale/deferred]
platforms/       per-technology defaults (e.g. flutter/)       [scaffold — no selector yet]
projects/        project-specific rules                        [ACTIVE]
teams/           authorized team variants                      [scaffold — no selector yet]
profiles/        personal preferences (cannot weaken locked)   [PARSED — not composed yet]
```

Plus support directories: `packages/` `schemas/`
`registry/` `locks/` `migrations/` `tests/` (see each README).

## The rule of composition (section 7.4)

A rule marked `locked` in a higher layer **cannot be weakened** by a lower one.
A lower layer may only STRENGTHEN it (stricter decision, wider coverage, extra
checks) or REPLACE it where the higher layer says `overridable`. `keel compile`
verifies this dimension by dimension and fails with the offending layer. The one
bounded, audited way to relax a `locked` rule is an `Exception` (see
`global/exceptions/`).

## Get going

```
keel doctor --workspace . --governed
keel claude              # run a governed client CLI (or: keel launch --client generic -- <cmd>)
```

The LLM (or you) authors the actual content — each folder's README + template is
the context for what belongs there. See `docs/AUTORIA.md` for how to author each
kind. Keel ships NO default rules on purpose: it does not know your project's
constraints.
"#;

pub const GLOBAL_README: &str = r#"# global/ — rules for EVERY project  [ACTIVE]

The base layer (section 8.5). Anything here applies to all projects that resolve
against this workspace. Put your non-negotiable, organization-wide rules here and
mark them `locked` so no project can weaken them (section 7.4).

- `rules/`       — the global rules (see its README + template)
- `exceptions/`  — governed waivers that relax a locked rule in a bounded area
"#;

pub const GLOBAL_RULES_README: &str = r#"# global/rules/  [ACTIVE]

One `kind: Rule` per `.yaml` file. These apply to every project. Mark the
non-negotiable ones `locked: true` — a project may then only strengthen them.

`author`, `adrRef` and `reviewAfter` are MANDATORY (ADR-023): every rule has an
owner, an originating decision and a review window (so `keel prune` can later
propose deleting it with data). See `rule.yaml.example`.
"#;

pub const GLOBAL_RULE: &str = r#"# GLOBAL rule template — applies to EVERY project. Rename to <name>.yaml.
#
# `locked: true` => a lower layer may only STRENGTHEN it; weakening is a compile
# error (section 7.4). Relax it only via a bounded Exception (../exceptions/).
#
# apiVersion: keel/v1alpha1
# kind: Rule
# metadata:
#   id: global.no-raw-queries        # unique across the workspace
#   author: your-name
#   adrRef: adr:ADR-001              # the decision that originates it
#   reviewAfter: P6M                 # ISO-8601 review window
# spec:
#   locked: true
#   reversibility: reversible        # irreversible => `unknown` escalates to a human (4.7)
#   scope: { paths: { include: ["src/**"] } }
#   on: [file.edited]                # events: file.edited, command.requested, ...
#   detect: { using: builtin:text.regex, with: { pattern: "rawQuery" } }
#   enforcement:
#     invalid: { decision: block, report: { message: "use the query builder" } }
#     valid:   { decision: allow }
"#;

pub const EXCEPTIONS_README: &str = r#"# global/exceptions/  [ACTIVE]

The ONLY governed way to relax a `locked` rule (section 7.4). An Exception is
owned by the scope that declared the lock, names a bounded `scope.paths.include`
and an `expiry`. The lock is LIFTED only within that scope and stands at full
strength everywhere else; the relaxation is recorded as a human decision. An
expired or unbounded exception does nothing. See `exception.yaml.example`.
"#;

pub const EXCEPTION_TMPL: &str = r#"# Exception template — rename to <name>.yaml to activate it.
#
# apiVersion: keel/v1alpha1
# kind: Exception
# metadata: { id: reports-waiver }
# spec:
#   rule: rule:global.no-raw-queries   # the locked rule being relaxed
#   owner: global                      # MUST match the layer that locked it
#   reason: "Legacy reporting migrates next quarter."
#   scope: { paths: { include: ["src/reports/**"] } }   # the lock is lifted ONLY here
#   expiry: "2027-01-01"               # ISO date; an expired exception is dead
"#;

pub const CONTAINMENT_README: &str = r#"# global/containment/  [ACTIVE — macOS; Linux degrades to shims]

The HARD RING: the OS-sandbox backstop (section 5.2 runner). A `kind: Containment`
declares ONLY what the kernel can enforce regardless of PATH — `denyUnlink`
(globs of files that may not be deleted), `denyWriteOutside`, `denyNetwork`.
Command interposition (shims) governs the PATH surface; an absolute-path call
like `/bin/rm` steps around it. Containment is what the child cannot step
around: the kernel refuses the action.

It composes by UNION across layers (restrictions only add) and enters the
snapshot hash, so `keel lock --verify` detects any drift. On a platform with
no sandbox provider (Linux today) or with `keel <cli> --containment shims`,
the level degrades to shims WITH A BANNER — never silently. See
`containment.yaml.example`.
"#;

pub const CONTAINMENT_TMPL: &str = r#"# Containment template — rename to <name>.yaml to activate it.
#
# apiVersion: keel/v1alpha1
# kind: Containment
# metadata: { id: global.hard.protect-docs }
# spec:
#   denyUnlink: ["**/*.md"]     # these files cannot be deleted, even via /bin/rm
#   denyWriteOutside: true      # writes confined to the workspace subtree
#   denyNetwork: false          # deny outbound network for the child
"#;

pub const ORGS_README: &str = r#"# organizations/  [repositories.yaml ACTIVE; components org-scale/deferred]

Per-organization configuration. Each subdirectory is one organization
(e.g. `my-company/`). `repositories.yaml` is USED today: it maps a repository
identity to a project so `keel compile` can verify the binding (section 7.1).
`organization.yaml` and `composition.yaml` are informational for now (the
composition ORDER is the fixed chain of section 7.2). The `components/` layout
(policies/contracts/workflows/permissions) is org-scale and not loaded yet.
"#;

pub const ORG_INSTANCE_README: &str = r#"# organizations/my-company/  (rename to your organization)

- `repositories.yaml`  [ACTIVE] — maps `provider/id` repos to Keel projects; the
  compliance check verifies a repo resolves to the project it claims (section 7.1).
- `organization.yaml`  [informational] — org identity/metadata.
- `composition.yaml`   [informational] — the composition order is fixed (7.2).
- `components/`        [org-scale, deferred] — see its README.
"#;

pub const ORGANIZATION_TMPL: &str = r#"# organization.yaml template — rename to organization.yaml.
#
# apiVersion: keel/v1alpha1
# kind: Organization
# metadata: { id: my-company }
# spec:
#   displayName: "My Company"
"#;

pub const REPOSITORIES_TMPL: &str = r#"# repositories.yaml template — rename to repositories.yaml to activate it.
# Maps a repository's identity to the Keel project it resolves to (section 7.1).
#
# apiVersion: keel/v1alpha1
# kind: RepositoryRegistry
# metadata: { id: my-company-repositories }
# spec:
#   repositories:
#     - provider: github
#       id: my-company/my-app          # the repo's own identity
#       project: project:my-company/my-app
#       locked: true                   # this mapping cannot be reassigned lower down
"#;

pub const COMPOSITION_TMPL: &str = r#"# composition.yaml — informational in this version.
# The composition ORDER is the fixed chain of section 7.2
# (global -> organization -> platform -> project -> team -> profile -> session);
# it is not configured per-workspace. This file documents your org's intent.
#
# apiVersion: keel/v1alpha1
# kind: Composition
# metadata: { id: my-company }
# spec:
#   notes: "Order is fixed by section 7.2; layers are selected by repo identity (7.1)."
"#;

pub const COMPONENTS_README: &str = r#"# components/ — org-scale, NOT loaded yet  [DEFERRED]

The spec section 8.5 places organization-scale components here
(`policies/`, `contracts/`, `workflows/`, `permissions/`). Those kinds are part
of the enterprise/distribution story (section 8.3: they add distribution, not new
composition semantics) and are NOT compiled yet. This directory is a documented
placeholder — authoring real component YAML here is rejected loudly by
`keel compile` rather than silently ignored. For now, express governance as
rules in `global/rules/` and `projects/<name>/rules/`.
"#;

pub const PLATFORMS_README: &str = r#"# platforms/  [scaffold — no selector yet]

Per-technology defaults (e.g. `flutter/`, `rust/`), section 8.5. A platform layer
would carry rules shared by all projects of a technology. There is no standalone
selector for it yet (the binding carries only project + workspace), so platform
layers are not composed in this version. Put shared rules in `global/` for now.
"#;

pub const PROJECTS_README: &str = r#"# projects/  [ACTIVE]

One subdirectory per project (e.g. `app/`). A project layer holds the rules,
tools and tests specific to that project; it composes ON TOP of `global/` (and
its organization) and may only strengthen a locked global rule, or replace one
marked `overridable`. A repository binds to a project via `.keel/project.yaml`
(`project:<org>/<name>`); the `<name>` is the subdirectory here.
"#;

pub const PROJECT_INSTANCE_README: &str = r#"# projects/app/  (this workspace is bound here by default)

The default binding (`.keel/project.yaml`) is `project:local/app`, so
`keel compile` composes `global/` + this project. Rename/copy for real projects
and update the binding with `keel bind --project project:<org>/<name>`.

- `rules/`  — this project's rules (may strengthen global ones)
- `tools/`  — external analyzers this project's rules call
- `tests/`  — RuleTests gating publication
"#;

pub const PROJECT_RULES_README: &str = r#"# projects/app/rules/  [ACTIVE]

One `kind: Rule` per `.yaml`. Applies to THIS project only. To strengthen a
global rule, declare a rule with the SAME `id` and a stricter shape (higher
decision, wider coverage, extra checks) — the compiler verifies it is at least as
restrictive (section 7.4). See `rule.yaml.example`.
"#;

pub const PROJECT_RULE: &str = r#"# PROJECT rule template — applies to THIS project only. Rename to <name>.yaml.
#
# apiVersion: keel/v1alpha1
# kind: Rule
# metadata:
#   id: app.no-todo
#   author: your-name
#   adrRef: adr:ADR-002
#   reviewAfter: P6M
# spec:
#   on: [file.edited]
#   detect: { using: builtin:text.contains, with: { text: "TODO" } }
#   enforcement:
#     invalid: { decision: review }
#     valid:   { decision: allow }
"#;

pub const TOOLS_README: &str = r#"# tools/  [ACTIVE]

External tools a rule's `validate`/`detect` calls. A tool is CODE (section 4.4):
it receives the event as JSON on stdin and answers `valid|invalid|unknown`
(or SARIF, or an exit code). Declare one `kind: Tool` per `.yaml`; reference it
from a rule as `tool:<id>`. See `tool.yaml.example`.
"#;

pub const TOOL_TMPL: &str = r#"# Tool template — rename to <name>.yaml to activate it.
#
# apiVersion: keel/v1alpha1
# kind: Tool
# metadata: { id: my-analyzer, version: 0.1.0 }
# spec:
#   command: [python3, bin/my_analyzer.py]   # relative to the workspace root
#   timeoutMs: 5000
#   output: verdict-json   # also: sarif | exit-code (0=valid,1=invalid,other=unknown)
"#;

pub const TESTS_README: &str = r#"# tests/  [ACTIVE]

RuleTests for this project. `keel compile` does NOT publish a snapshot if any
RuleTest fails (section 10.2) — your safety net for editing rules. One
`kind: RuleTest` per `.yaml`. See `test.yaml.example`.
"#;

pub const TEST_TMPL: &str = r#"# RuleTest template — rename to <name>.yaml to activate it.
#
# apiVersion: keel/v1alpha1
# kind: RuleTest
# metadata: { id: app.no-todo.basic }
# spec:
#   target: rule:app.no-todo
#   event: { kind: file.edited, file: src/x.dart, content: "// TODO later" }
#   expect: { verdict: invalid, decision: review, origin: deterministic }
"#;

pub const TEAMS_README: &str = r#"# teams/  [scaffold — no selector yet]

Authorized team variants (section 8.5): rules for a specific team, below the
project layer, that may only strengthen. There is no standalone selector for team
membership yet, so team layers are not composed in this version.
"#;

pub const PROFILES_README: &str = r#"# profiles/  [PARSED — not composed yet]

Personal preferences (section 8.5), the LOWEST authority layer — it can never
weaken a `locked` rule. A `kind: Profile` parses today (implementation strategy,
verbosity, an alternative agent binding where allowed), but profiles are not yet
selected into the composition. See `profile.yaml.example`.
"#;

pub const PROFILE_TMPL: &str = r#"# Profile template — rename to <your-name>/profile.yaml (profiles are namespaced).
#
# apiVersion: keel/v1alpha1
# kind: Profile
# metadata: { id: your-name }
# spec:
#   client: codex
#   preferences: { implementationStrategy: tdd, verbosity: compact }
"#;

pub const PACKAGES_README: &str = r#"# packages/  [DEFERRED — invariant 3]

Versioned, reusable components shared across workspaces (section 8.5). Packaging
is not implemented yet; components live inline in their layers for now.
"#;

pub const GOVERNED_RESOURCE_README: &str = r#"# Keel-owned resources

Resources in this directory are parsed, validated, compiled into the immutable
snapshot and delivered only through runtime operations. They are never copied
to provider configuration.
"#;

pub const DEFAULT_WORKFLOW: &str = r#"apiVersion: keel/v1alpha1
kind: Workflow
metadata: { id: default, version: 1.0.0 }
spec:
  config:
    phases: [investigation, planning, implementation, verification, audit, resolution, acceptance, delivery]
"#;

pub const MOCK_EXECUTOR: &str = r#"apiVersion: keel/v1alpha1
kind: ModelExecutor
metadata: { id: mock, version: 1.0.0 }
spec:
  # A governed executor is a LOCAL CLI (D-012): keel runs `command`, writes the
  # prompt to its stdin and treats stdout as the response. Keel never speaks a
  # provider API. `cat` is a deterministic echo executor for wiring/tests;
  # replace with e.g. `[codex, exec, --json]` or `[claude, -p]`.
  config:
    command: [cat]
"#;

pub const SCHEMAS_README: &str = r#"# schemas/  [scaffold]

Schemas for artifacts, requests, results and findings (section 8.5). Keel embeds
its core JSON Schemas in the binary today; workspace-level custom schemas are a
future refinement.
"#;

pub const REGISTRY_README: &str = r#"# registry/  [engine-generated]

The resolved component index (section 8.5). Keel derives resolution from the
layers at compile time; it does not persist a separate registry here in this
version. Leave empty — it is not authored by hand.
"#;

pub const LOCKS_README: &str = r#"# locks/  [engine-generated]

Resolution locks (section 8.5). The pinned resolution for a bound repository is
written to that repo's `.keel/keel.lock` by `keel lock`; this workspace-level
`locks/` directory is not populated in this version. Not authored by hand.
"#;

pub const MIGRATIONS_README: &str = r#"# migrations/  [scaffold]

Schema-version migrations (section 8.5) for when the DSL evolves. None are needed
at the current version; this is a placeholder for forward compatibility.
"#;

pub const WS_TESTS_README: &str = r#"# tests/ (workspace-level)  [scaffold]

Cross-cutting tests for rules, tools and COMPOSITION (section 8.5) — e.g. tests
that assert a locked rule cannot be weakened. Per-project RuleTests live under
`projects/<name>/tests/` and already gate `keel compile`. Workspace-level
composition tests are a future refinement; author per-project tests for now.
"#;
