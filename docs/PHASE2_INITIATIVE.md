# Keel — iniciativa Phase 2 (planificada, no empezada)

> Especificación de las **unidades enteras de Phase 2**. NINGUNA está empezada:
> la spec (sección 15.1, 15.3) condiciona todo el crecimiento a que la **corrida
> REAL de Phase 0c** muestre un delta material y sostenido. Este doc existe para
> que estén **planificadas y visibles** (no ocultas), no para implementarlas
> ahora. Índice cruzado con [`PARCIALES.md`](PARCIALES.md) (columna ⏭),
> [`ROADMAP.md`](ROADMAP.md) y [`PROGRAMA_DE_TRABAJO.md`](PROGRAMA_DE_TRABAJO.md)
> (T4–T11).

**Gate único de arranque:** correr Phase 0c sobre sesiones reales (harness ya
existe: `keel-measure`, ver `datasets/phase0c/README.md`). Hasta ese delta,
empezar cualquier unidad de abajo es prematuro.

---

## P2-1 · Ejecución real de agentes + proveedor/modelo seleccionable (#1-exec)
**Spec:** sección 14.4–14.7. **Estado:** `invoke.agent` se REGISTRA, no se ejecuta en
gate/observe (`runtime.rs`); el único spawn real es `keel audit` manual.
**Qué:** un `Agent` declara la responsabilidad; un `AgentExecutor` declara CÓMO y
DÓNDE corre — **proveedor y modelo específicos por agente** (p. ej. un agente de
traducción en `codex-5.3` o `haiku` por económico; el agente principal en Claude,
que invoca a `codex-5.3-agent`, recibe el resultado y termina al cumplir).
**Piezas:** `AgentRoutingPolicy` (selección ordenada, candidatos con `when`,
fallback on unavailable/timeout, `neverOn: [policy-denied]`, `required:
{structuredOutput, configurationIsolation: clean}`); broker que resuelve
Agent+Executor+snapshot; `AgentRequest`/`AgentResult` con provenance y usage
(inputTokens/outputTokens/costUsd). **Ya listo para apoyarse:** inv 12
(outputSchema), inv 13 (maxTokens), inv 14 (executor en el lock).

## P2-2 · Scheduler de agentes en paralelo (#6) + backlog SQL por proyecto (#13)
**Estado:** no existe. **Qué (pedido del usuario):** antes de lanzar agentes, el
runtime identifica cuántos se necesitan; si superan el **máximo configurable**:
1. agrupa los temas dependientes entre sí;
2. lanza los agentes máximos permitidos, uno como **cabeza de cada sección**;
3. una cabeza sin cupo queda como **task de alta prioridad en SQLite**;
4. ese task es el siguiente al **limpiarse por completo un árbol de agentes
   dependientes** (así no le roba cupo a un flujo agéntico en curso);
5. al **tomar** un agente ese task, **sale de la cola** (para que no lo tome otra
   sesión) → done-log **append-only** (el ledger nunca borra; "salir de la cola"
   ≠ borrar historia).
**Substrate (#13):** un `tasks` en SQLite **por proyecto** (patrón del ledger
`.keel-state/`), la cola accionable; los markdown (PARCIALES/STATUS) siguen siendo
la narrativa humana. Config: máximo de agentes en paralelo (por proyecto).
**Encastre con Keel:** son los límites de delegación (inv 13: profundidad además
de tiempo/tokens) + el grafo de delegación (maxDepth) materializados como scheduler.

## P2-3 · Guardrails de límites de uso (#8)
**Qué:** cuando se agotan límites (rate/uso), el flujo **pausa y continúa al
resetear** en vez de fallar duro; el estado de la cola (P2-2) sobrevive en SQLite,
así que retomar es reanudar la cola, no reempezar.

## P2-4 · Activación de `capabilities`
**Spec:** sección 11.3. **Estado:** compiladas y surfaced como texto "Phase 2", nunca
activan/limitan nada (`runtime.rs`). **Qué:** que el `load.capabilities` de una
regla realmente habilite/limite lo que el agente puede hacer, y que
`availableCapabilities` llegue al ContextPacket (cierra G3 de PARCIALES).

## P2-5 · Máquina de fases (sección 6.2 / inv 17)
**Estado:** los eventos de fase existen como enum; sin emisor ni gating. El
mecanismo skill-on-action (#4) YA funciona en replay (regla en evento de fase
entrega skill; ejemplo `keel-dsl/tests/corpus/rules_11_4.yaml`), pero **ningún
adapter emite eventos de fase en vivo**. **Qué:** transiciones artifact-gated
(el runtime autoriza la fase, el modelo NO la declara), y un adapter que emita
`analysis.started`/`implementation.started`/… para que #4 funcione en vivo.

## P2-6 · Selección granular de inputs (F4/F5)
**Estado:** `ToolCall.inputs` / `Invoke.inputs`/`output` se parsean y se **tiran al
compilar** (round-trip Phase 0a); el runtime entrega el evento entero. **Qué:**
compilarlos y honrar la entrega selectiva de contexto (requiere el modelo de
contexto rico de Phase 2).

## P2-7 · MCP gateway
**Spec:** sección 14.12 (ADR-005/006, diferidos). **Estado:** no existe. **Qué:** un
`kind: MCPProvider` (transporte stdio/streamable-http, auth por secret-ref,
`exposes: [{capability, tool}]`). MCP es para capacidades externas; **no** es el
mecanismo de gobernanza (la gobernanza es Keel).

## P2-8 · Monotonicidad de composición D1–D4 (sección 7.4) · ✅ HECHO
**Estado:** IMPLEMENTADO. Al existir la 2ª capa de autoridad (workspace por capas
§8.5 + resolución §7.1), `composition::compose` verifica en el compilador D1
cobertura, D2 sensibilidad, D3 consecuencia, D4 carga contra cada ancestro
`locked` — con diff exacto de la dimensión debilitada y la capa culpable
(`MonotonicityViolation`). `merge:append` y `overridable` incluidos. Ver
`crates/keel-engine/src/composition.rs`.

---

## Sección ML (nota de investigación, #10)
No todo lo "inteligente" necesita un LLM. Procesos específicos y repetibles
(clasificar la intención de un comando, detectar el entorno de una connection
string, rankear candidatos de routing) son candidatos a **ML determinista
entrenado** (un clasificador liviano, embeddings locales) antes que a una llamada
LLM: más barato, reproducible, sin tokens. Alineado con la [doctrina](DOCTRINA.md)
(herramienta en frío > IA). Es investigación a evaluar por proceso, no un build
comprometido.

---

## Qué NO cambia por esto
El plano local sigue cooperativo (no resiste un dev decidido, sección 5.1);
`locked` sigue siendo garantía solo en CI; el modelo nunca lee configuración
Keel (ADR-004); el ledger sigue append-only. Phase 2 amplía el alcance, no
relaja las garantías del núcleo.
