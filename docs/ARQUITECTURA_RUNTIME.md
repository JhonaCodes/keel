> **CORRECTION (D-012, 2026-08-07).** This document describes, in parts, the
> design of "session owned by Keel via provider API" (RuntimeHost ->
> ModelExecutor -> API). That direction was REVERTED: **Keel is a PARENT
> runtime that governs the EXECUTION ENVIRONMENT of the model's CLI and does NOT
> use provider APIs.** Where this text speaks of calling the model's API, of
> `keel run`, or of `keel configure executor`, it is OBSOLETE — see
> `DECISIONES.md` (D-012 a-d) and the real flow in `USO_INSTALACION.md`. The
> full rewrite of this document to parent-runtime architecture is pending work
> (not an oversight).

# Runtime Architecture

## Ownership and Boundaries

Keel governs the cycle of intention, context, reasoning, planning, delegation,
action, verification, and delivery. The model produces proposals; Keel decides
what context is enabled, what operation can proceed, and what capability executes.

```text
structured input
  -> session + phase + immutable snapshot
  -> component and context resolver
  -> ModelExecutor
  -> normalized response (text/tool call/agent request/artifact)
  -> verification + policy + grants
  -> capability or AgentBroker
  -> evidence ledger
  -> observable transition
```

Ownership is of the Keel process. A model that can edit the client, invoke shell
directly, or change the snapshot is outside governed mode.

## Complete Observed Vocabulary

### Resources and Capabilities

- `Skill`: compact/full operational knowledge and examples.
- `Knowledge`: queryable source with provenance.
- `Blueprint`: work template, inputs, outputs, phases, and requirements.
- `Agent`: logical responsibility; not a process or provider.
- `Workflow`: phases, transitions, and required artifacts.
- `Tool`: deterministic local or external function.
- `MCPProvider`: normalized external capability; never phase authority.
- `Hook`: internal Keel event trigger.
- `Policy`/`Rule`: decision, detection, preconditions, validation, and enforcement.

### Control and Evidence

- `ModelExecutor`: boundary with Claude, Codex, or other model.
- `ComponentRegistry`: index of resources declared in the snapshot.
- `ContextResolver` and `CapabilityManager`: selection and grants of context/capabilities.
- `AgentBroker` and `AgentScheduler`: routing, isolation, limits, leases, and cancellation.
- `PhaseController`/guards: transitions with observable conditions.
- `EvidenceLedger`, `Receipt`, `Provenance`, and `Attestation`: traceability and observability boundaries.
- `Snapshot`, `Lock`, `Binding`, `Identity`, and `RepositoryRegistry`: identity and reproducibility of effective config.
- `Composition`, `Profile`, and `Exception`: monotone inheritance and scoped relaxations.
- `Scope`, `Constraint`, `Detector`, `Precondition`, and `Validator`: applicability and evaluation.

## Threat Model

The prompt layer is not a security boundary. Practical security depends on the
model having no direct access to filesystem, shell, Git, MCP, or provider config,
and every capability passing through Keel. The local plane does not protect
against a user with privileges over the host process; that limit is declared as
advisory. Strong sandboxing and CI are additional planes.

## Skills and Context

Files live in `skills/` within the Keel workspace. The snapshot records identity,
version, and paths; `RuntimeHost::from_snapshot` hydrates from that root.
`skill.read` calculates the hash of delivered content and creates a receipt.
`plan.submit`, `action.request`, `agent.invoke`, `phase.advance`, and `delivery`
block if a required skill is missing.

The runtime can guarantee availability, protocolized reading, and version/hash.
It cannot prove the model's internal comprehension.

## Durable State and Phases

`RuntimeStore` preserves sessions, component receipts, artifact receipts, and
phase-transition receipts in SQLite. Evidence is append-only; the effective phase
is reconstructed from history. Reopening a session with a different snapshot, an
invalid phase sequence, or a guard pointing to an invalid artifact fails closed.

The base sequence is Investigation, Planning, Implementation, Verification,
Audit, Resolution, Acceptance, and Delivery. The runtime validates artifact
content with JSON Schema and calculates its canonical hash before allowing the
transition. The next architectural step is to compile workflows, contracts, and
schemas into the snapshot so the API doesn't receive the schema from the caller.

## Integrations

The normative integration is `Keel Runtime -> ModelExecutor -> provider`.
Interactive CLIs, hooks, and provider settings are not part of the final
delivery. MCP only connects a declared capability, with permissions, policy,
provenance, and evidence; it does not carry Keel control.

## Process and Product Entry

The first implementation uses an ephemeral process per session:

```text
keel run
  -> resolves binding + lock + snapshot
  -> opens/restores RuntimeStore
  -> builds RuntimeHost and ModelExecutor
  -> executes loop until delivery, cancellation, or error
  -> closes executor and persists evidence
```

The process is sovereign because the model only receives mediated operations and
capabilities, not because it persists as a daemon. Persistence allows resumption;
a later daemon would only optimize latency and concurrent sessions.

The CLI is the only user entry point. `init` creates and compiles; `configure`
manages executors and secrets; `doctor --governed` validates snapshot, lock,
config, and store; `run` starts or resumes a session. Task and executor are
fixed in SQLite so resumption doesn't change identity. No step writes provider
config or instruction files.
