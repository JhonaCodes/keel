# Keel — registro de parciales (nada oculto)

> Registro **único y honesto** de todo lo que está incompleto, es forward-declaration
> o está diferido. Nace de una auditoría exhaustiva del código real (todos los `.rs`
> de `crates/` + `test/`, las 18 invariantes y las 3 planning docs). Complementa a
> [`STATUS.md`](../STATUS.md) (matriz punto a punto), [`ROADMAP.md`](ROADMAP.md) y
> [`PROGRAMA_DE_TRABAJO.md`](PROGRAMA_DE_TRABAJO.md).
>
> **Hallazgo de la auditoría:** la sospecha de "muchas cosas a medias ocultas" en
> gran parte NO se confirma — las docs rastrean lo incompleto con alta fidelidad y
> las 18 invariantes tienen entrada. El **único partial funcional que estaba oculto**
> (guardado y nunca evaluado, sin registro en ningún doc) es `constraints` (F1); el
> resto ya estaba documentado o es una unidad entera de Phase 2.

Leyenda, sin zona gris:
- **✅ completo** — cableado y probado para lo que declara.
- **🔨 cerrable** — parcial real, se cierra en el track de verificación (no bloqueado).
- **⏭ Phase 2** — unidad ENTERA no empezada, gated por diseño (la spec la condiciona a la corrida real de Phase 0c). **No es "a medias"**: es trabajo no iniciado con su propio gate.

---

## ✅ Completo y probado (contexto)

| Pieza | Evidencia |
|---|---|
| L1 gate (bloqueo pre-acción, exit 2, packet en stderr) | `crates/keel-cli/src/gate.rs`; `test/tests/gate_exit_code.rs` |
| L2 skills (entrega compact 1×/sesión, referencia después, escala a full en oscilación) | `crates/keel-engine/src/session.rs:98-170` |
| L3 auditor semántico (`keel audit`, origin=semantic, invalid→review) | `crates/keel-engine/src/audit.rs`; `test/tests/audit_agent.rs` |
| Ledger append-only (declared vs effective, telemetría, oscilación) | `crates/keel-engine/src/ledger.rs` |
| Lock + binding + plano CI + preflight adapter | `lock.rs`, `commands.rs`, `adapter.rs`; `lock_cli.rs`, `ci_cli.rs`, `agent_lock.rs` |
| inv 12 outputSchema validation | `audit.rs:93-112` |
| inv 13 maxTokens (over budget → unknown + finding, tokens reales) | `audit.rs:113-160` |
| Phase 0c **harness** (medición passive vs enforce → reporte con delta) | `test/src/measure.rs`; `datasets/phase0c/` |

---

## 🔨 Abierto y cerrable ahora (track de verificación)

| # | Qué | Estado real | file:line | Falta | Tamaño | Fase |
|---|---|---|---|---|---|---|
| **F1** | `constraints` (`environment.allow/deny`) | ✅ **HECHO (Fase C, PR pendiente)**: tipado a `CompiledConstraints` en compile (shape malformado → error, no silent) y evaluado en runtime (`runtime::env_violation`): deny bloquea siempre, allow no-vacío = allowlist estricta. Fluye por el branch `invalid`. | `runtime.rs env_violation`; `snapshot.rs CompiledConstraints`; `compile.rs BadConstraints` | — | C ✅ |
| **G1/G2** | ContextPacket: campos + `source`+snapshot hash | ✅ **HECHO (Fase D)**: `render` emite línea `source: rule=… snapshot=sha256:…`; hash threadeado desde el snapshot | `packet.rs render` | — | D ✅ |
| **G4** | Exemplar garantizado en `block` + rule-debt | ✅ **HECHO (Fase D)**: `deliver_skills(force_exemplar)` re-adjunta el par en block aunque el skill esté cargado; warning de compile si un branch block carga skills sin exemplar | `session.rs deliver_skills`; `compile.rs` (rule-debt) | — | D ✅ |
| **Cov** | Cobertura e2e del packet | ✅ **HECHO (Fase D)**: `test/tests/packet_content.rs` asserta el packet en stderr del binario real (verdict+finding+source+snapshot+evidence) | `packet_content.rs` | — | D ✅ |
| **#12** | Aislamiento de tests vs config del usuario | `gate.rs:64-69` inyecta el env real → una precondición `env.present` puede voltearse por una var del host/CI | `gate.rs:64-69`, `tools.rs:141-149` | Flag `--no-inherit-env` + harness hermético (env scrub, clock/id fijos) | Chico-Medio | B |
| **#7** | Re-entrega de skill tras compactación | ✅ **HECHO (Fase E)**: evento de capa-sesión `context.compacted` resetea `loaded_skills` de la sesión → el próximo match re-entrega. Extensión consciente (no uno de los 17 eventos de gobernanza). | `event.rs ContextCompacted`; `gate.rs`; `session.rs` | — | E ✅ |
| **T3** | Baseline de seguridad del executor (secret-ref/allowlist/sandbox) — el limbo "permissions" de inv 13 | `run_executor` arma `Command` crudo, hereda env; **desbloqueado** (no gated por 0c) pero **no programado en este track** | `audit.rs::run_executor` | env_clear + allowlist + network-deny/read-only | Medio | (sin programar) |

---

## ⏭ Phase 2 — unidades enteras, gated (NO son "a medias")

Unidades ENTERAS no empezadas, cada una con su **propia** precondición (no todas por el mismo gate): la mayoría por la corrida REAL de Phase 0c (la spec gatea el crecimiento); Mono por una 2ª capa de autoridad; MCP por el diferido de ADR-005/006. Se consolidarán en `PHASE2_INITIATIVE.md` (se crea en Fase F); mientras, el detalle vive en [`PROGRAMA_DE_TRABAJO.md`](PROGRAMA_DE_TRABAJO.md) (T4-T11) y [`ROADMAP.md`](ROADMAP.md).

| # | Unidad | Precondición | Estado real | file:line | Doc |
|---|---|---|---|---|---|
| #1-exec | Ejecución real de agentes + `AgentExecutor`/proveedor-modelo seleccionable + broker/routing | Phase 0c real | `invoke.agent` se REGISTRA, no se ejecuta (salvo `keel audit` manual); no existe tipo `Routing` | `runtime.rs:227-234`; `snapshot.rs:354` | STATUS 14.4❌/14.5🟡; ROADMAP #6; PROGRAMA T4-T7 |
| #6 | Scheduler paralelo (máx configurable, agrupar dependientes, cabeza-de-sección, sobrantes → task sqlite) | Phase 0c real | No existe | — | ROADMAP/PROGRAMA (Phase 2) |
| SQL-tasks | Backlog de tareas en SQLite por proyecto (insertar → salir de la cola al claim/complete; done-log append-only) — substrate de #6 | Phase 0c real | No existe; hoy el único SQLite por workspace es el ledger (`.keel-state/ledger.sqlite`) | `ledger.rs` (patrón a reusar) | nuevo (pedido del usuario) |
| #8 | Guardrails de límites (continuar al resetear) | Phase 0c real | No existe | — | ROADMAP/PROGRAMA (Phase 2) |
| Cap | Activación de `capabilities` (limitar/activar en runtime) | Phase 0c real | Compilado y surfaced como texto "Phase 2", nunca activa nada | `runtime.rs:216-226`; `snapshot.rs:347` | ROADMAP #8 |
| Fases | Máquina de fases §6.2 / inv 17 (transiciones artifact-gated) | Phase 0c real | Enum de eventos existe; sin emisor ni gating; ningún adapter emite eventos de fase en vivo | `event.rs:20-56`, `event.rs:10-14`; `gate.rs:243-293` | STATUS 6.2/inv17🟡; ROADMAP #7; PROGRAMA T9 |
| #4 | skill-on-action en vivo | (depende de Fases) | El mecanismo funciona (regla en evento de fase entrega skill) pero depende de la máquina de fases de arriba | ejemplo `keel-dsl/tests/corpus/rules_11_4.yaml:182-193` | (parte de Fases arriba) |
| Mono | Monotonicidad de composición D1-D4 (§7.4) | 2ª capa de autoridad | Stub documentado; el lattice D3 ya está en `keel-core` | `compile.rs:177,197`; `keel-core/src/lib.rs:66-74` | STATUS inv15/7.4⏭; ROADMAP #4; PROGRAMA D1 |
| MCP | MCP gateway (§14.12) | ADR-005/006 diferidos | No existe en código | — | STATUS 14.12🟡; ROADMAP #9; PROGRAMA D2 |
| maxDepth | Límites de profundidad/coste cruzado de delegación (inv 13) | grafo de delegación (Phase 2) | Diferido | `audit.rs` | STATUS inv13🟡 |
| F4/F5 | Selección granular de inputs (`ToolCall.inputs`, `Invoke.inputs/output`) | modelo de contexto rico (Phase 2) | Se parsean y se **tiran al compilar** (round-trip Phase 0a); el runtime entrega el evento entero | `rule.rs:143-146,241-244`; `compile.rs:478-483,474` | (nuevo — antes solo doc de campo) |

---

## Las consideraciones del usuario → dónde viven

| # | Consideración | Estado | Dónde |
|---|---|---|---|
| 1 | agents ≠ skills (ortogonalidad) | ✅ **HECHO (Fase E)**: test `skills_and_agents_are_orthogonal_paths` | Fase E2 |
| 1-exec | agente con proveedor/modelo seleccionable (codex/claude/haiku) | ⏭ Phase 2 | PHASE2_INITIATIVE (Fase F) |
| 2 | tests de que el runtime funciona | 🔨 suite e2e | Fases B/D/E |
| 3 | gate bloquea + entrega contexto/condiciones | 🔨 | Fase D |
| 4 | skill-on-action (leer skill antes de escribir) | ⏭ (mecanismo listo, falta emisor de fase) | PHASE2 / Fase E3 |
| 5 | formatos que el LLM necesita | 🔨 | Fase D |
| 6 | scheduler máx agentes paralelos + cola sqlite | ⏭ Phase 2 | PHASE2_INITIATIVE (Fase F) |
| 7 | no recargar skill salvo pérdida por compact | 🔨 | Fase E1 |
| 8 | guardrails al agotar límites | ⏭ Phase 2 | PHASE2_INITIATIVE (Fase F) |
| 9 | script/tool en frío > IA (0 tokens) | 🔨 doctrina | Fase F1 |
| 10 | ML en procesos específicos | ⏭ investigación | Fase F3 |
| 11 | ~~bug anzco~~ | descartado (no es Keel) | — |
| 12 | aislar tests de las configs del usuario | 🔨 | Fase B |
| 13 | backlog de tareas en SQLite por proyecto (sale de la cola al claim/complete) | ⏭ Phase 2 (substrate de #6) | PHASE2_INITIATIVE (Fase F) |
