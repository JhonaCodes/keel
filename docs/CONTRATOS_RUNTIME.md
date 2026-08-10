# Runtime Contracts

## Operations

```text
session.start
component.list
skill.read
knowledge.read
plan.submit
action.request
agent.invoke
phase.advance
delivery
session.close
```

Authorization of an operation is a function of session state, snapshot, phase,
receipts, grants, policy, and artifacts. The model's text does not change that
state.

## `skill.read`

Minimum request:

```json
{
  "operation": "skill.read",
  "skill_id": "flutter.state-management",
  "variant": "compact",
  "session_id": "session-123",
  "phase": "planning",
  "reason": "reference requirement"
}
```

Minimum response:

```json
{
  "skill_id": "flutter.state-management",
  "version": "1.2.0",
  "content_hash": "sha256:...",
  "content": "...",
  "receipt_id": "receipt-01...",
  "required": true,
  "session_id": "session-123",
  "phase": "planning",
  "reason": "reference requirement",
  "read_at": "2026-08-07T00:00:00Z"
}
```

The receipt is registered append-only in SQLite. When reopening the session,
Keel restores consumed components and rejects the session if the snapshot doesn't
match. The requested phase must match the runtime's real phase; the model cannot
fake it. A textual promise doesn't count and produces `REQUIRED_COMPONENT_READ`
when attempting to advance.

## Phases and Artifacts

The concrete sequence implemented by `PhaseController`/`RuntimeStore` today
(`investigation -> planning -> implementation -> verification -> audit ->
resolution -> acceptance -> delivery`) predates the parent-runtime pivot and
is NOT wired into production (`keel-host`/`keel-cli` don't import
`RuntimeHost`). It is being replaced by a phase design authored with jflow's
real vocabulary (analysis/red/green/refactor/audit/verify/done), derived as
events from ledger evidence rather than tracked as separate state — see
`planificacion/ordenes_trabajo/PLAN_MAESTRO.md#H-010`. The artifact-per-phase
CONTRACT SHAPE described below is durable and architecture-agnostic; only the
specific phase names above are superseded.

Each transition requires a valid artifact (one type per phase, e.g.
Investigation Report, Solution Contract, Implementation Record, Evidence
Report, Audit Report, Resolution Record, Acceptance Record — names will
follow whatever phase set H-010 lands on). Each type can only be registered
in its owning phase, so a future phase cannot be preloaded. Keel validates
content via JSON Schema, calculates canonical hash, and registers both the
artifact receipt and transition receipt before changing state in memory.

Transitions cannot skip phases. On restore, Keel verifies the order and that each
transition points to the valid artifact that enabled it; manipulated history fails
closed.

Current state: schemas are delivered to the runtime validation API. Resolving them
exclusively from contracts compiled in the snapshot remains pending — see
`planificacion/ordenes_trabajo/PLAN_MAESTRO.md#H-002`.

## ModelExecutor

The executor runs a LOCAL command and returns its output — no HTTP request to
any model provider. It exposes the local command spec (`config.command`,
`config.env`) and cancellation. It does not decide skills, policies, phases,
capabilities, shell, filesystem, MCP, or agents.

The request's `session_id` must match the running session; a cross-request is
rejected before reaching the executor.

Implemented: `MockModelExecutor` (tests) and `CliModelExecutor` — the only
non-mock executor, confined to `cwd=root`, `env_clear()` + declared
`config.env`, running the local CLI command (`crates/keel-runtime/src/
executor.rs`). This is how the interactive model CLI itself (`claude -p`,
`codex exec`) is invoked when Keel needs to run one as a governed AGENT
(distinct from the interactive session the operator drives via `keel
claude`/`keel codex`, which wraps the CLI via PTY, not via `ModelExecutor`).

## AgentScheduler / agent.invoke

`keel.agent.invoke` (served by the MCP server, `crates/keel-host/src/
mcp.rs`) resolves `Agent -> ModelExecutor (CliModelExecutor) -> lease via
AgentScheduler -> validates result against outputSchema -> returns to
caller`. This is how agents are actually invoked today — cross-model (a
Claude parent session can invoke a Codex-executed agent, deterministically,
without any provider API).

Scheduler contract: each task has id, session, project, parent, depth, agent
id, executor id, budget, state, and lease. States: `pending`, `claimed`,
`running`, `completed`, `failed`, `cancelled`. The scheduler limits
concurrency and prevents duplicate claims via SQLite transaction and
recoverable lease.

Current state: the SQLite queue can be durable or in-memory, applies a global
limit, makes transactional claim, renews leases, and recovers tasks whose lease
expires. It doesn't yet model per-project/session limits, depth, fan-out, graph,
budgets, priorities, or cascading cancellation — see
`planificacion/ordenes_trabajo/PLAN_MAESTRO.md#H-003`. Agent subprocesses are
confined by `cwd`/`env` today, not yet wrapped in the full shim+sandbox ring
— see `#H-015`.

The child agent receives only declared context, skills, capabilities, credentials,
and budget. Implicit inheritance is forbidden.

## Governed CLI

```text
keel init <workspace> [--json]
keel bind <workspace> [--json]
keel compile <workspace> [--json]
keel test <workspace> [--json]
keel lock <workspace> [--verify] [--json]
keel doctor --workspace <workspace> --governed [--json]
keel claude|codex -- [args...]
keel launch --client generic -- <cmd>
keel gate --client <claude-code|codex> < hook-payload.json
keel mcp --workspace <workspace> --session <id>
keel knowledge append|verify
keel ci resolve|run
keel use <workspace>
```

`init` leaves valid snapshot, lock, and store, with no default rules (Keel
ships nothing by design). `bind` associates a repo to a project — the repo
only ever stores binding + lock, never definitions. `compile`/`test`/`lock`
validate the workspace and fix resolution, shared between local and CI.
`claude`/`codex`/`launch` start a governed session: the client CLI runs as
the child, wrapped in PTY passthrough, with the shim, hook bridge, and
sandbox wired in before the child process starts. `gate` and `mcp` are the
two channels the child CLI uses to talk to Keel (hook bridge and MCP
server, respectively) — not invoked directly by the operator.

Current exit codes: `0` completed and `1` error/denial. A blocked action
inside a governed session exits `2` (see `keel gate`'s doc comment in
`crates/keel-cli/src/gate.rs`). Differentiated codes for every denial/approval
case remain pending.

## Secrets

An executor contains `secret-ref`, never the value. Locally the reference points
to Keychain/Secret Service; in CI it points to a declared environment variable.
Resolving a secret is an internal operation not visible to the model, and its
value is excluded from serialization, logs, errors, snapshot, lock, and ledger.
