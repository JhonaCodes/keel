# Contributing to Keel

Thanks for your interest. Keel is open core: the runtime, CLI, engine,
runtime, compiler, CLI and specification (`docs/`) all live here under Apache-2.0.
Organizational-scale components are out of scope for this repository; pending
work is tracked in `docs/planificacion/`.

## Developer Certificate of Origin (DCO)

Contributions are accepted under the **DCO** — a lightweight, sign-off-based
alternative to a CLA. By signing off a commit you certify the
[Developer Certificate of Origin 1.1](https://developercertificate.org/):
that you wrote the change or have the right to submit it under the project's
license.

Sign off every commit by adding a `Signed-off-by` trailer:

```bash
git commit -s -m "your message"
# -> Signed-off-by: Your Name <you@example.com>
```

By signing off you agree your contribution is licensed under Apache-2.0.

## What a good change looks like

Keel's guarantees are structural, so contributions must preserve them:

- **Tests stay green.** `cargo test --workspace` (currently 61 tests). New
  behavior comes with a test that fails for the right reason first.
- **Forbidden dependency edges hold.** `compiler ⇏ runtime`, `runtime ⇏ dsl`,
  `snapshot ⇏ dsl`, `ledger ⇏ runtime`, and the session/packet/audit modules
  never import `keel-dsl`. This is enforced by
  `crates/keel-engine/tests/arch_boundaries.rs` — do not weaken it.
- **The ledger is append-only** and origin classes are never mixed
  (deterministic vs semantic, section 6.4).
- **`keel observe` never blocks** (telemetry, ADR-021); only governed capabilities
  enforces, and only preventable events (inner ring + completion) exit 2.
- **A semantic verdict never authorizes an irreversible action** (section 4.7).
- Every file keeps its `SPDX-License-Identifier` header.

The specification (`docs/`) is the source of truth; if code and spec disagree,
say so in the PR. The `keel-auditor` agent and `STATUS.md` track conformance.

## Workflow

1. Open an issue describing the problem or proposal.
2. Branch, write the test, implement, keep the suite green.
3. Open a PR with signed-off commits. Explain which spec section (section ) or ADR
   the change relates to.

## Trademark

The code is open; the name is not. See `TRADEMARK.md` before naming a fork or
derivative.
