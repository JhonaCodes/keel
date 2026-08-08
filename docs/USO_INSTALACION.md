# Installation, Configuration, and Usage

Keel is a parent runtime that runs locally: it executes ABOVE the model's CLI
(Claude Code, Codex, or others), contains it within the environment Keel creates,
and governs its actions deterministically. Keel does NOT use APIs from model
providers (see `DECISIONES.md`, D-012).

## Install from Repository

```bash
./install.sh
export PATH="$HOME/.local/bin:$PATH"
```

The installer supports macOS and Linux, compiles with the repository lock, and
installs TWO binaries in `$HOME/.local/bin`: `keel` and `keel-shim` (the
interposition shim travels with keel; an installation without it is a broken
installation, fail-closed). The prefix can be changed:

```bash
./install.sh --prefix /controlled/path
```

## Create an Operational Workspace

```bash
keel init ~/keel-workspace
keel doctor --workspace ~/keel-workspace --governed
```

`init` scaffolds the composition layers (`global/`, `projects/<name>/`, ...),
compiles the snapshot, fixes the lock, opens the store, and **registers the
workspace as your default** (`~/.keel/config.json`), so `keel claude` works from
anywhere without `--workspace`. `doctor --governed` verifies that the snapshot
loads, the lock matches the published snapshot, and the store opens. (`--json` on
any command produces single-line output for scripts.)

Workspace resolution order: `--workspace` > `KEEL_WORKSPACE` > walk up from cwd
to `workspace.yaml` > **the registered default** > error.

## Run a Governed CLI

```bash
keel claude                 # or: keel codex
keel launch --client generic -- /bin/sh -c "<command>"   # any CLI
```

`keel <cli>` launches the CLI as a CHILD process under a PTY (passes interactive,
unmodified) within the environment Keel creates. A governed command (rm, git,
...) passes through a shim → Keel's broker → `evaluate_event` in Enforce mode:
if a rule denies it, **the command never becomes a process** (exit 2 +
ContextPacket on stderr); if not, the real binary executes.

Options:

```text
keel launch --client <id> [options] -- <cmd> [args...]
  --workspace <path>       # priority 1 (else: KEEL_WORKSPACE; else: walk-up to workspace.yaml)
  --containment full|shims # full (default) = shims + OS sandbox; shims = interposition only
  --no-suggest             # disables supervisor suggestions (P3); enforcement stays active
  --task "..."             # initial task, passed to CLI per its adapter
  --session <id>           # resume Keel session identity
```

## The Hard Ring: OS Containment

PATH interposition governs the PATH surface; an absolute path (`/bin/rm`) bypasses it.
For the ring the child cannot evade, declare a `kind: Containment` (subdir
`global/containment/`) with what the kernel can enforce:

```yaml
apiVersion: keel/v1alpha1
kind: Containment
metadata: { id: global.hard.protect-docs }
spec:
  denyUnlink: ["**/*.md"]     # cannot be deleted, even with /bin/rm
  denyWriteOutside: true      # writes confined to workspace
  denyNetwork: false
```

The Containment enters the snapshot hash (drift detectable by `keel lock
--verify`) and generates the OS sandbox profile. If no provider is available
or you use `--containment shims`, the level drops to shims WITH BANNER — never
silently.

Hard ring coverage differs by OS (macOS Seatbelt does exact `denyUnlink` globbing;
Linux Landlock does `denyWriteOutside`/`denyNetwork` but NOT extension globbing;
Windows = WSL2). See [`CONTENCION_MULTIPLATAFORMA.md`](CONTENCION_MULTIPLATAFORMA.md).

## Convergence: Skills and Agents Through Keel

On launch, Keel wires its MCP endpoint in the child. The model discovers and loads
its skills THROUGH Keel (not from its own config):

- `keel.skills.list` — catalog of governed skills + load state.
- `keel.skills.load` — delivers content to context and registers receipt.
- `keel.rules.query` — which rules apply to a command/path (advisory).
- `keel.agent.invoke` — runs a governed agent (possibly in ANOTHER model, via
  a local CLI executor) and returns its output validated against its
  `outputSchema`. Cross-model, deterministic, no API.

A `kind: ModelExecutor` declares a local COMMAND:

```yaml
apiVersion: keel/v1alpha1
kind: ModelExecutor
metadata: { id: auditor-cli, version: 1.0.0 }
spec:
  config:
    command: [codex, exec, --json]   # Keel runs this; prompt via stdin, stdout = response
```

If the child ignores or deletes the MCP config, nothing breaks: convergence is
not enforcement; hard rings (shims, sandbox) are independent and always active.

## Cognitive Direction

The supervisor observes the ledger live and, on a deterministic signal that the
model is stuck (oscillation: the same rule blocking 3 times in the session),
SURFACES a suggestion to the operator in the transcript. It does NOT write to the
model's stream: Keel helps without interfering with its reasoning. `--no-suggest`
silences it.

## Development and Verification

```bash
cargo test --workspace --locked
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

Signed binary releases, self-update, and remote rollback remain distribution work;
the installer from checkout is functional today.
