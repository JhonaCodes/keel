# Phase 0c — datasets de medición de enforcement

Phase 0c es el experimento que decide si el proyecto crece (spec sección 15.1):
mide las **violaciones arquitectónicas que llegan a revisión humana con y sin
enforcement activo**, sobre las mismas reglas, modelo y cliente. Es **medición,
no código**: el harness `keel-measure` orquesta el binario `keel` ya existente
(`compile` / `observe` / `gate`) sobre un dataset y agrega el resultado del
ledger en solo lectura. No agrega lógica al runtime.

## Correr el harness

```sh
# Requiere el binario keel compilado (cargo build).
cargo run -p keel-tests --bin keel-measure -- \
  --dataset datasets/phase0c/v0-synthetic \
  --out target/phase0c/v0
```

Produce `report.json` (máquina) y `report.md` (documento de decisión) en `--out`.
El brazo pasivo (`keel observe`) y el brazo enforce (`keel gate`) corren en
workspaces efímeros separados, cada uno con su propio ledger. La corrida es
idempotente (parte de estado limpio en cada ejecución).

## Anatomía de un dataset

```
<dataset>/
  manifest.yaml     # id, descripción, kind, task_count
  workspace/        # un workspace keel COMPLETO (fija "las mismas reglas")
    workspace.yaml
    rules/*.yaml
  tasks/*.jsonl     # un archivo por task = una sesión; un evento por línea
  expected.yaml     # ground-truth por task (habilita falsos positivos vs etiquetas)
```

## Métricas

- **Primaria (MEDIDA):** violaciones que llegan a revisión, pasivo vs enforce.
  El `delta` es la diferencia — las violaciones de anillo interno que enforcement
  previene antes de la acción y que por lo tanto no llegan a revisión.
- **Secundarias:** cola `unknown` (medida), oscilación (proxy), latencia por
  regla (proxy: tiempo de tool, no end-to-end), falsos positivos (vs etiquetas).
- **GAP conocido:** `tokens = 0` estructural en Phase 0 (tools deterministas). El
  conteo real de tokens es el invariante 13 (camino del executor), no este
  experimento.
- **Baseline honesto (MANUAL):** la configuración completa actual
  (instrucciones + skills + linters del proyecto) y, donde exista, la alternativa
  por lenguaje. No es derivable del ledger; el reporte reserva una sección para
  cargarla a mano antes de tratar el delta como decisivo.

## Criterio de continuación

`CONTINUE` si el delta es material y sostenido (rate ≥ umbral, default 0.10) ·
`SMALLER-PROJECT` si hay violaciones pero el delta no es material (el proyecto
viable es el subconjunto ledger + evaluación pasiva) · `INCONCLUSIVE` si el
dataset no tiene violaciones que medir.

## v0-synthetic — SINTÉTICO

`v0-synthetic` **prueba que el harness es correcto y reproducible** end-to-end
con reglas builtin-only (portable, sin tools externas ni red). Ejercita el
bloqueo de anillo interno que produce el delta, el feedback de anillo externo, la
denegación de completion, la cola `unknown` y la oscilación.

**No es la corrida real de Phase 0c.** La decisión real exige capturar
**sesiones reales de agente** en un repo real, con el mismo modelo y cliente. El
harness está hecho para que ese paso solo agregue `tasks/*.jsonl` y sus etiquetas
en `expected.yaml` — nada más (spec sección 15.1; PROGRAMA_DE_TRABAJO.md línea 45).
