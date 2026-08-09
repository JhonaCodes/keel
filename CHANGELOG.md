# Changelog

Starts from `0.11.0` forward — not a retroactive reconstruction of the
whole project history (that lives in `git log`). Versions here track
`Cargo.toml`'s workspace `version`, which is independent of the "spec
version" used by `docs/RACC_reference_architecture_v0_9_1.md`.

## Unreleased

- **Workspace `.env` for secrets**: `keel launch`/`mcp`/`gate` load a gitignored
  `<workspace>/.env` into keel's environment before resolving `${VAR}`, so
  secrets (API keys, `HOME`) can live in the workspace instead of the shell
  profile. `KEY=VALUE` lines (with `#` comments, optional `export `, quoted
  values); a variable already exported in the shell is never overwritten. The
  file is loaded into keel's process only — a `CliModelExecutor` still runs
  `env_clear` + `PATH` + only the vars its `config.env` declares, so the whole
  secret set is never handed to a child.

- **Evidence auto-capture** (`keel gate`, client-hook bridge): when a Bash
  command that is a known test runner (`cargo test`, `flutter test`, `pytest`,
  `npm test`, …) completes, keel synthesizes a `test.completed` event whose
  content carries the pass/fail signal from the real exit code. This lets an
  `evidence.recorded` precondition auto-unblock (e.g. "no write until a RED test
  was recorded") from an OBSERVED run, without the model hand-feeding evidence —
  the port of jflow's evidence-capture into the Keel enforcement path.
- **`ModelExecutor` `config.env`**: a governed CLI executor can now declare
  extra environment variables for its child (still `env_clear` + `PATH` only
  otherwise). A `${NAME}` value inherits from keel's environment (same
  convention as MCP provider configs). Fixes agent invocation for CLIs that need
  `HOME` to find their auth config (`claude -p`, `codex`), which previously
  failed under the stripped environment.

## 0.11.0

- `evidence.recorded` builtin precondition: block an action until the
  session's ledger already contains evidence of a given past event (and,
  optionally, verdict) — the generic counterpart of `skill.loaded` for any
  event kind, e.g. "no write until a RED test was recorded this session."
- `KnowledgeChain` (`kind: Knowledge`): hash-chained, append-only growth for
  memory that grows session to session, anchored in `.keel/keel.lock` as
  `knowledge_checkpoints` so growth never triggers false drift in `keel lock
  --verify`, while `keel knowledge verify` recomputes the chain from storage
  and catches retroactive tampering.
