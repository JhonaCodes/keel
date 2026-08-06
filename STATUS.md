# Keel — conformance status against the specification

Point-by-point map of the implementation (product **Keel**; theory/spec name
**RCCA**) against `RCCA_reference_architecture_v0_9_1_EN.md` and `RCCA_future_EN.md`.

Legend: ✅ done · 🟡 partial · ❌ missing · ⏭ deferred by the spec itself.

**One-line honesty note (ADR-020/021):** the ledger was built first and records
every evaluation in both modes (`declared` vs `effective`). Enforcement now
exists (the three intervention layers below). The **Phase 0c experiment** —
measuring violations-reaching-review with vs. without enforcement on a real
repo — has **not been run**; that measurement, not the code, is what the spec
makes the gate for growing further.

Snapshot of the tree: 4 crates (`keel-core`, `keel-dsl`, `keel-engine`,
`keel-cli`), **82 tests green**. Kinds: Workspace, Rule, Tool, Skill,
Agent, AgentExecutor, RuleTest. Commands: `workspace init`, `compile`,
`observe`, `gate`, `audit`, `adapter` (+`--check` preflight), `bind`, `lock`,
`ci resolve`/`ci run`, `explain`, `prune`, `test`, `doctor`.

---

## The three intervention layers (the user's core requirement)

| Layer | When | Mechanism | Status | Evidence |
|---|---|---|---|---|
| **L1 pre-execution gate** | before an irreversible action runs | `keel gate` → Enforce mode → exit 2 + ContextPacket; the action never becomes a process (§5.3 inner ring) | ✅ | `keel-cli/src/gate.rs`, `keel-engine/src/runtime.rs` (Mode::Enforce), `packet.rs` |
| **L2 cognitive activation** | while working, a concept surfaces | rule `load.skills` → Session Manager delivers compact once, references thereafter, escalates to full on oscillation (§6.5, §14.12) | ✅ | `keel-engine/src/session.rs`, `kind: Skill` |
| **L3 post-action verification** | after write / at completion | post-edit feedback (outer ring), completion gate (§12.3), specialized auditor agent with `origin=semantic` (§14) | ✅ (seed) | `gate.rs` (completion), `keel-engine/src/audit.rs`, `kind: Agent`/`AgentExecutor` |

---

## §4 Principles & the 18 structural invariants (§4.9)

| Item | Status | Evidence / note |
|---|---|---|
| 4.1 freedom in analysis, discipline in execution | 🟡 | modes exist; full phase lifecycle is §6.2 (partial) |
| 4.2 LLM does not read config (ADR-004) | ✅ | runtime/snapshot/tools ⇏ dsl, enforced by `tests/arch_boundaries.rs` |
| 4.3 one logical integration per client | ✅ | adapter is a thin bridge (`gate --client`), rules in runtime |
| 4.4 rule declares, tool implements, tool is code | ✅ | `tools.rs`, external tool manifests, 0 tokens |
| 4.5 detector never decides | ✅ | `run_detector` returns hit/no-hit only; fail-open |
| 4.6 three-state verdicts | ✅ | `Verdict::{Valid,Invalid,Unknown}` in keel-core |
| 4.7 reversibility decides fate of `unknown` | ✅ | compiler floor to deny-pending-approval; auditor invalid→review |
| 4.8 observable vs attested | 🟡 | origin classes recorded; explicit `attested` guard type not yet modeled |
| **inv 1** unique id + owner | ✅ | component ids; duplicate-id compile error |
| **inv 2** never copied between scopes | ✅ | references only |
| **inv 3** reusable in versioned packages | ⏭ | single workspace, no packages yet |
| **inv 4** repo holds binding/lock/CI only | ✅ | `keel bind` → `.keel/project.yaml`; `keel lock` → `.keel/keel.lock`; `lock.rs` |
| **inv 5** local paths/secrets never versioned | ✅ | `.keel-state/` gitignored; commands stay relative |
| **inv 6** snapshot published only if compile+tests pass | ✅ | atomic compile (`commands.rs::compile`) |
| **inv 7** last valid snapshot retained | ✅ | `snapshot.prev.json` |
| **inv 8** blocking policy needs adapter control (preflight) | ✅ | `adapter.rs` capability manifest + `keel adapter --check` preflight rejects unhonorable blocks |
| **inv 9** local & CI same lock/hash | ✅ | `keel.lock` pins the snapshot hash; `keel lock --verify` / `keel ci resolve` fail on drift |
| **inv 10** secrets by reference | ⏭ | no secrets in scope yet |
| **inv 11** Agent declares what, Executor how/where | ✅ | `kind: Agent` / `kind: AgentExecutor` |
| **inv 12** child result schema-validated | 🟡 | `audit.rs` validates verdict-json shape; full JSON-Schema of AgentResult pending |
| **inv 13** delegation limits (depth/time/cost) | 🟡 | timeout enforced; maxDepth/cost budgets partial |
| **inv 14** executor/model change in provenance | 🟡 | recorded in ledger detail; not in a lock |
| **inv 15** composition monotonicity | ⏭ | one authority layer — documented stub (`compile.rs::composition_stub`) |
| **inv 16** session append-only, non-authoritative | ✅ | `session.rs` only records deliveries; ledger has no UPDATE/DELETE |
| **inv 17** phases owned by runtime, artifact-gated | 🟡 | completion gate exists; full phase machine §6.2 pending |
| **inv 18** adversarial input delimited as data | ✅ | `audit.rs` DATA markers (§13.2) |

## §5 Trust boundary & rings

| Item | Status | Evidence |
|---|---|---|
| 5.1 honest local threat model (cooperative) | ✅ | README + packets never claim inviolable local enforcement |
| 5.2 guarantee matrix by plane | ✅ | local plane + compliance CI plane (`keel ci`) reusing the same engine over the lock |
| 5.3 interception rings (inner pre-action / outer post-hoc) | ✅ | `gate.rs` `preventable` = inner ring + completion; file.edited = feedback |

## §6 Model & lifecycle

| Item | Status | Evidence |
|---|---|---|
| 6.1 components | ✅ | Rule/Detector/Tool/Skill/Agent/Executor/Snapshot |
| 6.2 phases owned by runtime | 🟡 | completion gate + audit phase; full Investigation→Delivery machine pending |
| 6.3 observable/attested guards | 🟡 | origin classes; typed guard conditions pending |
| 6.4 ledger: facts vs attestations | ✅ | `OriginClass`, deterministic never mixed with semantic |
| 6.5 blocking + oscillation | ✅ | `ledger.oscillations`, gate escalates to full skill |

## §7 Composition & `locked`

| Item | Status | Evidence |
|---|---|---|
| 7.1 resolution by repo identity | ✅ | `keel bind` derives `project:org/repo` from the git remote → `.keel/project.yaml` |
| 7.2 composition order | ⏭ | single layer |
| 7.3 inheritance types | ⏭ | — |
| 7.4 monotonicity D1–D4 | ⏭ | documented stub; lattice ready in `keel-core::Decision` |
| 7.5 session append-only | ✅ | see inv 16 |
| 7.6 conflicts not silently resolved | ✅ | duplicate-id compile error |
| 7.7 rule lifecycle / prune | ✅ | `keel prune` with ledger evidence + human decisions |

## §8–§13

| Item | Status | Evidence |
|---|---|---|
| 8 architecture / workspace layout | ✅ | crates + `workspace.rs` (rules/tools/skills/agents/tests) |
| 9 installation & operation | 🟡 | CLI commands; no signed installer / project attach |
| 10.1 compile pipeline | ✅ | `compile.rs` (parse→schema→refs→[composition stub]→conflicts→index→snapshot) |
| 10.2 atomic compilation | ✅ | staging → RuleTests gate → publish |
| 10.3 hot reload | ⏭ | ephemeral process (ADR-010) |
| 10.4 ContextPacket | ✅ | `packet.rs` — verdict + constraint + exemplar + evidence, no YAML/paths |
| 11 DSL | ✅ | envelope + all Phase kinds + JSON Schemas in `schemas/` |
| 11.4 preconditions (ADR-022) | ✅ | `env.present`/`flag.present` + external, live env at gate |
| 11.6 SARIF findings (ADR-016) | ✅ | `sarif.rs`; `finding.v1` absent |
| 12.1 adapter contract / manifest | ✅ | `AdapterManifest` (claude-code) + `keel adapter --check` compile-time preflight |
| 12.2 one logical integration | ✅ | hook transports, runtime decides |
| 12.3 completion authorization | ✅ | completion gate |
| 12.4 compatible vs governed mode | 🟡 | compatible only (no proxy) |
| 13.1 security baseline | 🟡 | env hygiene relied on; secret-ref/allowlists/sandbox pending |
| 13.2 adversarial content delimited | ✅ | `audit.rs` |
| 13.3 repository identity | ⏭ | compliance-plane concern |

## §14 Specialized agents & cross-model

| Item | Status | Evidence |
|---|---|---|
| 14.1 agent vs executor | ✅ | two kinds |
| 14.3 agent manifest | 🟡 | minimal (role/executor/output/budget); full manifest pending |
| 14.4 routing policy | ❌ | no AgentRoutingPolicy |
| 14.5 invoke from rule | 🟡 | recorded in eval; executed only via `keel audit` |
| 14.6–14.7 request/result flow | 🟡 | `audit.rs` request+validate; full AgentRequest/Result artifacts pending |
| 14.8 executor driver | ✅ | structured argv, `{prompt}` + stdin, no shell concat |
| 14.10 isolation/interaction modes | 🟡 | auditor read-only + timeout; worktrees/depth graph pending |
| 14.12 tools/MCP/skills | 🟡 | tools + skills done; MCP gateway pending |

## §15 Validation & phases

| Item | Status | Evidence |
|---|---|---|
| 15.1 Phase 0a — DSL expressiveness | ✅ | corpus §11.3–11.5 parses + round-trips (`keel-dsl/tests/corpus.rs`) |
| 15.1 Phase 0b — passive telemetry | ✅ built / 🟡 unproven | `keel observe` + ledger; needs real-session data |
| 15.1 Phase 0c — enforcement experiment | ❌ | the measurement gating further growth — **not run** |
| Phase 1 — local core | 🟡 | engine + enforcement + lock/binding + CI plane + preflight done; monotonicity (§7.4) still a stub |
| Phase 2 — full cycle & cross-model | 🟡 | audit seed + completion; broker/routing/full phases pending |
| §16 limitations | ✅ | acknowledged in README + here (cooperative local plane, etc.) |

## ADRs 1–23

001 ✅ · 002 ✅ · 003 ✅ · 004 ✅ · 005 ⏭(no MCP yet) · 006 ⏭ · 007 ✅(lock file + verify) ·
008 ✅(adapter preflight) · 009 ✅ · 010 ✅(ephemeral) · 011 ✅ · 012 🟡 · 013 🟡 · 014 🟡 · 015 ✅ ·
016 ✅ · 017 ✅ · 018 🟡 · 019 ✅ · 020 ✅(this doc honors it) · 021 ✅ · 022 ✅ · 023 ✅

## `RCCA_future.md`

All ⏭ by design (ADR-020): Control Plane, signed catalog, per-person identity,
workflow certification, web panel. None started.

---

## What to do next, in the spec's own order

Done since the first pass: **lock + binding (inv 4/9)** (`keel bind`/`keel lock`),
the **compliance CI plane (§5.2)** (`keel ci resolve`/`run` + `examples/ci/`),
and the **adapter capability preflight (inv 8, §12.1)** (`keel adapter --check`).
See `docs/ROADMAP.md` and `docs/PLAN_IMPLEMENTACION.md` for the full record.

Still open, in the spec's own order:

1. **Run Phase 0c** — capture real agent sessions, compare violations-to-review
   with/without `keel gate`. This is the decision point, not more features.
2. **Composition + monotonicity (§7.4)** — activate the stub once a second
   authority layer exists.
3. **Phase 2 (§14.4+)** — agent broker/routing + full phase machine, gated by
   the Phase 0c result.
