# Starter workspace

A real, testable workspace — not a tutorial fixture. Every rule here has a
passing `RuleTest`, and every command below was run against the real `keel`
binary before this file was written. Copy this directory to start testing
Keel on something non-trivial instead of an empty `keel init` scaffold.

## What's in here

- `rules/require-deploy-token.yaml` — `env.present` precondition: blocks a
  `git` invocation unless `DEPLOY_TOKEN` is set in the environment
  (stand-in for "don't push/deploy without credentials"). Uses `git`
  specifically because live PATH-shim interposition only covers a fixed
  default set of commands (`rm`, `unlink`, `mv`, `git`, `dd`, `shred` —
  see `docs/AUTORIA.md`'s note on `DEFAULT_SHIM_COMMANDS`); a family not in
  that list compiles and passes `RuleTest`, but silently never fires live.
- `rules/record-test-result.yaml` + `rules/require-red-before-write.yaml` —
  the `evidence.recorded` pair: `record-test-result` leaves an evidence
  trail for `test.completed` (invalid when the content says `FAILED`);
  `require-red-before-write` FORCES that evidence to exist before an `rm`
  runs. Same pattern proven end to end in
  `test/tests/host_launch.rs::a_write_is_blocked_until_red_evidence_is_recorded_in_the_session`.
- `knowledge/session-notes.yaml` — a `Knowledge` component: memory that
  grows across sessions without ever showing up as drift in `keel lock
  --verify`.
- `tests/*.yaml` — one `RuleTest` per rule outcome (block case + allow case),
  run by `keel test`.

## Walkthrough

```bash
cp -r examples/starter-workspace /tmp/kw-test && cd /tmp/kw-test

# Compile + run the RuleTests (5 tests, all green)
keel compile
keel test

# Bind + lock (the compliance-plane baseline)
keel bind --project "project:example/starter"
keel lock

# Grow the Knowledge chain — does NOT drift the lock
keel knowledge append --id session-notes --content "first real entry"
keel lock --verify        # still clean
keel knowledge verify --id session-notes   # "OK — 1 entries, chain intact."
```

## See the deploy-token rule block, live

```bash
env -u DEPLOY_TOKEN keel launch --client generic -- /bin/sh -c "git --version"
# blocked — packet names `starter.require-deploy-token`, exit 2

DEPLOY_TOKEN=x keel launch --client generic -- /bin/sh -c "git --version"
# allowed (REVIEW, not silence — recorded in the ledger either way)
```

## See the evidence.recorded gate, live

`RuleTest` fixtures can't express the PASS path of `evidence.recorded` yet
(`schemas/ruletest.schema.json` doesn't expose `recorded_evidence` on the
event — see the gap noted in `docs/AUTORIA.md`'s RuleTest section). Verify it
with the real binary instead, exactly like the pattern in
`test/tests/host_launch.rs`:

```bash
echo "expendable" > target.txt

# No evidence yet this session → rm is refused, the process never exists.
keel launch --client generic --session s1 -- /bin/sh -c "rm target.txt"

# Record a RED test result for session s1.
echo '{"kind":"test.completed","session_id":"s1","content":"1 failed — assertion FAILED"}' \
  | keel gate --client native --session s1

# Same session, now WITH evidence → rm is allowed.
keel launch --client generic --session s1 -- /bin/sh -c "rm target.txt"
```

## What NOT to expect

Per `STATUS.md`, `RuntimeHost`/`Phase`/`PhaseController` are not wired into
this path — nothing here exercises a "phase machine." What you're testing is
the real enforcement path: `keel_engine::runtime::evaluate_event` through
the shim/PATH broker and the client-hook bridge.
