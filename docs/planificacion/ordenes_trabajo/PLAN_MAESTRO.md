# Plan Maestro — Motor Keel

> Reescritura completa (2026-08-10). Reemplaza íntegramente el contenido
> anterior (vocabulario M0-M6, `ModelExecutor`/`RuntimeHost` como frontera de
> API de proveedor), que había quedado obsoleto y auto-contradictorio. La
> arquitectura vigente es el runtime padre (D-012), extendida por D-013 a
> D-016. Historia completa de la decisión en `DECISIONES.md`.

## 1. Objetivo

Keel gobierna el ENTORNO DE EJECUCIÓN del CLI del modelo (D-012) — no su API.
El trabajo restante cierra brechas de enforcement, evidencia y convergencia
sobre ese modelo; no reintroduce el camino de API de proveedor.

## 2. Baseline implementado

Resumen — el detalle completo vive en `STATUS.md`, no se duplica acá:

- **P1, doble anillo de interposición determinista** — shim de PATH sobre
  `DEFAULT_SHIM_COMMANDS` (`crates/keel-engine/src/adapter.rs`) + puente de
  hook por cliente (`keel gate`, `crates/keel-cli/src/gate.rs`) + sandbox real
  del SO (Seatbelt/Landlock, `kind: Containment`, `crates/keel-host/src/
  sandbox.rs`, `launch.rs`).
- **P2, convergencia MCP** — servidor JSON-RPC 2.0 real por stdio
  (`crates/keel-host/src/mcp.rs`): `keel.skills.list`, `keel.skills.load`,
  `keel.rules.query`, `keel.agent.invoke`.
- **P3, dirección cognitiva** — supervisor de oscilación, no bloqueante
  (`crates/keel-host/src/supervisor.rs`).
- **D-013, enriquecimiento de prompt** — `UserPromptSubmit` entrega contexto
  relevante sin bloquear.
- **D-014, enrutado declarativo** — `match{terms,context,autoload}` en
  skills/agentes, compilador deriva términos, router puntúa y desambigua
  (`crates/keel-engine/src/routing.rs`, `crates/keel-cli/src/gate.rs`).
- **D-015, capa Platform** — `platforms/<tech>/` como capa de composición
  real seleccionada por `.keel/project.yaml` (`LayerId::Platform`).
- **D-016, entrega oportuna** — matcher `PreToolUse` catch-all, contexto no
  bloqueante en `file.edited`/`command.requested`/`tool.requested`/
  `SessionStart` (`build_delivery_context`, `emit_delivery`), enrutado
  extendido a `components` (blueprints/knowledge/workflows).
- **Captura de evidencia RED/GREEN** (0.11.0/0.12.0) — un `Bash` de
  test-runner en `PostToolUse` se convierte en evento `test.completed` con el
  resultado real (`is_test_runner`/`test_outcome_content` en
  `crates/keel-cli/src/gate.rs`), consumido por la precondición builtin
  `evidence.recorded`.
- **Gate genérico de escritura por Bash** (`keel-workflow/global/rules/
  bash-write-guard.yaml` + `tools/bash-write-guard.py`) — cierra el bypass de
  `echo/sed -i/tee/cp/install/truncate > archivo.{rs,dart,py,ts,...}`
  evadiendo `file.edited`. Resuelto **sin cambios de motor**: el puente de
  hook (`command.requested`) ya ve TODOS los comandos Bash, no solo los del
  shim de PATH — una `Rule`+`Tool` de contenido alcanza. Ver H-008 en
  "Cerrado/superado".

## 3. Roadmap activo

Cada ítem: problema, enfoque, criterio de aceptación + test, referencias.

### H-009 [P1] Captura de resultado de Task-tool/subagente

**Problema:** `gate.rs` no tiene arm `"Task"` en el match de herramientas —
cae al genérico `tool.requested` (observe-only, sin extracción de
pass/fail). Bloquea el puerto fiel de `verify-before-close.sh` (gate de
Stop que requiere evidencia de audit+verify) y el patrón go/no-go
(code-auditor, edu-revisor).

**Enfoque:** construir el hermano de `is_test_runner`/`test_outcome_content`
para la tool `Task`: parsear el resultado de un subagente completado
buscando un marcador GO/NO-GO o pass/fail, sintetizar un evento
`audit.completed`/`verification.completed` consumible por
`evidence.recorded`.

**Aceptación + test:** un `Task` completado con marcador GO/NO-GO en su
resultado sintetiza el evento correspondiente. Test en `gate_hook.rs`.

### H-010 [P1] Capa de fases TDD/SDD

**Problema:** jflow gobierna hoy con una máquina de fases real
(analysis→red→green→refactor→audit→verify→done + selección de workflow
simple/tdd/sdd) que Keel no tiene. La evidencia (H-009 resuelto) es
necesaria pero NO suficiente — "evidencia de que algo pasó" y "cómo se
procesa/secuencia" son responsabilidades distintas.

**Enfoque:** autorar un `Phase` enum NUEVO con la taxonomía real de jflow —
NO revivir `crates/keel-runtime/src/phase.rs` (taxonomía RCCA incorrecta:
`Investigation/Planning/Implementation/Verification/Audit/Resolution/
Acceptance/Delivery`, sin concepto RED/GREEN, construida para el diseño de
API ya descartado). Representar la fase como EVENTOS en el mismo
`evaluate_event`, derivada de evidencia acumulada en el ledger (RED por
`test.completed=FAILED`, GREEN por un `test.completed=passed` posterior,
AUDIT/VERIFY por H-009) — no como archivo de estado separado, para evitar el
bug de estado obsoleto que jflow mismo tuvo. Reusar
`PhaseTransitionReceipt`/`ArtifactReceipt`/`PhaseError` de `phase.rs` solo
como referencia de diseño, no como código importado. Entregar contenido por
fase extendiendo `build_delivery_context`/`emit_delivery` (D-016) — no un
canal nuevo.

**Aceptación + test:** una sesión con workflow `tdd` deriva red→green
correctamente desde `test.completed=FAILED` seguido de
`test.completed=passed`, sin persistir estado propio.

**Depende de:** H-009.

### H-011 [P1] MCPProvider real + `wire_convergence` multi-servidor

**Problema:** `MCPProvider` es un valor de enum sin campos propios en
`schemas/governed-component.schema.json` (sin `match`, sin info de
conexión) — a diferencia de `skill`/`agent`, que sí tienen `match` completo.
`wire_convergence` (`crates/keel-host/src/launch.rs`) hardcodea UN solo
servidor MCP literal (`"keel"`) en las tres ramas de inyección
(`ConfigFileFlag` Claude, `ConfigOverrideFlag` Codex, `EnvConfigVar`
OpenCode) — los tres formatos son mapas nombre→config, ya multi-servidor
por diseño.

**Enfoque:** dar a `MCPProvider` schema real (comando/args o url+auth, más
`match` para enrutado contextual D-014); generalizar `wire_convergence` de
un literal fijo a iterar sobre `Vec<McpServerSpec>` (el propio de Keel + N
`MCPProvider` resueltos). Permite integrar MCPs de terceros (Linear, GitHub)
sin instalación manual por proveedor.

**Aceptación + test:** un workspace con 2 `MCPProvider` (keel + uno externo)
inyecta ambos simultáneamente en Claude/Codex/OpenCode, enrutados por
`match`. Test en `test/tests/mcp_stdio.rs` o equivalente.

### H-012 [P2] Blueprints → Skills (seguimiento de motor)

**Problema:** `kind: Blueprint` es, igual que `MCPProvider`, un enum bare en
`governed-component.schema.json` — sin `match`, sin entrega compact/full
progresiva, sin `keel.skills.load` bajo demanda. `STATUS.md` ya confirma que
"no tienen lógica de evaluación dedicada más allá del almacenamiento
genérico en el snapshot" — cero ventaja real sobre `Skill` hoy.

**Enfoque:** la migración de contenido (375 componentes en
`platforms/{dart,flutter,rust}/blueprints/`) vive en
`keel-workflow/MIGRATION_BACKLOG.md`, no acá. Este ítem trackea el
follow-up de motor: deprecar/retirar el kind `Blueprint` del schema una vez
completada esa migración.

**Aceptación + test:** con 0 componentes `kind: Blueprint` activos en
`keel-workflow`, `governed-component.schema.json` marca `Blueprint` como
deprecated (o se retira) y la documentación lo refleja.

### H-013 [P2] Paridad Codex para el puente de hooks por contenido

**Problema:** D-012.e — `pattern-guard`/`rn-guard` (y cualquier regla
`file.edited`) disparan SOLO en Claude hoy; Codex no las ve.

**Enfoque:** extender el puente de hook por cliente a Codex con la misma
semántica de bloqueo.

**Aceptación + test:** la misma regla de contenido bloquea en una sesión
`keel codex` igual que en `keel claude`; test cross-provider.

### H-014 [P3] Aislamiento fuerte Linux (Landlock)

**Problema:** hoy Linux degrada a shims (`--containment shims`); Landlock es
allow-list por path sin globs, no puede replicar `denyUnlink: *.md`
selectivo. Documentado en `docs/CONTENCION_MULTIPLATAFORMA.md`.

**Enfoque:** `SandboxProvider` para Linux con lo que Landlock sí puede dar
(`denyWriteOutside`, `denyNetwork`); las reglas de tipo `denyUnlink`
selectivo quedan shim-only en Linux, documentado explícitamente como límite,
no como bug.

**Aceptación + test:** `SandboxProvider` de Linux bloquea un acceso fuera de
allow-list igual que el perfil SBPL de macOS; test en CI Linux (tolerante a
runner sin soporte Landlock).

### H-006 [P3] Ledger unificado

Receipts, artefactos y transiciones viven en `runtime.sqlite`; evaluaciones
de reglas conservan el ledger anterior. Falta una vista append-only
unificada de model calls, capability decisions, delegaciones, usage y
costes.

**Aceptación + test:** `keel explain <ev_id>` resuelve model calls,
capability decisions, delegaciones, usage y costes desde UNA fuente.

### H-007 [P3] Distribución firmada / self-update / rollback

`install.sh` instala desde checkout. No existen todavía releases firmados,
self-update ni rollback remoto.

**Aceptación + test:** `keel doctor` verifica firma del binario instalado;
pipeline de release produce artefacto firmado + rollback probado.

## 4. Backlog no priorizado

Abierto, no perdido, no activo ahora mismo:

- **H-002** Contracts no son autoridad completa — el schema de artefacto
  usado por el vertical CLI sigue siendo interno; falta resolver
  `contract_id` desde workflow.
- **H-003** Scheduler incompleto — faltan límites por proyecto, depth,
  fan-out, ciclos, tokens, coste, prioridades y cancelación cascada.
- **H-004** Broker no resuelve credenciales del hijo — la selección
  automática de un driver local configurado para el hijo aún debe
  integrarse con el registro de executors del CLI.
- **H-005 (resto)** Hooks internos declarativos (`kind: Hook`) se compilan
  como componentes pero sin dispatcher/transport propio. La mitad de este
  hallazgo referida a MCP la cierra H-011; esta mitad (hooks internos)
  sigue abierta.
- **H-015** Subprocesos de `agent.invoke` sin shim+sandbox completo — hoy
  solo confinados por `cwd`/`env` (de `PRUEBAS_Y_ACEPTACION.md`, "criterios
  pendientes").

## 5. Cerrado / superado (registro de auditoría)

- **H-008** (bypass de Bash / gate genérico de escritura) → RESUELTO — ver
  detalle en Sección 2 (Baseline). El fix real terminó siendo del lado de
  contenido (`keel-workflow`), no del motor: el enfoque que este documento
  proponía originalmente (extender `DEFAULT_SHIM_COMMANDS`) no hizo falta,
  porque el puente de hook por cliente ya observa `command.requested` para
  CUALQUIER Bash, sin depender de la lista fija del shim de PATH. Alcance
  real: cubre extensiones de código fuente (`.rs/.dart/.py/.ts/...`), no
  archivos arbitrarios — suficiente para el caso que importa (proteger
  `pattern-guard`/`rn-guard`/reglas de contenido de código), no exhaustivo
  para cualquier tipo de archivo.
- **H-001** (workflow de 8 fases, `PhaseController` no conectado) →
  SUPERADO por H-010: rediseño desde cero con la taxonomía RED/GREEN real
  de jflow, no la de RCCA (`Investigation/Planning/.../Delivery`).
  `PhaseController`/`RuntimeHost` siguen sin importarse en el camino de
  producción (`grep -rln "RuntimeHost" --include="*.rs" .` solo devuelve
  sus propios tests de crate) — se mantiene así a propósito, H-010 no lo
  wirea, lo reemplaza.
- **Plan viejo "Compiled Workflow" (ítem 1 de 8)** → equivalente a H-001,
  ver arriba.
- **Plan viejo "MCPProvider" (ítem 4 de 8: transports/discovery/
  normalización/secret refs/pre-post policy/provenance)** → parcialmente
  absorbido en H-011 (schema + multi-servidor); el resto (secret refs,
  pre/post policy, provenance) reclasificado como H-005 (backlog).
- **Plan viejo "Internal Hooks" (ítem 5 de 8)** → equivalente a H-005
  (backlog).
- **Plan viejo "Complete Scheduler"/"Complete AgentBroker" (ítems 2 y 3 de
  8)** → equivalentes a H-003/H-004 (backlog).
- **Plan viejo "Unified Ledger"/"Distribution"/"Strong Isolation" (ítems 6,
  7, 8 de 8)** → llevados sin cambio de contenido como H-006, H-007, H-014.
- **H-016 (reescritura integral de `PROYECTO.md`/`ARQUITECTURA_RUNTIME.md`/
  `CONTRATOS_RUNTIME.md`)** → EJECUTADO en esta misma consolidación
  (2026-08-10). Los tres documentos fueron reescritos con la arquitectura
  real y ya no cargan un banner de corrección pendiente.

## 6. Orden sugerido

- H-009 primero — desbloquea H-010 y es la brecha activa más importante que
  queda (H-008 ya está resuelto).
- H-009 antes de H-010 — las fases audit/verify de H-010 derivan de la
  evidencia que aporta H-009.
- H-011 y H-013 son independientes entre sí y del resto.
- H-012 depende de trabajo en el repo hermano (`keel-workflow`); ver
  `MIGRATION_BACKLOG.md` Phase 2.

## 7. Ver también

- `STATUS.md` — baseline operativo detallado (no duplicado acá).
- `PRUEBAS_Y_ACEPTACION.md` — contrato de aceptación, gates obligatorios,
  evidencia F1-F5.
- `DECISIONES.md` — el ADR log completo (D-001 a D-016), por qué de cada
  decisión.
- `../../../../keel-workflow/MIGRATION_BACKLOG.md` — qué falta AUTORAR en
  el repo hermano de contenido.
- `../README.md` — reglas de autoría de este documento.
