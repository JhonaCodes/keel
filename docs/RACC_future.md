# RACC — Organizational Scale Extensions

**Status:** material deferred from core specification v0.9
**Activation condition:** this document is detailed and promoted to specification when Phase 0 demonstrates material delta and Phases 1–2 produce operational data (ADR-020). Until then, it records intention and design constraints, not contracts.

The composition model and monotonicity semantics of `locked` (core, section 7) are identical across all profiles. What this material adds is **distribution, signing, administration, and identity**, not new rule semantics.

---

## 1. Enterprise Profile

On top of the core Team profile, the organization adds:

- Remote Control Plane as distributed source of truth;
- signed catalog of packages, rules, and workflows;
- centrally-managed blocking policies;
- permissions and roles;
- package distribution;
- central audit;
- mandatory execution on compliance plane.

Turning off a local team does not eliminate configuration: the source of truth remains in the versioned workspace or Control Plane.

## 2. Control Plane

```text
Keel Control Plane (RACC implementation)
├── organizations
├── policies
├── packages
├── signatures
└── central audit
```

Design constraints already decided:

- the Control Plane introduces no second semantics: it publishes the same DSL objects, signed;
- the flow edit → validate → persist → compile → publish snapshot is identical to local;
- Control Plane downtime degrades to last-known-good local, never to ungoverned session.

## 3. Developer Identity and Per-Person Permissions

The core resolves by repository identity and explicitly declares it does not express per-person permissions. This extension adds: roles (who can approve an `Exception` object, who certifies a workflow, who administers the catalog), integration with the organization's identity provider, and employee/contractor separation. Requirement: per-person permissions operate on the compliance plane; the local plane gains no ability to resist evasion by incorporating identity.

## 4. Strong Local Attestation

Evidence signing with keys out of reach of the runtime user, server-side verification of the evidence chain, and detection of binding/lock manipulation from infrastructure not controlled by the developer. Its own project with its own threat model; the core deliberately excludes it (core section 5.1).

## 5. Equivalent Workflow Certification

```text
Nui standard workflow ─┐
                       ├── Production Readiness Contract (RACC-defined)
Progressive Keel flow ─┘
```

An alternative workflow certifies against: benchmarks; finding rate; evidence; cost; reproducibility; corporate policies.

What must be unified: non-negotiable policies; output contracts; evidence schemas; permitted capabilities; security rules; acceptance criteria; delivery. It is not mandatory to unify the workflow's internal steps.

Customization allowed per profile: model; client; authorized workflow; TDD/SDD if both are allowed; autonomy level; presentation. Not modifiable: blocking security; locked architecture; mandatory evidence; corporate permissions; delivery requirements.

## 6. Corporate Repository Registry

The organization maintains a signed registry (`repositories.yaml` with signature). If a corporate repository removes or alters its binding, the compliance plane: blocks integration; requires lock regeneration; reports invalid config. The local plane only degrades and registers (core, 13.3). The LLM does not intervene in this decision.

## 7. Web Panel

Local or remote console: view effective config; edit definitions; validate schemas; diff before apply; compile; review conflicts and monotonicity violations; inspect sessions; findings and evidence by source class; test tools; check adapters, executors, and bindings; inspect child sessions and delegation costs; cancel an AgentInvocation; rollback to last valid snapshot.

The panel maintains no second source of truth:

```text
edit → validate → persist to workspace/control plane → compile → publish snapshot
```

## 8. Reference CI (promoted to core in Phase 2)

```yaml
name: Keel RACC Audit
on:
  pull_request:
  push:
    branches: [main]
jobs:
  keel-racc:
    runs-on: ubuntu-latest
    permissions:
      contents: read
      pull-requests: write
    steps:
      - uses: actions/checkout@v4
      - name: Install Keel
        run: ./install.sh
      - name: Authenticate
        env:
          KEEL_TOKEN: ${{ secrets.KEEL_TOKEN }}
        run: keel configure executor default
      - name: Validate binding and lock
        run: keel doctor --workspace . --governed
      - name: Run configured Keel workflow
        run: keel run --workspace . --task "audit"
      - name: Upload evidence
        if: always()
        uses: actions/upload-artifact@v4
        with:
          name: keel-results
          path: .artifacts/keel/
```

CI does not depend on the daemon or the developer's local MCP: it starts its ephemeral runtime and resolves capabilities per config. If a workflow invokes secondary agents, CI resolves them from the same lock, and the job fails before executing if: the fixed executor is not available; its version doesn't meet the lock; it doesn't produce structured output; a credential is missing; data policy forbids the provider; the required sandbox cannot be applied.

## 9. Context Economy (Reference)

Three levels of loading: nothing at start; compact on demand; full only on oscillation or request. The runtime does not expose hundreds of tools simultaneously: it activates only capabilities applicable to the current state. A child agent receives independent and scoped context; neither is the parent's conversation copied by default, nor is the child's complete result added to the parent's context: the validated artifact and a synthesis appropriate to the phase are returned.
