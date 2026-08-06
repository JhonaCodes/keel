# Keel — inventario de lo que falta (por precondición)

> Vista ejecutiva de lo pendiente, **agrupada por lo que la desbloquea** (no por
> estado). Complementa las otras docs con un corte distinto:
> [`PARCIALES.md`](PARCIALES.md) lista por estado (✅/🔨/⏭),
> [`PHASE2_INITIATIVE.md`](PHASE2_INITIATIVE.md) especifica las unidades de Phase 2,
> [`STATUS.md`](../STATUS.md) es la conformidad punto-a-punto, y
> [`PROGRAMA_DE_TRABAJO.md`](PROGRAMA_DE_TRABAJO.md) el backlog tarea-por-tarea
> (T1–T11, D1–D8). Aquí solo el "qué falta y qué lo destraba".
>
> **Regla de oro:** la spec (secciones 15.1/15.3) pone **un solo gate de
> crecimiento** — la **corrida REAL de Phase 0c** (medir violaciones-que-llegan-
> a-review con vs sin enforcement sobre sesiones reales). Casi todo (B) espera ese
> dato. El harness ya existe (`keel-measure`); falta capturar sesiones reales.
>
> Estado base (ya hecho): Phase 0a/0b ✅, Phase 1 núcleo local prácticamente
> completo (3 capas, ledger, lock+CI, `constraints` evaluado, ContextPacket con
> source+exemplar, harness hermético, `context.compacted`). Sin NUI. 107 tests.

Leyenda: **NO-EMPEZADO** · **PARCIAL** (scaffolding inerte) · **stub** (no-op documentado).

---

## (A) Desbloqueado AHORA — no depende de Phase 0c

| Ítem | Estado | Qué falta |
|---|---|---|
| **T1 — correr Phase 0c de verdad** | harness ✅, corrida ❌ | Capturar sesiones reales + baseline honesto → reporte con el delta. Es medición, no código. **Es el gate de todo (B).** |
| **T3 — seguridad del executor** (inv 13 "permissions") | slice env EN CURSO | `keel audit` hereda el env del padre. Slice finalizable ahora: env allowlist + scrub. Resto de T3 (sandbox OS network/fs, secret-ref) queda en (C). |

## (B) Detrás de la corrida REAL de Phase 0c (Phase 2 — ninguna empezada)

| Unidad | Tu # | Estado |
|---|---|---|
| Ejecución real de agentes + proveedor/modelo seleccionable + broker/routing | **#1-exec** | PARCIAL(seed): `invoke.agent` solo se registra; sin routing ni 2º proveedor |
| Scheduler paralelo + backlog SQLite por proyecto | **#6+#13** | NO-EMPEZADO |
| Guardrails de límites + reanudar | **#8** | NO-EMPEZADO |
| Activación de `capabilities` (+ al ContextPacket, G3) | — | PARCIAL: compilado, nunca consumido |
| Máquina de fases §6.2 / inv 17 (habilita #4 en vivo) | **#4** | PARCIAL: enum sin emisor |
| Selección granular de inputs (F4/F5) | — | PARCIAL: se parsea, se tira al compilar |
| Modos de aislamiento del agente §14.10 (T8) | — | PARCIAL: solo read-only+timeout |
| Guarda tipada observable/attested §4.8/6.3 (T10) | — | NO-EMPEZADO |
| Modo gobernado/proxy §12.4 (T11) | — | NO-EMPEZADO |
| maxDepth / coste cruzado de delegación (inv 13) | — | NO-EMPEZADO (necesita grafo de delegación) |
| ML en procesos específicos | **#10** | NO-EMPEZADO (investigación) |

## (C) Detrás de OTRA precondición (no Phase 0c)

| Ítem | Precondición |
|---|---|
| ~~Monotonicidad de composición D1–D4 §7.4~~ ✅ HECHO (Fase 4: `composition.rs` + capas del workspace §8.5) | — |
| MCP gateway §14.12 | ADR-005/006 diferidos |
| Sandbox OS del executor (network-deny/read-only) + secret-ref — resto de T3 | sandboxing por SO + subsistema de secretos (inv 10 / D4) |
| Packages versionados (inv 3), hot reload §10.3, identidad fuerte §13.3, instalador firmado §9 | decisiones de diseño / Phase 1+ |
| Todo `RCCA_future.md` (Control Plane, catálogo firmado, panel web, …) | Phase 0 con delta + datos de Phases 1-2 (ADR-020) |

---

## Secuencia recomendada
1. **Correr Phase 0c real (T1)** — desbloquea todo (B); es tu decisión (medición, no código).
2. **T3 — aislar el env del executor** — único código desbloqueado ahora.
3. Con delta material de 0c → Phase 2 por orden de spec: **#1-exec** → **#6+#13** → **#8** → capabilities/fases/resto.
4. Aparte, por su precondición: MCP (ADR-005/006). (Monotonicidad §7.4: HECHA en Fase 4.)

**Nada de esto está a medias fingiendo estar hecho**: cada ítem tiene estado real y precondición explícita en las docs enlazadas arriba.
