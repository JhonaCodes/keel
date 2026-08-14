# Runtime Architecture

## Ownership and Boundaries

Keel governs the cycle of intention, context, reasoning, planning, delegation,
action, verification, and delivery — by wrapping the EXECUTION ENVIRONMENT of
the model's own CLI (D-012), never its API. The model produces proposals;
Keel decides what context is enabled, what operation can proceed, and what
capability executes.

```text
keel claude/codex
  -> PTY passthrough (keel-host)
  -> child model CLI's command
  -> PATH shim + per-client hook bridge (keel gate)
  -> evaluate_event against compiled snapshot rules
  -> block (exit 2, the command never exists as a process) or allow
  -> OS sandbox (Containment: Seatbelt/Landlock) as an additional hard ring
  -> MCP server (keel-host/src/mcp.rs) delivers skills/agents/rules on demand
  -> evidence ledger
  -> observable transition (derived from accumulated evidence)
```

Ownership is of the Keel process — the parent that wraps the child CLI, not
an API session. A model that can bypass the shim, the hook bridge, AND the OS
sandbox simultaneously is outside governed mode; each ring is designed to
hold even if another is degraded (`--containment shims` opts into the weaker
ring explicitly, never silently).

## Complete Observed Vocabulary

### Resources and Capabilities

- `Skill`: compact/full operational knowledge and examples, with
  `match{terms,context,autoload}` for declarative routing (D-014).
- `Knowledge`: queryable source with provenance, versioned as a hash-chained
  append-only log (`keel knowledge append/verify`).
- `Agent`: logical responsibility executed by a distinct local CLI
  (cross-model), invoked via `keel.agent.invoke` with a validated
  `outputSchema`.
- `Workflow`: phases, transitions, and required artifacts.
- `Tool`: deterministic local or external function.
- `MCPProvider`: normalized external capability; never phase authority.
- `Hook`: internal Keel event trigger.
- `Policy`/`Rule`: decision, detection, preconditions, validation, and enforcement.

### Control and Evidence

- `ModelExecutor`: trait that runs a LOCAL command (`CliModelExecutor` for a
  real CLI, `MockModelExecutor` for tests) — no HTTP boundary to any
  provider.
- `ComponentRegistry`: index of resources declared in the snapshot.
- `ContextResolver` and `CapabilityManager`: selection and grants of context/capabilities.
- `AgentBroker` and `AgentScheduler`: routing, isolation, limits, leases, and cancellation.
- `PhaseController`/guards: transitions with observable conditions.
- `EvidenceLedger`, `Receipt`, `Provenance`, and `Attestation`: traceability and observability boundaries.
- `Snapshot`, `Lock`, `Binding`, `Identity`, and `RepositoryRegistry`: identity and reproducibility of effective config.
- `Composition`, `Profile`, and `Exception`: monotone inheritance and scoped relaxations.
- `Scope`, `Constraint`, `Detector`, `Precondition`, and `Validator`: applicability and evaluation.

## Threat Model

The prompt layer is not a security boundary. Practical security rests on two
independent rings that both evaluate the same compiled rules
(`evaluate_event`): the PATH shim + per-client hook bridge (deterministic,
zero-token interposition — a blocked command never runs), and the OS sandbox
(`kind: Containment`, Seatbelt on macOS shipped per D-012.a) confining the
child CLI's actual process regardless of what it tries. Linux (Landlock) has
partial coverage — see `CONTENCION_MULTIPLATAFORMA.md`. CI running the same
lock and snapshot (`keel ci resolve|run`) is the complementary plane for
reproducible verification.

## Skills and Context

Files live in `skills/` within the Keel workspace, across composition layers
(`global/`, `platforms/<tech>/` — D-015 —, `projects/<name>/`). The snapshot
records identity, version, and paths. The child CLI discovers and fetches
skills/agents/rules through the MCP server (`keel-host/src/mcp.rs`,
`keel.skills.list`/`keel.skills.load`/`keel.rules.query`) and — for
proactive, non-blocking delivery at the right moment (D-016) — through the
per-client hook bridge (`keel gate`, `build_delivery_context`/
`emit_delivery`), which surfaces relevant skills/agents/components on
`SessionStart`, `file.edited`, `command.requested`, and `tool.requested`,
routed by declarative `match` (D-014). `skill.read`/`keel.skills.load`
calculates the hash of delivered content and creates a receipt.
`plan.submit`, `action.request`, `agent.invoke`, `phase.advance`, and
`delivery` block if a required skill is missing.

The runtime can guarantee availability, protocolized reading, and version/hash.
It cannot prove the model's internal comprehension.

## Durable State and Phases

`RuntimeStore` preserves sessions, component receipts, artifact receipts, and
phase-transition receipts in SQLite. Evidence is append-only; the effective
phase is reconstructed from history. Reopening a session with a different
snapshot, an invalid phase sequence, or a guard pointing to an invalid
artifact fails closed. The runtime validates artifact content with JSON
Schema and calculates its canonical hash before allowing a transition — this
storage/evidence architecture is stable and does not depend on API-vs-PTY
transport.

The concrete phase taxonomy is NOT the one this module currently implements
(`Investigation/Planning/Implementation/Verification/Audit/Resolution/
Acceptance/Delivery`) — that sequence predates the parent-runtime pivot,
has no RED/GREEN concept, and `PhaseController`/`RuntimeHost` are not
imported by `keel-host`/`keel-cli` in production today (verifiable with
`grep -rln "RuntimeHost" --include="*.rs" .`). The real, currently-planned
phase design is authored fresh with Keel's actual workflow vocabulary
(analysis/red/green/refactor/audit/verify/done), derived as events from
accumulated ledger evidence rather than a separately-tracked state file —
see `planificacion/ordenes_trabajo/PLAN_MAESTRO.md#H-010`.

## Integrations

Keel never calls a model provider's HTTP API. The normative integration is:

```text
keel claude/codex -> PTY -> PATH shim + broker (evaluate_event) -> Containment sandbox (Seatbelt/Landlock) -> MCP server (skills/agents/rules)
```

MCP is the real convergence channel through which the child CLI requests
skills/agents/rules FROM Keel (D-012.b) — declared, validated, and
registered by Keel, with permissions, policy, provenance, and evidence.

## Process and Product Entry

Each governed session is an ephemeral process wrapping the child CLI:

```text
keel claude/codex/launch --client <name> -- <cmd>
  -> resolves binding + lock + snapshot
  -> opens/restores RuntimeStore
  -> wires convergence (MCP config + hook bridge) into the child's own config
  -> spawns the child CLI under PTY passthrough, shim, and sandbox
  -> executes until delivery, cancellation, or error
  -> closes and persists evidence
```

The process is sovereign because the model only receives mediated operations
and capabilities, not because it persists as a daemon. Persistence allows
resumption; a later daemon would only optimize latency and concurrent
sessions.

The CLI is the only user entry point. `init` creates and compiles the
workspace; `bind` associates a repo to a project (binding + lock only, never
definitions); `compile`/`test`/`lock` validate and fix resolution;
`doctor --governed` validates snapshot, lock, config, and store read-only;
`claude`/`codex`/`launch --client generic` start a governed session wrapping
the child CLI; `gate` is the per-client hook bridge; `mcp` runs the
convergence server; `ci resolve|run` runs the same plane in CI. No step
writes provider config or instruction files outside the ephemeral
per-session wiring Keel itself manages.
