# RCCA — Organizational-scale extensions

**Status:** material deferred from core specification v0.9.1
**Activation condition:** this document is detailed and promoted to specification once Phase 0 demonstrates a material delta and Phases 1–2 produce operational data (ADR-020). Until then, it records intent and design constraints, not contracts.

The composition model and the monotonicity semantics of `locked` (core, section 7) are identical across all profiles. What this material adds is **distribution, signing, administration and identity** — not new rule semantics.

---

## 1. Enterprise profile

On top of the core's Team profile, the organization adds:

- a remote Control Plane as a distributed source of truth;
- a signed catalog of packages, rules and workflows;
- centrally administered blocking policies;
- permissions and roles;
- package distribution;
- central audit;
- mandatory execution on the compliance plane.

Powering off a local machine deletes no configuration: the source of truth remains in the versioned workspace or the Control Plane.

## 2. Control Plane

```text
RCCA Control Plane
├── organizations
├── policies
├── packages
├── signatures
└── central audit
```

Design constraints already decided:

- the Control Plane introduces no second semantics: it publishes the same DSL objects, signed;
- the edit → validate → persist → compile → publish-snapshot flow is identical to local;
- Control Plane downtime degrades to local last-known-good, never to an ungoverned session.

## 3. Developer identity and per-person permissions

The core resolves by repository identity and explicitly declares it does not express per-person permissions. This extension adds: roles (who can approve an `Exception` object, who certifies a workflow, who administers the catalog), integration with the organization's identity provider, and employee/contractor separation. Requirement: per-person permissions operate on the compliance plane; the local plane gains no bypass resistance by incorporating identity.

## 4. Strong local attestation

Evidence signing with keys outside the runtime user's reach, server-side verification of the evidence chain, and detection of binding/lock tampering from infrastructure the developer does not control. A project of its own, with its own threat model; the core deliberately excludes it (core section 5.1).

## 5. Certification of equivalent workflows

```text
Nui standard workflow ─┐
                       ├── Production Readiness Contract
Progressive RCCA flow ─┘
```

An alternative workflow is certified against: benchmarks; finding rate; evidence; cost; reproducibility; corporate policies.

Mandatorily unified: non-negotiable policies; output contracts; evidence schemas; allowed capabilities; security rules; acceptance criteria; delivery. Internal workflow steps need not be unified.

Profile-level customization allowed: model; client; authorized workflow; TDD/SDD where both are permitted; autonomy level; presentation. Not modifiable: blocking security; locked architecture; mandatory evidence; corporate permissions; delivery requirements.

## 6. Corporate repository registry

The organization maintains a signed registry (`repositories.yaml`, signed). If a corporate repository removes or alters its binding, the compliance plane: blocks integration; requires lock regeneration; reports invalid configuration. The local plane only degrades and records (core, 13.3). The LLM plays no part in this decision.

## 7. Web panel

Local or remote console: view effective configuration; edit definitions; validate schemas; diff before apply; compile; review conflicts and monotonicity violations; inspect sessions; findings and evidence by origin class; test tools; check adapters, executors and bindings; inspect child sessions and delegation costs; cancel an AgentInvocation; roll back to the last valid snapshot.

The panel maintains no second source of truth:

```text
edit → validate → persist in workspace/control plane → compile → publish snapshot
```

## 8. Reference CI (promoted to core in Phase 2)

```yaml
name: RCCA Audit
on:
  pull_request:
  push:
    branches: [main]
jobs:
  rcca:
    runs-on: ubuntu-latest
    permissions:
      contents: read
      pull-requests: write
    steps:
      - uses: actions/checkout@v4
      - name: Install RCCA
        run: rcca-install
      - name: Authenticate
        env:
          RCCA_TOKEN: ${{ secrets.RCCA_TOKEN }}
        run: rcca auth login --token "$RCCA_TOKEN"
      - name: Validate binding and lock
        run: rcca ci resolve
      - name: Run configured RCCA workflow
        run: rcca ci run
      - name: Upload evidence
        if: always()
        uses: actions/upload-artifact@v4
        with:
          name: rcca-results
          path: .artifacts/rcca/
```

CI depends on neither the developer's daemon nor their local MCP: it starts its own ephemeral runtime and resolves capabilities per configuration. If a workflow invokes secondary agents, CI resolves them from the same lock, and the job fails before executing if: the pinned executor is unavailable; its version does not satisfy the lock; it cannot produce structured output; a required credential is missing; data policy forbids the provider; the required sandbox cannot be applied.

## 9. Context economy (reference)

Three loading levels: nothing at start; compact on demand; full only on oscillation or request. The runtime does not expose hundreds of tools simultaneously: it activates only the capabilities applicable to the current state. A child agent receives independent, bounded context; neither the parent's conversation is copied by default, nor is the child's full result appended to the parent's context: the validated artifact and a phase-appropriate synthesis are returned.
