# Changelog

Starts from `0.11.0` forward — not a retroactive reconstruction of the
whole project history (that lives in `git log`). Versions here track
`Cargo.toml`'s workspace `version`, which is independent of the "spec
version" used by `docs/RACC_reference_architecture_v0_9_1.md`.

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
