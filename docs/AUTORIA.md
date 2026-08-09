# Authoring Guide — How to Create Each Type

Practical reference for authoring components in a Keel workspace — by a human or
any AI. Each `kind` goes in its folder by convention; `keel init` already leaves
a README + a `.example` in each (the loader IGNORES `.example`: nothing activates
until renamed to `<name>.yaml`).

Always overwrite the mental model: **the rule declares; the tool implements;
the tool is code.** A cold-decidable process is a deterministic tool (0
tokens), not a model call.

Minimum flow after authoring anything:

```bash
keel compile --workspace <ws>   # validates schema + runs RuleTests + publishes snapshot
keel lock --workspace <ws>      # fixes lock to snapshot (drift detectable with --verify)
```

Layers live in `global/` (applies everywhere), `projects/<name>/` (that project
only), and elsewhere (section 8.5). Examples use `global/`; you can author the
same under `projects/app/`.

---

## Rule — the rule that governs an action

Folder: `global/rules/<name>.yaml`. Decides on an EVENT (e.g.
`command.requested`, `file.edited`). The verdict comes from a `validate` (a
tool), and `enforcement` maps the verdict to a decision.

```yaml
apiVersion: keel/v1alpha1
kind: Rule
metadata: { id: global.no-delete-md, author: jhonacode, adrRef: adr:ADR-001, reviewAfter: P6M }
spec:
  reversibility: irreversible          # delete is irreversible; an `unknown` escalates to human
  on: [command.requested]              # events that trigger the rule
  validate: { using: tool:no-delete-md }   # the tool that gives the verdict (valid/invalid/unknown)
  enforcement:
    invalid: { decision: block, report: { message: "deleting .md files is forbidden" } }
    valid:   { decision: allow }
```

- **Mandatory in metadata:** `id`, `author`, `adrRef`, `reviewAfter` (ADR-023).
- **`on`:** one or more of the 18 event kinds; the inner ring
  (`command.requested`) is pre-action (real blocking).
- **`decision`:** `allow` < `review` < `block` < `deny-pending-approval`.
- **Optional:** `scope: { paths: { include: ["src/**"] } }` (limit by path),
  `detect` (cheap pre-filter), `preconditions` (state of the world), `locked: true`
  (a lower layer can only strengthen it).
- **Force a skill (or agent) for a job:** a `builtin:skill.loaded` precondition
  blocks the action until the session has loaded that skill through Keel — so
  Keel does NOT suggest, it REQUIRES. The packet tells the model which to load
  (`keel.skills.load`); after loading, it retries and passes. Example: require
  `web-guide` before a `git`:

  ```yaml
  spec:
    on: [command.requested]
    detect: { using: "builtin:command.classify", with: { families: ["git"] } }
    preconditions:
      - using: "builtin:skill.loaded"
        with: { id: web-guide }
        onFail: block          # deny | block | review
    enforcement:
      valid: { decision: allow }
  ```

  (Builtin preconditions: `env.present`, `flag.present`, `skill.loaded`,
  `evidence.recorded`.) Note: this governs COMMANDS Keel sees via shims; an
  internal client write that doesn't pass through a command doesn't trigger
  the rule.
- **Gap (verify before relying on it):** live PATH-shim interposition only
  covers a FIXED default set of command names —
  `DEFAULT_SHIM_COMMANDS` in `crates/keel-engine/src/adapter.rs`: `rm`,
  `unlink`, `mv`, `git`, `dd`, `shred`. A `command.classify` family outside
  this list compiles cleanly and its `RuleTest` passes (RuleTests evaluate
  the rule engine directly, bypassing the shim layer), but the rule NEVER
  fires in a real `keel launch` session — nothing warns you at compile time
  (`keel_engine::adapter::preflight` only catches event KINDS with zero
  interposition mechanism for the adapter, not specific ungimmed commands).
  If your rule needs to govern a command outside this list, it needs a
  `Containment` declaration (OS-sandbox ring) instead, or the shim list
  needs extending in code.
- **Require evidence of a past event in this session:** a
  `builtin:evidence.recorded` precondition blocks the action until the
  session's ledger already contains an event of the given kind (and,
  optionally, verdict) — generic counterpart of `skill.loaded` for any past
  event, not just a loaded skill. Example: no file write until a RED test
  was recorded this session (needs a companion rule that marks
  `test.completed` invalid when the test failed — nothing is decided outside
  an authored rule):

  ```yaml
  spec:
    on: [file.edited]
    detect: { using: "builtin:command.classify", with: { families: ["src/**"] } }
    preconditions:
      - using: "builtin:evidence.recorded"
        with: { event: "test.completed", verdict: invalid }   # verdict is optional
        onFail: block
    enforcement:
      valid: { decision: allow }
  ```
  **Where does `test.completed` come from?** Under `keel claude`, keel's
  client-hook bridge auto-captures it: when a Bash command that is a known test
  runner (`cargo test`, `flutter test`, `pytest`, `npm test`, …) COMPLETES, keel
  synthesizes a `test.completed` event whose content carries the pass/fail
  signal from the real exit code (`FAILED` when it failed). So the companion
  rule above only needs to CLASSIFY it (`builtin:text.contains { value: FAILED }`
  → Invalid); the evidence is recorded by keel from the observed run, not the
  model's claim. This closes the loop: a real RED test unblocks the write; a
  passing one does not.
- **Gotcha:** builtin detectors (`text.regex`/`text.contains`) look at the
  CONTENT, not the command string. To decide on a command by its text, use an
  external tool (below).

---

## Tool — the code that decides (validate/detect/precondition)

Folder: `global/tools/<name>.yaml` + the script. Keel runs the `command`, passes
the EVENT as JSON on stdin, and interprets output per `output`.

```yaml
apiVersion: keel/v1alpha1
kind: Tool
metadata: { id: no-delete-md, version: 0.1.0 }
spec:
  command: [sh, global/tools/no-delete-md.sh]   # relative to workspace root
  timeoutMs: 5000
  output: exit-code        # exit 0 = valid | exit 1 = invalid | other = unknown
                           # (also: verdict-json | sarif)
```

The script (referenced above). Receives event on stdin; decides by exit code:

```sh
#!/bin/sh
payload="$(cat)"
cmd="$(printf '%s' "$payload" | sed -n 's/.*"command"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')"
first="$(printf '%s' "$cmd" | awk '{print $1}')"
case "${first##*/}" in
  rm|unlink)
    if printf '%s' "$cmd" | grep -qiE '\.md($|[^a-zA-Z0-9])'; then exit 1; fi ;;  # .md -> block
esac
exit 0   # everything else -> allow
```

- The ref from a rule is `tool:<id>`. `chmod +x` is not needed if you invoke with
  `[sh, ...]`.
- Exit-code contract: **0=allow, 1=block, any other=unknown** (fail-safe).

---

## Containment — the hard OS ring (kernel)

Folder: `global/containment/<name>.yaml`. Declares ONLY what the kernel can
enforce, regardless of PATH. Enters the snapshot hash; generates the OS sandbox
profile (macOS Seatbelt; Linux Landlock pending).

```yaml
apiVersion: keel/v1alpha1
kind: Containment
metadata: { id: global.hard.protect-docs }
spec:
  denyUnlink: ["**/*.md"]   # cannot be deleted, even with /bin/rm (exact glob on macOS)
  denyWriteOutside: true    # writes confined to workspace
  denyNetwork: false
```

- Composes by UNION across layers (restrictions only add up).
- **Coverage by OS:** see `CONTENCION_MULTIPLATAFORMA.md`. On Linux the `denyUnlink`
  glob is NOT kernel-hard (Landlock has no globs); it stays shim-only.

---

## Skill — knowledge Keel delivers to the model

Folders: `global/skills/<name>.yaml` + content `.md` files. The model loads it
via `keel.skills.load` (MCP); Keel registers the receipt.

```yaml
apiVersion: keel/v1alpha1
kind: Skill
metadata: { id: keel_review_pr_coderabbit, version: 0.1.0 }   # keel_ + keyword (see below)
spec:
  description: Resolve CodeRabbit review comments on a GitHub PR   # optional but recommended
  match:                                          # optional: declarative routing (D-014)
    terms: [coderabbit]                           # explicit alias — the disambiguator
    context: [github_pr]                          # structured object type
    autoload: false                               # true → inject on a strong match
  compact: global/skills/keel_review_pr_coderabbit_keel.md   # short variant (first delivery)
  full: global/skills/keel_review_pr_coderabbit_full_keel.md # optional: scales on oscillation
  examples:                                        # optional: pairs for packet exemplar
    - ["skim the PR by hand", "resolve each CodeRabbit thread"]
```

- **CONDITION (enforced on compile):** a skill's content files MUST end with
  `_keel.md`. A `compact`/`full` that doesn't comply is a compile error
  (`SkillNaming`). The suffix makes provenance legible — delivered BY Keel —
  wherever content is read.
- The `.md` is free text; Keel delivers it as-is to context.
- A rule can request it: `enforcement.invalid.load.skills: ["skill:keel_review_pr_coderabbit"]`.
- **`description` (recommended):** a one-line summary that flows into the compiled
  snapshot, so Keel can EXPOSE a catalog to the model (D-013) and DERIVE routing
  terms from it (D-014). See "Routing & naming standard" below.
- **`match` (optional, D-014):** declares WHEN this skill applies. `terms` are
  explicit aliases weighted above derived words (the lever that disambiguates
  siblings); `context` is a structured object type (`github_pr`, `github_issue`,
  `linear_ticket`, `jira_issue`); `autoload: true` injects the compact content on
  a strong match instead of only exposing it. Omit it entirely and Keel still
  routes the skill from terms derived off its `id` + `description`.

---

## Routing & naming standard (D-014) — read before authoring a skill or agent

Keel decides which skills/agents relate to a prompt DETERMINISTICALLY, from what
each capability declares. The name and the description ARE the automatic routing
signal, so author them on purpose. This standard is portable: any agent on any
machine authoring for a Keel workspace must follow it.

1. **`id` = `keel_` + contextual keyword(s).** The `keel_` prefix marks
   provenance (this is delivered by Keel) and is IGNORED by routing (it is a
   stop-word). What routes is the rest, so it MUST carry the discriminating
   keyword: `keel_review_pr_coderabbit`, not `keel_review_pr`. Structural words
   (`skill`, `agent`, `workflow`) are also dropped — they are not intent.
2. **`description` = one line with real semantics.** It is tokenized into derived
   routing terms and shown in the exposed catalog. "Resolve CodeRabbit comments
   on a GitHub PR" routes; "review helper" does not.
3. **Add `match.terms` for anything that must win over a sibling.** Two review-PR
   skills collide on `review`/`pr`; only the one that declares `terms:
   [coderabbit]` wins a CodeRabbit prompt. This is the precision lever.
4. **Add `match.context` for structured objects.** `github_pr`, `github_issue`,
   `linear_ticket`, `jira_issue` — matched from a URL or cue word in the prompt,
   the highest-weight signal. A cue like `pr`/`pull`/`linear`/`ticket` in the id
   or description infers it automatically, but declaring it is clearer.
5. **`autoload` sparingly.** Reserve it for a skill the model should always have
   when its trigger fires strongly; everything else is exposed, not forced.

Scoring (for intuition): structured context (3) > explicit term (2) > derived
term (1); ties break by specificity (more declared conditions win). An agent
declares the same `match` block; agents are exposed as suggestions, never
auto-invoked. **Governed components** (Blueprint, Knowledge, Workflow…) take the
same `description` + `match`, so a blueprint is surfaced at the right moment too.

**Opportune delivery (D-016).** keel surfaces the relevant resources not only on
the prompt but at each MOMENT — a file edit, a command, any tool, session start —
via the hook's `additionalContext`, without blocking. Two levers author it:
- `match` (above) drives WHAT is relevant to a moment (routed by the file
  content / command / prompt text).
- a rule with `enforcement.always.load.skills` on an event delivers a skill
  deterministically at that moment ("cognitive activation"): e.g. a rule
  `on:[file.edited]` scoped to `**/*.dart` that always loads `keel_code_rules`
  puts the Flutter rules in context every time the model edits Dart.

---

## Knowledge — versioned memory with verifiable integrity

Folder: `global/knowledge/<name>.yaml`. Memory that grows session to session
(notes, decisions, a running log the model writes back to) without
disturbing `keel lock --verify`. `spec.content` is a PATH, like Skill's
`compact`/`full` — the snapshot hash covers the path string, not the file's
bytes, so growth never looks like drift. Integrity is a separate, real
guarantee: each entry's hash chains the previous entry's hash (Merkle-log
shape), and the chain's head at lock time is anchored in `.keel/keel.lock`
as `knowledge_checkpoints` — versioned, so it can't be silently rewritten
alongside the chain.

```yaml
apiVersion: keel/v1alpha1
kind: Knowledge
metadata: { id: session-notes, version: 0.1.0 }
spec:
  content: .keel-state/knowledge/session-notes.sqlite   # path, not inline content
```

Grow it and verify it:

```sh
keel knowledge append --id session-notes --content "decided X because Y"
keel lock                 # anchors the current head as knowledge_checkpoints["knowledge:session-notes"]
keel knowledge append --id session-notes --content "follow-up: Z"
keel lock --verify        # still clean — growth after lock is not drift
keel knowledge verify --id session-notes   # recomputes the chain from storage;
                                            # reports where it broke, if a row
                                            # was tampered with directly (e.g. via sqlite3)
```

- Without `--id`, `keel knowledge verify` checks every declared `Knowledge`
  component.
- `knowledge_checkpoints` is deliberately excluded from `Lock::verify`'s
  drift comparison — that's what makes growth legitimate instead of drift.
  `keel knowledge verify` is the separate, explicit check for tampering.

---

## ModelExecutor — a local CLI as an "model" for agents

Folder: `global/executors/<name>.yaml`. Keel runs the `command`, passes the
prompt on stdin, and takes stdout as the response. **NOT a provider API**
(D-012): it's a local command.

```yaml
apiVersion: keel/v1alpha1
kind: ModelExecutor
metadata: { id: auditor-cli, version: 1.0.0 }
spec:
  config:
    command: [codex, exec, --json]   # or [claude, -p], or your own script
    env:                              # optional: extra env for the child
      HOME: "${HOME}"                 # ${VAR} inherits from keel's environment
```

- **`config.env` (optional):** the child runs with `env_clear` + `PATH` only, so
  it inherits NO ambient secrets. Most real CLIs still need a couple of vars to
  run — e.g. `claude -p` / `codex` need `HOME` to find their auth config. Declare
  them here: a value of the exact form `${NAME}` inherits `NAME` from keel's own
  environment (same convention as MCP provider configs); any other value is
  literal; a `${NAME}` that is unset resolves to empty. Without this, an executor
  whose CLI needs `HOME` fails to authenticate.
- **Where `${VAR}` comes from — a gitignored `<workspace>/.env`:** keel loads a
  `.env` at the workspace root into its environment before resolving `${VAR}`
  (in `keel launch`/`mcp`/`gate`). So put secrets like `ANTHROPIC_API_KEY=…` in
  `<workspace>/.env` (add `.env` to the workspace `.gitignore`), reference them
  with `${ANTHROPIC_API_KEY}` in `config.env`, and each executor gets only the
  vars it declares — the `.env` is never handed wholesale to the child. A
  variable already exported in the shell takes precedence over the file.

---

## Agent — a responsibility routed to an executor

Folder: `global/agents/<name>.yaml`. The model invokes it via
`keel.agent.invoke`; Keel runs it through the scheduler and validates its output
against the `outputSchema` before trusting it (cross-model).

```yaml
apiVersion: keel/v1alpha1
kind: Agent
metadata: { id: keel_auditor_silent_failures }   # keel_ + keyword (routing standard)
spec:
  role: audit                              # audit | review | implement
  executor: executor:auditor-cli           # the ModelExecutor that runs it
  objective: Audit a diff for swallowed errors and silent failures.
  match:                                    # optional: same routing model as skills (D-014)
    terms: [silent, failures]
  outputSchema: global/agents/verdict.schema.json   # optional: validates output (invariant 12)
```

- **`match` (optional, D-014):** same block as skills. Agents are EXPOSED as
  suggestions on a matching prompt (`keel.agent.invoke agent=<id>`), never
  auto-invoked; `autoload` does not apply. Terms are derived from `id` +
  `objective` when `match` is absent, so follow the naming standard above.

The schema (standard JSON Schema):

```json
{ "type": "object", "required": ["verdict"],
  "properties": { "verdict": { "type": "string" }, "note": { "type": "string" } } }
```

---

## RuleTest — test a rule, compile gate

Root folder `tests/<name>.yaml` (or `projects/<name>/tests/`). `keel compile`
runs them and does NOT publish if they fail. Compares the DECLARED decision.

```yaml
apiVersion: keel/v1alpha1
kind: RuleTest
metadata: { id: global.no-delete-md.blocks-md }
spec:
  target: rule:global.no-delete-md
  event: { kind: command.requested, command: rm notes.md }
  expect: { fired: true, verdict: invalid, decision: block, origin: deterministic }
```

Always author at least one block case and one allow case per rule.

- **Gap (verify before relying on it):** `schemas/ruletest.schema.json`'s
  `spec.event` does not expose `loaded_skills` or `recorded_evidence` —
  today there is no way to author a RuleTest fixture for the PASS path of a
  `skill.loaded` or `evidence.recorded` precondition (only the block path,
  which needs no session state). Verify that path with the real binary
  instead: `keel gate --client native` to record the evidence, then `keel
  launch`/`keel claude` and observe the command actually runs — see
  `test/tests/host_launch.rs::a_write_is_blocked_until_red_evidence_is_recorded_in_the_session`
  for the pattern, and `examples/starter-workspace/` for a copy-pasteable
  version of it.

---

## Exception — relax a `locked` rule, scoped

Folder: `global/exceptions/<name>.yaml`. The ONLY governed way to relax a
`locked` rule, within a scope and with expiry; registers as a human decision.

```yaml
apiVersion: keel/v1alpha1
kind: Exception
metadata: { id: reports-waiver }
spec:
  rule: rule:global.no-raw-queries       # the locked rule to relax
  owner: global                          # MUST be the layer that locked it
  reason: "Legacy reporting migrates next quarter."
  scope: { paths: { include: ["src/reports/**"] } }   # lock lifts ONLY here
  expiry: "2027-01-01"                   # an expired exception does nothing
```

---

## Where technology content goes — the `packages/` layer (D-015)

Reusable content for ONE technology lives in a package (invariant 3: reusable
components live in packages, not copied per project — invariant 2):

```text
packages/
├── flutter/            # namespaced by technology
│   ├── rules/          # e.g. widget-classes, no-DI-in-VMs (scope: languages:[dart])
│   ├── skills/         # e.g. keel_flutter_adaptive_ui  (match: terms:[flutter,adaptive])
│   ├── knowledge/
│   └── tools/
└── rust/
    ├── rules/          # scope: languages:[rust]
    └── skills/
```

- A `packages/<tech>/` bundle COMPOSES for every project (between the base
  layers and the project). What keeps a Flutter package off a Rust repo is each
  component's own declaration: a rule scopes to `languages: [dart]`, a skill
  declares `match`. So author those — an unscoped rule in a package would fire
  everywhere.
- Same `rules/ skills/ agents/ knowledge/ tools/` convention as any layer.
- Versioned per component via `metadata.version`; cross-workspace pinning is not
  implemented yet.

---

## Authoring Checklist

1. Put the file in its `kind` folder (renamed, no `.example`).
2. `keel compile` — if schema or a RuleTest fails, fix it (error points to exact field).
3. `keel lock` — fixes the snapshot.
4. For new rules: add its RuleTest (block + allow) before trusting it.
5. If the rule targets `command.requested`, confirm the command's family is
   in `DEFAULT_SHIM_COMMANDS` (see the Rule section above) — otherwise the
   RuleTest passes but the rule never fires live.

See [`examples/starter-workspace/`](../examples/starter-workspace/) for a
complete, copy-pasteable workspace exercising `env.present`,
`evidence.recorded`, and `Knowledge`, verified against the real binary.
