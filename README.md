# Keel

<img src="assets/logo.png" alt="Keel Logo" style="width: 100%; max-width: 100%; height: auto; display: block;" />

Keel is a parent runtime that runs locally: it executes ABOVE the model's CLI
(Claude Code, Codex, or others), contains it within the environment it creates,
and governs its actions deterministically — before they happen, and in a way that
the model cannot misconfigure. Keel does NOT use APIs from model providers
(see `docs/DECISIONES.md`, D-012).

## Local Installation

```bash
./install.sh
export PATH="$HOME/.local/bin:$PATH"
keel --version
```

Installs `keel` and `keel-shim` (they travel together).

## Quick Start

```bash
keel init ~/keel-workspace --json
keel doctor --workspace ~/keel-workspace --governed --json
keel claude --workspace ~/keel-workspace     # or: keel codex, or keel launch --client generic -- <cmd>
```

`init` creates the workspace, binding, snapshot, lock, and SQLite store. It does not
create or modify provider configuration: keel governs the CLI's ENVIRONMENT, not
its API.

## How It Governs (Three Planes)

```text
keel <cli>
  -> PTY: the CLI runs interactively, unmodified
  -> P1 shims: governed command -> broker -> evaluate_event(Enforce)
        block => exit 2 + ContextPacket (never becomes a process)
  -> P1 OS sandbox: generated profile from `kind: Containment` (hard ring)
  -> P2 MCP: the model queries/loads skills and agents THROUGH keel
  -> P3 supervisor: suggests to operator on oscillation (without interfering)
```

P1 enforcement never depends on model cooperation or P2 convergence. Agents are
local CLI executors: a session in one model can request an audit that runs in
another, with no API.

## Workspace

Supports `rules`, `tools`, `skills`, `agents`, `containment`, `blueprints`,
`knowledge`, `workflows`, `contracts`, `hooks`, `policies`, and `executors`
(local CLI commands). Components are validated, hashed, and compiled into an
immutable snapshot; the lock fixes them (`keel lock --verify` detects drift).

## Development

```bash
cargo test --workspace --locked
cargo clippy --workspace --all-targets -- -D warnings
```

## Documentation

- **[Usage and Installation](docs/USO_INSTALACION.md)** — complete guide for installation and first steps
- **[Design Decisions](docs/DECISIONES.md)** — justification for architectural decisions
- **[Runtime Architecture](docs/ARQUITECTURA_RUNTIME.md)** — internal design and components
- **[Runtime Contracts](docs/CONTRATOS_RUNTIME.md)** — behavioral specifications
- **[Component Authoring](docs/AUTORIA.md)** — how to create rules, skills, agents, and workflows
- **[Master Plan](docs/planificacion/ordenes_trabajo/PLAN_MAESTRO.md)** — work order and limits

For more on the RACC architecture:
- **[RACC Reference Architecture](docs/RACC_reference_architecture_v0_9_1.md)** — complete specification (v0.9.1)
