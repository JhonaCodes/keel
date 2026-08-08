> **CORRECTION (D-012, 2026-08-07).** This document describes, in parts, the
> design of "session owned by Keel via provider API" (RuntimeHost ->
> ModelExecutor -> API). That direction was REVERTED: **Keel is a PARENT
> runtime that governs the EXECUTION ENVIRONMENT of the model's CLI and does NOT
> use provider APIs.** Where this text speaks of calling the model's API, of
> `keel run`, or of `keel configure executor`, it is OBSOLETE — see
> `DECISIONES.md` (D-012 a-d) and the real flow in `USO_INSTALACION.md`. The
> full rewrite of this document to parent-runtime architecture is pending work
> (not an oversight).

# Runtime Contracts

## Operations

```text
session.start
component.list
skill.read
knowledge.read
blueprint.read
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
  "reason": "blueprint requirement"
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
  "reason": "blueprint requirement",
  "read_at": "2026-08-07T00:00:00Z"
}
```

The receipt is registered append-only in SQLite. When reopening the session,
Keel restores consumed components and rejects the session if the snapshot doesn't
match. The requested phase must match the runtime's real phase; the model cannot
fake it. A textual promise doesn't count and produces `REQUIRED_COMPONENT_READ`
when attempting to advance.

## Phases and Artifacts

```text
investigation -> planning -> implementation -> verification
              -> audit -> resolution -> acceptance -> delivery
```

Each transition requires a valid artifact: Investigation Report, Solution
Contract, Implementation Record, Evidence Report, Audit Report, Resolution
Record, and Acceptance Record. Each type can only be registered in its owning
phase, so a future phase cannot be preloaded. Keel validates content via JSON
Schema, calculates canonical hash, and registers both the artifact receipt and
transition receipt before changing state in memory.

Transitions cannot skip phases. On restore, Keel verifies the order and that each
transition points to the valid artifact that enabled it; manipulated history fails
closed.

Current state: schemas are delivered to the runtime validation API. Resolving them
exclusively from contracts compiled in the snapshot remains pending.

## ModelExecutor

The executor receives normalized `ModelRequest` and returns normalized
`ModelResponse`. It exposes provider/model, completion, and cancellation. It
does not decide skills, policies, phases, capabilities, shell, filesystem, MCP,
or agents.

The request's `session_id` must match the `RuntimeHost`; a cross-request is
rejected before reaching the executor.

Implemented: `MockModelExecutor`, Anthropic Messages, and OpenAI Responses. HTTP
drivers translate messages, tools, text, and tool calls to the normalized
contract. Interactive CLIs are not canonical runtime nor remain as an alternative
mode. Smoke tests with real providers require operator credentials.

## AgentScheduler

Target contract: each task has id, session, project, parent, depth, agent id,
executor id, budget, state, and lease. States: `pending`, `claimed`, `running`,
`completed`, `failed`, `cancelled`. The scheduler must limit concurrency and
prevent duplicate claims via SQLite transaction and recoverable lease.

Current state: the SQLite queue can be durable or in-memory, applies a global
limit, makes transactional claim, renews leases, and recovers tasks whose lease
expires. It doesn't yet model per-project/session limits, depth, fan-out, graph,
budgets, priorities, or cascading cancellation.

The child agent receives only declared context, skills, capabilities, credentials,
and budget. Implicit inheritance is forbidden.

## Governed CLI

```text
keel init <workspace> --executor mock [--json]
keel configure executor add|list|test|remove|default
keel doctor --workspace <workspace> --governed [--json]
keel run --workspace <workspace> --task <text> [--executor <id>] [--json]
keel run --workspace <workspace> --resume <session-id> [--json]
```

`init` leaves valid snapshot, lock, mock config, and store. `run` only accepts
executors resolved by Keel config, requires lock and snapshot to match, creates
session identity, and emits state, phase, snapshot, and executor. Task and
executor are persisted. `resume` continues from durable phase, rejects snapshot
drift, and does not allow replacing the fixed executor.

Current exit codes: `0` completed and `1` error, denial, or unfinished session.
Differentiated codes for denial/approval remain pending.

## Secrets

An executor contains `secret-ref`, never the value. Locally the reference points
to Keychain/Secret Service; in CI it points to a declared environment variable.
Resolving a secret is an internal operation not visible to the model, and its
value is excluded from serialization, logs, errors, snapshot, lock, and ledger.
