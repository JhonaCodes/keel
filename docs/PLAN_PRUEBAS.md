# Keel — plan de pruebas (qué testear para ir cerrando y modificando)

> Checklist vivo. Marca `[x]` lo verificado. Divide en: (0) qué YA cubre la suite
> automática, (1) pruebas funcionales/e2e para hacer AHORA a mano y ganar
> confianza, (2) huecos de test a cerrar en el código actual, (3) tests que se
> agregan cuando aterrice cada unidad de Phase 2. Correr todo desde `RCCA/keel/`.

## 0 · Ya cubierto por la suite (108 tests) — no reescribir, solo correr
- `cargo test --workspace` — 108 verdes. Cubre:
  - [x] L1 gate: exit 2 en violación de anillo interno, exit 0 limpio, feedback outer-ring, completion denegado (`test/tests/gate_exit_code.rs`).
  - [x] L2 skills: entrega 1×, referencia después, escala a full en oscilación (`crates/keel-engine/tests-unit/session.rs`).
  - [x] L3 audit: verdict semántico→review, outputSchema (inv 12), maxTokens (inv 13), timeout→unknown, spawn real (`test/tests/audit_agent.rs`).
  - [x] constraints environment.allow/deny (F1) — deny bloquea, allow allowlist, malformado→compile error (`tests-unit/runtime.rs`, `tests-unit/compile.rs`, `test/tests/constraints.rs`).
  - [x] ContextPacket: source+snapshot hash, exemplar en block (`tests-unit/packet.rs`, `test/tests/packet_content.rs`).
  - [x] Aislamiento hermético: env del host no voltea preconditions (`test/tests/isolation.rs`).
  - [x] context.compacted resetea skills; agents≠skills (`test/tests/compaction.rs`, `tests-unit/runtime.rs`).
  - [x] env isolation del executor: no hereda env salvo allowlist (`test/tests/audit_agent.rs`).
  - [x] Phase 0c harness e2e produce reporte con delta (`test/tests/phase0c_harness.rs`).
  - [x] lock/CI/adapter preflight/agent-lock (`test/tests/{lock_cli,ci_cli,adapter_cli,agent_lock}.rs`).

## 1 · Pruebas funcionales / e2e a hacer AHORA (a mano, contra el binario real)
Ganar confianza viendo el runtime actuar, no solo "verde":
- [ ] **Wrap del LLM (la demo):** compilar un workspace con una regla builtin y correr `keel gate` con un comando que viola → ver `BLOCKED` + packet + exit 2; con uno limpio → exit 0. (Reproduce lo que ya vimos.)
- [ ] **Ledger honesto:** tras varios `gate`, `keel explain <ev_id>` resuelve regla+veredicto+decisión+detail; `keel prune` propone keep/adjust/prune con datos.
- [ ] **observe (pasivo):** correr `keel observe --events <jsonl>` y confirmar que NADA bloquea (effective ≤ review) pero todo queda en el ledger (declared preservado).
- [ ] **constraints de entorno:** regla con `constraints.environment.deny:[production]` → un comando con "production" en la connection string bloquea; con "local" pasa.
- [ ] **maxTokens:** un `AgentExecutor` que reporta tokens > budget → `keel audit` da `unknown` + finding; ledger con tokens reales.
- [ ] **Aislamiento de env del executor:** `AgentExecutor` sin `env` → el subproceso no ve secretos del host; con `env:[VAR]` → sí.
- [ ] **context.compacted:** cargar un skill, mandar `context.compacted`, confirmar que se re-entrega en el próximo match.
- [ ] **Phase 0c harness:** `cargo run -p keel-tests --bin keel-measure -- --dataset datasets/phase0c/v0-synthetic --out target/phase0c/v0` → leer `report.md` (delta, cross-check exit-2, gaps anotados).
- [ ] **Adapter real (Claude Code):** `keel adapter claude-code --print` → pegar el hook en un `.claude/settings.json` de prueba y ver el gate actuar en una sesión real (end-to-end con el LLM de verdad — la prueba definitiva del wrap).

## 2 · Huecos de test a cerrar (código actual)
- [ ] **exit==2 directo por evento** (ya hay `violating_command_request_exits_2`; confirmar cobertura de transition/delivery.requested inner-ring, no solo command).
- [ ] **isolation test sin skip vacuo:** hoy `executor_does_not_inherit_host_env…` hace early-return si no hay `$HOME`. Sustituir por una var-sonda garantizada (sin `set_var`, que es unsafe en edition 2024) — p. ej. un archivo/patrón que no dependa del entorno.
- [ ] **prune con reviewAfter vencido:** test de fecha (hoy depende del reloj real; inyectar ts fijo por el path de test).
- [ ] **doctor:** test e2e de `keel doctor` (verde y con snapshot ausente).

## 3 · Tests a agregar cuando aterrice cada unidad de Phase 2
(No escribir hasta que la unidad exista; ver `docs/PHASE2_INITIATIVE.md`.)
- [ ] **#1 broker/routing + proveedor seleccionable:** un `invoke.agent` en gate SÍ ejecuta vía broker; routing elige executor por `when`; fallback on timeout/unavailable; `neverOn: policy-denied`; provenance en el ledger.
- [ ] **#6 scheduler + backlog SQLite:** máximo de paralelos respetado; dependientes agrupados; sobrante encolado en SQLite; al tomar sale de la cola; done-log append-only.
- [ ] **#8 guardrails:** al agotar límite, pausa y reanuda al resetear; la cola SQLite sobrevive.
- [ ] **capabilities activación:** una capability declarada realmente habilita/limita; `availableCapabilities` llega al ContextPacket (cierra G3).
- [ ] **máquina de fases:** transición autorizada por artefacto; el modelo NO puede declarar su fase; skill-on-action dispara en vivo (#4).
- [ ] **monotonicidad D1–D4:** al haber 2ª capa, un debilitamiento (D1/D2/D3/D4) es compile error con el diff exacto.

## Comandos base
```sh
cargo test --workspace          # suite completa (108)
cargo fmt --all --check         # formato
cargo clippy --all-targets --all-features -- -D warnings   # lint bloqueante
cargo run -p keel-tests --bin keel-measure -- --dataset datasets/phase0c/v0-synthetic --out target/phase0c/v0
```
