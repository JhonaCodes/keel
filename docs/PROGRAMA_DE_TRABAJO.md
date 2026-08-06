# Keel — Programa de trabajo

> **Cómo usar este documento.** Es el backlog vivo y autoexplicativo del proyecto.
> Para responder "¿qué falta?", leé el **Tablero** y tomá la primera tarea con
> estado `LISTO` (no bloqueada). Cada ficha trae **por qué**, **contexto**
> (archivos/evidencia), **qué hacer** y **hecho cuando**. Al terminar una tarea,
> marcá su casilla, actualizá `STATUS.md` y dejá aquí el número de PR.
>
> Complementa a [`STATUS.md`](../STATUS.md) (matriz de conformidad),
> [`ROADMAP.md`](ROADMAP.md) (prioridades) y
> [`FUNCIONAMIENTO_INTERNO.md`](FUNCIONAMIENTO_INTERNO.md) (cómo funciona hoy).
>
> Estado del repo al escribir esto: `main` verde, 85 tests, fmt/clippy limpios,
> lint del CI bloqueante; invariantes 4/8/9/11/12/14 ✅.

**Estados:** `LISTO` (accionable ya, sin bloqueo) · `BLOQUEADO` (espera a otra tarea) · `DIFERIDO` (fuera de alcance por diseño — NO es deuda).

---

## Tablero

| ID | Tarea | Estado | Bloqueo / orden |
|----|---|---|---|
| **T1** | Phase 0c — experimento de enforcement (medición) ⭐ | **LISTO** | ninguno — es el gate de crecimiento |
| **T2** | Conteo de tokens del executor + enforcement de `maxTokens` (inv 13) | **LISTO** | ninguno |
| **T3** | Baseline de seguridad del executor (sección 13.1) | **LISTO** | ninguno |
| **T4** | `AgentRoutingPolicy` — routing de agentes (sección 14.4) | BLOQUEADO | tras T1 |
| **T5** | Ejecutar `invoke.agent` desde una regla, vía broker (sección 14.5) | BLOQUEADO | T4 |
| **T6** | Artefactos `AgentRequest`/`AgentResult` + broker (sección 14.6–14.7) | BLOQUEADO | T4 |
| **T7** | Manifiesto de agente completo (sección 14.3) | BLOQUEADO | T4 |
| **T8** | Modos de aislamiento del agente (sección 14.10) | BLOQUEADO | T6 |
| **T9** | Máquina de fases Investigación→Entrega (sección 6.2 / inv 17) | BLOQUEADO | tras T1 |
| **T10** | Guarda tipada observable/attested (sección 4.8 / 6.3) | BLOQUEADO | T9 |
| **T11** | Modo gobernado / proxy (sección 12.4) | BLOQUEADO | tras T1 |
| **D1–D8** | Diferidos por diseño (ver sección Diferidos) | DIFERIDO | — |

**La próxima tarea recomendada es T1.** La spec condiciona todo el crecimiento (T4–T11) a que Phase 0c mida un delta material; empezar Phase 2 antes es prematuro.

---

## Fichas — LISTO (se pueden empezar hoy)

### T1 — Phase 0c: experimento de enforcement ⭐
- **Por qué.** Es el **punto de decisión** que la propia spec pone antes de crecer (sección 15.1). La tesis del proyecto — "tener la regla disponible no garantiza que se aplique" — solo se prueba **midiendo**. Sin este dato, construir Phase 2 es fe, no ingeniería.
- **Qué falta.** Capturar sesiones reales de un agente y comparar **violaciones-que-llegan-a-revisión-humana** con `keel gate` vs sin él, mismo modelo/cliente/reglas, contra una línea base honesta (instrucciones + skills + linters). No es código de producto: es un **harness de medición + un reporte**.
- **Contexto.** La infraestructura de medición ya existe: el ledger registra `declared` vs `effective` en cada evaluación (`crates/keel-engine/src/ledger.rs`; modos `Passive`/`Enforce` en `crates/keel-engine/src/runtime.rs`). `keel observe` (pasivo) y `keel gate` (enforce) ya alimentan el ledger. Falta el harness que corra un set de tareas en ambos modos y agregue la métrica.
- **Qué hacer.** (1) Definir un set fijo y reproducible de tareas/eventos. (2) Correrlo en modo passive y en modo enforce. (3) Contar desde el ledger las violaciones que habrían llegado a revisión humana en cada caso. (4) Emitir el delta + un criterio explícito de "seguir a Phase 2 o no".
- **Hecho cuando.** Hay un dataset reproducible y un reporte con el delta medido; la decisión de crecer queda respaldada por datos, no por intuición.

### T2 — Conteo de tokens + enforcement de `maxTokens` (inv 13)
- **Por qué.** Un agente auditor puede correr en otro modelo con coste real. Sin límites, una delegación puede desbocarse en tokens/coste. Hoy solo se aplica `timeout`.
- **Qué falta.** El executor no reporta tokens: el ledger graba `tokens: 0` (un hueco honesto, documentado). Hay que (a) capturar el conteo real del executor y (b) marcar/abortar si supera el `maxTokens` declarado en el budget del agente.
- **Contexto.** `crates/keel-engine/src/audit.rs` → `run_audit` (el budget `timeout_ms` ya se aplica; `max_tokens` se compila en `CompiledAgent` pero no se consume). El campo vive en `crates/keel-dsl` (`AgentBudget`) y en `crates/keel-engine/src/snapshot.rs` (`CompiledAgent.max_tokens`). El `tokens: 0` está en `audit.rs` (entrada del ledger).
- **Qué hacer.** Definir cómo el executor emite su conteo (p. ej. un campo en el JSON de salida), leerlo, registrarlo en el ledger (reemplazar el `0`), y enforcementar `maxTokens` (exceder → `unknown` + finding).
- **Hecho cuando.** Un executor que excede `maxTokens` produce `unknown` con un finding explícito y el ledger registra tokens reales. `maxDepth` y `cost` cruzado quedan para Phase 2 (requieren el grafo de delegación).

### T3 — Baseline de seguridad del executor (sección 13.1)
- **Por qué.** El auditor L3 lanza un subproceso (posible otro modelo/CLI). Sin restricciones explícitas hereda el entorno y las credenciales del proceso padre — superficie de riesgo innecesaria.
- **Qué falta.** Secretos por referencia (nunca en claro), allowlist de comandos del executor, y sandbox por defecto (read-only / network-deny).
- **Contexto.** `crates/keel-engine/src/audit.rs` → `run_executor` (hoy construye `Command` directo, `stderr` a null, sin sandbox ni saneo de entorno). Ya existe un patrón de "capacidades declaradas + preflight" en `crates/keel-engine/src/adapter.rs` que se puede espejar.
- **Qué hacer.** Declarar las capacidades permitidas del executor (comando, red, secretos) y aplicarlas al construir el proceso; documentar que el plano local es cooperativo (sección 5.1) — esto reduce superficie, no promete inviolabilidad.
- **Hecho cuando.** Un executor corre por defecto sin red ni secretos heredados, y pedir más capacidad es explícito y auditable.

---

## Fichas — BLOQUEADO (Phase 2; empezar solo tras T1)

> La spec pide **no** iniciar Phase 2 hasta que Phase 0c (T1) demuestre un delta material y sostenido.

### T4 — `AgentRoutingPolicy` (sección 14.4) · depende: T1
- **Por qué.** Hoy un agente apunta a UN executor fijo. Falta política de routing (qué executor/modelo según contexto) y que quede fijada en el `lock`.
- **Contexto.** No existe ningún tipo `Routing` en el árbol. `CompiledAgent.executor` es un id único (`crates/keel-engine/src/snapshot.rs`); agents/executors ya están en el snapshot+lock (inv 14 hecho).
- **Hecho cuando.** Una `AgentRoutingPolicy` declarativa resuelve el executor y queda pinneada en el snapshot/lock.

### T5 — Ejecutar `invoke.agent` desde una regla (sección 14.5) · depende: T4
- **Por qué.** Hoy el `invoke.agent` de una regla **solo se registra, nunca se ejecuta** (texto "invoke recorded (NOT executed, Phase 2)"); el único spawn real es `keel audit` manual.
- **Contexto.** `crates/keel-engine/src/runtime.rs` (`branch_detail`, registro del invoke) y `crates/keel-cli/src/gate.rs` (`audit`, el único spawn real hoy).
- **Hecho cuando.** Una regla puede disparar un agente vía broker, mediado por el runtime, con resultado validado y advisory (nunca bloquea lo irreversible, sección 4.7).

### T6 — `AgentRequest`/`AgentResult` + broker de invocación (sección 14.6–14.7) · depende: T4
- **Por qué.** Falta el flujo completo request → resolve → execute → validate → return, con provenance.
- **Contexto.** `crates/keel-engine/src/audit.rs` ya arma el prompt con el material delimitado como dato (sección 13.2) y valida el resultado contra el `outputSchema` (inv 12 hecho), pero sin artefactos formales ni broker.
- **Hecho cuando.** Existen los artefactos `AgentRequest`/`AgentResult` y un broker que resuelve Agent+Executor+snapshot y devuelve un resultado con provenance.

### T7 — Manifiesto de agente completo (sección 14.3) · depende: T4
- **Por qué.** El manifiesto actual es mínimo (role/executor/objective/outputSchema/budget); faltan capabilities, isolation y permisos declarativos.
- **Contexto.** `crates/keel-dsl` (`AgentSpec`), `schemas/agent.schema.json`, `CompiledAgent` (`crates/keel-engine/src/snapshot.rs`).
- **Hecho cuando.** El manifiesto declara capacidades/permisos y el runtime los respeta.

### T8 — Modos de aislamiento del agente (sección 14.10) · depende: T6
- **Por qué.** Un agente hijo debe correr aislado (worktree / read-only / network-deny) y con límite de profundidad de delegación.
- **Contexto.** Hoy solo hay auditor read-only + timeout (`crates/keel-engine/src/audit.rs`).
- **Hecho cuando.** Hay modos de interacción (request-response / background / handoff / auditor) y aislamiento configurable con tope de profundidad.

### T9 — Máquina de fases Investigación→Entrega (sección 6.2 / inv 17) · depende: T1
- **Por qué.** "Hecho" debe ser una transición que el runtime autoriza por artefacto, no un estado que el modelo declara. Hoy solo hay completion gate + seed de audit.
- **Contexto.** `crates/keel-cli/src/gate.rs` (completion gate, sección 12.3); no hay máquina de estados de fases. ADR-018.
- **Hecho cuando.** Las fases del ciclo son transiciones gated por artefacto, propiedad del runtime.

### T10 — Guarda tipada observable/attested (sección 4.8 / 6.3) · depende: T9
- **Por qué.** Distinguir por tipo lo observable de lo atestado en las condiciones de guarda. La clase de evidencia `Attestation` ya existe; falta el **tipo de guarda**.
- **Contexto.** `crates/keel-core/src/lib.rs` (`OriginClass::Attestation`); preconditions en `crates/keel-dsl`. Es un solo ítem junto con 6.3.
- **Hecho cuando.** Las guardas expresan observable-vs-attested de forma tipada.

### T11 — Modo gobernado / proxy (sección 12.4) · depende: T1
- **Por qué.** Hoy solo hay modo compatible (hook). Falta un modo gobernado (proxy) para clientes que lo soporten.
- **Contexto.** Adapter/hook en `crates/keel-cli/src/gate.rs`.
- **Hecho cuando.** Existe un transporte proxy además del hook, sin cambiar la semántica de reglas.

---

## Diferidos por diseño (DIFERIDO — NO es deuda)

| ID | Ítem | Motivo |
|----|---|---|
| **D1** | Monotonicidad + composición D1–D4 (sección 7.4 / inv 15 / 7.2–7.3) | Solo aplica con una **2ª capa de autoridad**; hoy hay una. Stub honesto (`crates/keel-engine/src/compile.rs::composition_stub`); el lattice ya vive en `keel-core`. ADR-014. |
| **D2** | MCP gateway (sección 14.12) | ADR-005/006 diferidos; sin MCP en el alcance actual. |
| **D3** | Packages versionados reutilizables (inv 3) | Un solo workspace por ahora. |
| **D4** | Secrets por referencia como subsistema (inv 10) | Fuera de scope actual (el mínimo entra con T3). |
| **D5** | Hot reload (sección 10.3) | Proceso efímero por decisión (ADR-010). |
| **D6** | Identidad de repo fuerte (sección 13.3) | Asunto del plano de cumplimiento. |
| **D7** | Instalador firmado / `project attach` completo (sección 9) | Historia de instalación (Phase 1+). |
| **D8** | `RCCA_future` completo (Control Plane, catálogo firmado, panel web, identidad por persona) | ADR-020 — no se inicia hasta que Phase 0 demuestre delta. |

---

## Historial de avance

Cerrado antes de este programa (esta iniciativa): CI base + lint bloqueante · docs (funcionamiento/roadmap/plan) · endurecimiento de tests + reubicación fuera de `src` · `§`→sección · lock+binding (inv 4/9) · plano CI (`keel ci`) · preflight del adapter (inv 8) · agents/executors en el lock (inv 14) · validación de `AgentResult` vs `outputSchema` (inv 12).
