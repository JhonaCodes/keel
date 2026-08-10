# Keel

<img src="assets/logo.png" alt="Keel Logo" style="width: 100%; max-width: 100%; height: auto; display: block;" />

Keel is a parent runtime that runs locally: it executes ABOVE the model's CLI
(Claude Code, Codex, or others), contains it within the environment it creates,
and governs its actions deterministically — before they happen, and in a way that
the model cannot misconfigure. Keel does NOT use APIs from model providers
(see `docs/DECISIONES.md`, D-012).

## Platform support

For now the runtime runs on **macOS** — including the hard OS-sandbox ring
(Seatbelt). **Linux is in progress**: the PTY + shim layer runs, but the kernel
sandbox (Landlock, F2b) is pending, so containment degrades to shims (with a
banner, never silently). Windows is via WSL2. See
`docs/CONTENCION_MULTIPLATAFORMA.md`.

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
keel claude          # init already registered ~/keel-workspace as the default — or: keel codex / keel opencode
```

`init` creates the workspace, binding, snapshot, lock, and SQLite store. It does not
create or modify provider configuration: keel governs the CLI's ENVIRONMENT, not
its API.

`keel claude [args...]` / `keel codex [args...]` / `keel opencode [args...]` is
a pure passthrough shorthand for `keel launch --client <name> -- [args...]` — it
does NOT parse `--workspace` (or any other `launch` flag) itself; anything after
the client name is forwarded as-is to the real client binary. To target a
workspace other than the current directory or the one `keel init`/`keel use` last
registered as default, use the full form:
`keel launch --client claude --workspace ~/keel-workspace -- <args>`.

`init` ships with zero rules on purpose — nothing is enforced until you author
some. For a working, non-trivial workspace with real rules and passing tests
to copy instead of starting blank, see
[`examples/starter-workspace/`](examples/starter-workspace/).

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

Supports `rules`, `tools`, `skills`, `agents`, `containment`, `exceptions`,
`knowledge`, `workflows`, `contracts`, `hooks`, `policies`, and
`executors` (local CLI commands). Components are validated, hashed, and
compiled into an immutable snapshot; the lock fixes them (`keel lock
--verify` detects drift). `rules`, `tools`, `skills`, `agents`, `containment`,
`exceptions`, and `knowledge` are enforced/consumed today; `workflows`,
`contracts`, `hooks`, and `policies` are validated and hashed
but not yet evaluated beyond generic storage (see `STATUS.md`).

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
