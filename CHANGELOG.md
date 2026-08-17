# Changelog

Starts from `0.11.0` forward — not a retroactive reconstruction of the
whole project history (that lives in `git log`). Versions here track
`Cargo.toml`'s workspace `version`.

## 0.19.2

- **Audit evidence is keyed on the change-set, not on the session.** The scope is
  `sha256(patch)`: it identifies the diff, so a GO for it is evidence whoever
  observed it. Keying the lookup on `session_id` meant correct evidence recorded
  under a different id than the one the gate evaluates simply vanished — `observe`
  accepted it, the ledger stored it, and the gate kept reporting "no audit". The
  usual way out of that dead end was signing the same scope again, which is how a
  safety net teaches people to sign things twice. `Ledger::audits_for_scope` adds
  the scope-keyed rows on top of the session's own, in both enforcement paths.

- **The shim broker had the workspace/repo conflation too.** `Broker::decide`
  computed `target_for_command` against the workspace root, so a `git commit`
  interposed by the shim inside a governed session fingerprinted the governance
  repo, exactly like the hook bridge did before 0.19.1. It now resolves the repo
  from the shim request's `cwd`, with the workspace as fallback.

## 0.19.1

- **The audit gates fingerprint the repo being shipped, not the workspace.**
  `keel gate` computed the change-set from the `--workspace` path, which is where
  the RULES live. Since `keel launch` wires the hook pinned to that path, in every
  project except the governance workspace itself the gate audited the wrong tree:

  - With the workspace clean, `--target commit` resolved to its empty staged
    change-set — a constant hash (`files: []`) that any session could sign without
    auditing a line of what it was shipping. The gate looked enforced and was not.
  - With the workspace on an unmerged branch, `--target pr` demanded an audit of
    *those* files before allowing a PR in an unrelated repo — unsatisfiable without
    a false attestation in the ledger.

  `audit::repo_root` now resolves the client's repository (hook payload `cwd`, then
  `KEEL_CLIENT_CWD`, `PWD`, the process directory; first git toplevel wins) and
  `keel gate`, `keel audit-scope` and the MCP `keel.audit.scope` all compute against
  it. The workspace stays the fallback, so a payload without a usable cwd behaves as
  before rather than losing the gate. `.keel/project.yaml` was never consulted for
  this and still is not — the binding selects composition layers, not the diff.

- **PR bases resolve against the remote ref.** `target_for_pr` now prefers
  `origin/<base>` over a bare `<base>`. A local base branch that has not been pulled
  silently inflates the diff: in the case that exposed this, a 4-file branch
  fingerprinted as 46 files against a stale local `master`, so the audit and the
  gate could never agree on a scope. Qualified names and bases with no remote
  counterpart are left untouched.

- **The block packet names the change-set.** The `audit.recorded_for_change`
  precondition now lists the files (first 20, then a count) and states that the
  auditor runs as an in-session subagent. A bare hash makes rediscovering the scope
  the session's problem, and the cheapest way out of that is signing something
  nobody read.

## 0.19.0

- **Native in-session subagents for agent delivery (`nativeSubagent`)**: an
  `Agent` may now declare `nativeSubagent: <name>`, the equivalent native subagent
  in the launched client (e.g. Claude Code's `~/.claude/agents/<name>`). When the
  governed client provides in-session subagents (Claude Code), keel delivers that
  agent as an IN-SESSION `Task` subagent instead of steering the model to the
  external `keel.agent.invoke` CLI:
  - Schema + DSL + compiler carry the new optional field into the snapshot
    (`CompiledAgent.native_subagent`), so a change to it is drift `keel lock
    --verify` detects. `executor` stays required and is the fallback.
  - `keel gate`'s routed catalog (`routed_catalog_block`) now splits agents into
    two labelled sections — **Subagentes Task in-session** (spawn with the `Task`
    tool; never `keel.agent.invoke`) for native-mapped agents under Claude Code,
    and **Agentes cross-model** (opt-in `keel.agent.invoke`) for everything else.
    The client is threaded through the delivery chain; `Client::provides_native_
    subagents()` gates the behavior.
  - `keel launch` exports `KEEL_CLIENT` into the child env so workspace tools
    (e.g. `keel-catalog`) can deliver agents the same way keel's native delivery
    does.
  - Rationale: the external CLI path runs under `env_clear` and can hang the
    mono-thread `keel mcp`; the default closure auditor and the everyday experts
    (`code-auditor`, `flutter-dart-expert`, `rn-expert`, …) already exist as
    native subagents and should run in-session. `keel.agent.invoke` is reserved
    for genuine opt-in cross-model second opinions (`keel_external_auditor`).
- **`install.sh` installs atomically (fixes `zsh: killed` after an upgrade)**:
  the installer staged nothing and let `install` write straight onto
  `$prefix/bin/keel`. On macOS overwriting a Mach-O that a live process has
  mapped does NOT fail with `ETXTBSY` — it invalidates the code signature the
  kernel cached for that vnode, so every subsequent exec of the path is
  SIGKILLed (`CODESIGNING` / "Taskgated Invalid Signature") and `keel claude`
  dies instantly with a bare `zsh: killed`. Confusingly `codesign -v` still
  reports "valid on disk", because the bytes are fine and only the cached vnode
  is poisoned. Upgrading while a `keel claude` session was running therefore
  bricked the binary — and, if `$prefix/bin/keel` was a symlink, `install`
  followed it and corrupted the link target instead of replacing the link.
  Both binaries now stage to `.<name>.new` and `mv` into place: rename(2) is
  atomic, lands on a fresh inode, and replaces a symlink rather than following
  it. Recovery on an already-bricked install: `rm` the binary before copying —
  the poisoned inode stays alive for running processes and the new path gets a
  correctly cached signature.

## 0.16.0

- **Opportune context delivery — deliver the right resource at the right moment
  (D-016)**: keel no longer delivers context only on the prompt or a block. It now
  hands the model the relevant governed resources AT EACH MOMENT — "you're about
  to touch this, keel has these" — via the hook `additionalContext` channel,
  never blocking and never overriding the client's permission flow:
  - `keel gate` delivers on `file.edited` / `command.requested` / `tool.requested`
    (PreToolUse), on the prompt (UserPromptSubmit, unchanged), and at
    `SessionStart` — one shared assembler (`build_delivery_context`) + one emitter
    (`emit_delivery`) tagged per channel; a trivial moment surfaces nothing.
  - **Catch-all matcher**: the generated Claude Code `settings.json` now matches
    every tool (`""`), so keel SEES each tool the model is about to use. Bash /
    Edit / Write / WebFetch map to their governance events; anything else (a
    native MCP tool, a read, a search) becomes the new `tool.requested`
    event — observe-only, never a hard block by itself. `SessionStart` is wired.
  - **Routing over all component kinds (D-016 extends D-014)**: `match` /
    `CompiledMatch` and `routing::route` now cover governed components
    (blueprints, knowledge, workflows…), not just skills and agents — so a
    blueprint is surfaced at the right moment too. `route` returns a
    `RouteResult { skills, agents, components }`.
  - Reuses the existing `deliver_skills` thrift (don't re-send what's loaded);
    an `autoload` skill still gets its content injected. Backward compatible.
    Recompile + relock after upgrading.

## 0.15.0

- **`Package` composition layer — technology bundles (D-015, invariant 3)**:
  `packages/<tech>/` (e.g. `packages/flutter/`, `packages/rust/`) is now a real
  composition layer, added to `LayerId::CHAIN` between Platform and Project and
  selected for every project in resolution. A technology's reusable
  rules/skills/agents/knowledge compose once and reach the model instead of
  being copied per project (invariant 2); applicability is decided by each
  component's own `scope` (a dart-scoped Flutter rule never fires on a Rust repo)
  and `match` (D-014), so no per-project dependency selector is needed. Versioned
  cross-workspace pinning (section 8.5) stays deferred; a package is versioned
  per component via `metadata.version`. Empty `packages/` stays inert
  (backward compatible).
- **Declarative capability routing — SDK foundation (D-014)**: skills and agents
  gain an optional `match` block (`terms`, `context`, `autoload`) declaring WHEN
  they apply to a prompt, and the compiler DERIVES routing terms from each
  capability's `id` + `description`/`objective` — so a capability routes with
  zero authoring while an author can refine for precision (the lever that
  disambiguates siblings, e.g. `coderabbit` among several review skills).
  Resolved into the hashed snapshot as `CompiledMatch` (`terms` + `derived` +
  `context` + `autoload`); structured cues like `pr`→`github_pr` are inferred
  deterministically. The `keel_` provenance prefix and structural tokens
  (`skill`/`agent`/`workflow`) are dropped from derived terms; domain words
  (`review`, `pr`, `commit`) are kept. Backward compatible (absent `match` →
  derived-only). Recompile + relock after upgrading.
- **Native routing at the prompt (D-014 runtime half)**: a new `keel_engine::
  routing` module scores every compiled skill/agent against the prompt —
  structured context (a PR/ticket URL or cue word, weight 3) > explicit term
  (2) > derived term (1), ties broken by specificity — and `keel gate` now
  emits the ranked shortlist as `additionalContext` on `prompt.submitted`,
  replacing the workspace bag-of-words catalog with deterministic, auditable
  code. Each surfaced capability carries its trigger (`term:coderabbit`), shown
  in the operator banner; an `autoload` skill with a strong match has its
  compact content injected. The tokenizer and context cues are shared with the
  compiler so compile-time derivation and run-time matching never drift.

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
