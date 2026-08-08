> **CORRECTION (D-012, 2026-08-07).** This document describes, in parts, the
> design of "session owned by Keel via provider API" (RuntimeHost ->
> ModelExecutor -> API). That direction was REVERTED: **Keel is a PARENT
> runtime that governs the EXECUTION ENVIRONMENT of the model's CLI and does NOT
> use provider APIs.** Where this text speaks of calling the model's API, of
> `keel run`, or of `keel configure executor`, it is OBSOLETE — see
> `DECISIONES.md` (D-012 a-d) and the real flow in `USO_INSTALACION.md`. The
> full rewrite of this document to parent-runtime architecture is pending work
> (not an oversight).

# Master Plan — Closing the Governed Runtime

## Objective

Keel owns the session and cognitive cycle. Providers are `ModelExecutor`;
resources, operations, capabilities, agents, phases, and evidence are resolved
within the runtime.

## Implemented

### M0 - CLI Vertical

- Black-box test `init -> configure -> doctor -> run -> resume`.
- `init` publishes snapshot and lock, creates mock config and SQLite store.
- `run` starts or continues a governed session without manual config; task,
  executor, and snapshot are fixed for fail-closed resumption.

### M1 - Vocabulary and Snapshot

- Compiled kinds: Rule, Tool, Skill, Agent, Blueprint, Knowledge, Workflow,
  Contract, Hook, MCPProvider, ModelExecutor, AgentRoutingPolicy, and Policy.
- Open registry, requirements per phase, content, provenance, and hash within
  snapshot/lock.
- Missing references fail during compilation.

### M2 - Context and Loop

- Generalized skill and component reading with persistent receipts.
- Pending requirements block plan, action, agent, transition, and delivery.
- Each phase calls the executor in a bounded loop, dispatches Keel operations,
  and reinjects results before accepting the artifact.

### M3 - Capabilities

- `CapabilityManager` explicitly grants filesystem, shell, or Git.
- Paths remain confined to workspace.
- Rules evaluate in enforce mode before side effect.
- An absent or denied capability fails closed.

### M4 - Agents

- `AgentBroker` resolves Agent -> ModelExecutor and validates output contract.
- SQLite scheduler with states, concurrency limit, claims, and leases.
- `agent.invoke` is connected to the loop, resolves child executor local config,
  and returns `AgentResult` to parent.
- Cross-provider logical test parent Claude -> agent Codex.

### M5 - Executors and Configuration

- HTTP drivers for Anthropic Messages and OpenAI Responses.
- `configure executor add|list|test|remove|default`.
- Secrets by environment reference or Keychain/Secret Service.
- Deterministic mock mandatory for CI and demonstration.

### M6 - Replacement

- Removed all client-dependent integration, command-based executors, context
  packages, tests, and associated datasets.
- Scaffold without provider folders, with governed resources and mock executor.
- Functional installer from checkout for macOS/Linux.

## Remaining Work for Stable Release

1. **Compiled Workflow:** replace fixed machine with phases/transitions and
   contracts resolved entirely from snapshot.
2. **Complete Scheduler:** maxDepth, fan-out, cycles, tokens, cost, priorities,
   crash recovery, and cascading cancellation.
3. **Complete AgentBroker:** isolate child components/capabilities, apply
   timeout/cancellation, and register real budgets/usage.
4. **MCPProvider:** implement transports, discovery, tool normalization, secret
   refs, pre/post policy, and provenance.
5. **Internal Hooks:** event dispatcher with declared actions, no power to modify
   snapshot or skip policy.
6. **Unified Ledger:** record hashed requests/responses, capability decisions,
   delegations, usage, costs, and session closure.
7. **Distribution:** signed releases macOS/Linux, checksum, self-update, rollback,
   and testing from packaged artifact.
8. **Strong Isolation:** sandbox runners per platform for processes and MCP.

## Mandatory Order

1. Workflow/contracts.
2. Ledger and usage.
3. Complete scheduler/broker.
4. MCPProvider and internal hooks.
5. Sandbox.
6. Signed packaging.

Each item is implemented with RED test, integration test, and black-box test.
No client mechanisms or legacy code are reintroduced.
