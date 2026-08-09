# Changelog

Starts from `0.11.0` forward — not a retroactive reconstruction of the
whole project history (that lives in `git log`). Versions here track
`Cargo.toml`'s workspace `version`, which is independent of the "spec
version" used by `docs/RACC_reference_architecture_v0_9_1.md`.

## 0.14.1

- **Visible enrichment banner**: when keel delivers context on a prompt (D-013),
  it now surfaces a concise line to the operator — `keel ✦ contexto entregado al
  modelo: <rules>` — via the client `systemMessage` and stderr, so it is clear
  on screen that keel injected context (and which rules produced it), not just
  in the model's hidden context.

## 0.14.0

- **`Skill.description`**: an optional one-line description on a `Skill` that
  flows into the compiled snapshot, so a `prompt.submitted` enrichment tool can
  EXPOSE a catalog of available skills to the model (D-013) — "we have these
  skills; here is what each is for" — without the model reading every skill's
  content. Backward compatible (absent → none).

## 0.13.0

- **Prompt enrichment — keel DELIVERS context on the prompt (D-013)**: keel now
  wires the `UserPromptSubmit` hook and maps the prompt to a `prompt.submitted`
  event. A rule `on: [prompt.submitted]` runs a tool whose output (`findings`)
  is delivered to the model as `additionalContext` — so the model RECEIVES the
  task already deserialized (e.g. a decomposed Linear/GitHub ticket with its id,
  for the PR to reference) instead of fetching it itself. Deterministic, by
  code, non-restrictive (never blocks a prompt); generic (any source: a tool per
  Linear/Jira/GitHub/skill-catalog). This extends doctrine from pull-only
  (skills via MCP) to also a governed push of context — see DECISIONES.md D-013.
- **`keel gate` governs `WebFetch`**: the client-hook bridge now maps a
  `WebFetch` tool call to a preventable `command.requested` event carrying the
  URL as `command`, so a rule can gate URL reads (e.g. force a governed tool
  instead of reading a Linear/Jira/GitHub URL directly). Previously the bridge
  saw only Bash/Edit/Write, so a direct `WebFetch` bypassed all rules.

## 0.12.1

- **`keel init` no longer steals an established default workspace**: init
  registers the default only when there is no valid one yet; a scratch/test
  `keel init` on another path leaves the operator's default untouched (deleting
  that scratch dir would otherwise break every `keel <cli>` with "no workspace
  found"). Switching the default stays explicit via `keel use <path>`.

## 0.12.0

- **Workspace `.env` for secrets**: `keel launch`/`mcp`/`gate` load a gitignored
  `<workspace>/.env` into keel's environment before resolving `${VAR}`, so
  secrets (API keys, `HOME`) can live in the workspace instead of the shell
  profile. `KEY=VALUE` lines (with `#` comments, optional `export `, quoted
  values); a variable already exported in the shell is never overwritten. The
  file is loaded into keel's process only — a `CliModelExecutor` still runs
  `env_clear` + `PATH` + only the vars its `config.env` declares, so the whole
  secret set is never handed to a child. Tool scripts inherit keel's
  environment (no `env_clear` in the tool runner), so they read the same
  secrets. `keel init` scaffolds a gitignored `.env` template (you fill in the
  keys by hand) and adds `.env` to the workspace `.gitignore`.

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
