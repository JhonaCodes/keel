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
- **Entrega real de contenido de Skill fuera de la raíz del workspace**
  (0.18.0+, `crates/keel-engine/src/compile.rs`) — `keel.skills.load`
  entregaba un placeholder de "archivo faltante" para CUALQUIER skill
  autorada dentro de una capa (`global/`, `platforms/<tech>/`,
  `projects/<name>/`), es decir, para prácticamente todas. Ver H-017 en
  "Cerrado/superado".

- **Límite de workspace en la evaluación de reglas** (0.18.0+,
  `crates/keel-engine/src/runtime.rs`) — ninguna regla `file.edited` (con o
  sin `scope` declarado) dispara para un archivo fuera del workspace
  gobernado. Ver H-020 en "Cerrado/superado".
- **Captura de resultado de Task-tool/subagente** (0.18.0+,
  `crates/keel-cli/src/gate.rs`) — un `Task` (subagente: code-auditor,
  edu-revisor, cualquier revisor GO/NO-GO) completado se vuelve evidencia
  durable `task.completed`, consumible por `evidence.recorded` — habilita el
  patrón verify-before-close (gate de `Stop` que exige auditoría GO). Ver
  H-009 en "Cerrado/superado".
- **`MCPProvider` real + `wire_convergence` multi-servidor** (0.18.0+,
  `crates/keel-runtime/src/mcp_provider.rs` + `crates/keel-host/src/
  launch.rs`) — cualquier `kind: MCPProvider` autorado (comando stdio + env
  `${VAR}`) se cablea automáticamente, junto a la entrada propia de "keel",
  en las 3 formas de cliente (Claude/Codex/OpenCode) — integrar un MCP de
  terceros (Linear, GitHub) ya no requiere instalación manual por cliente.
  Ver H-011 en "Cerrado/superado".
- **Captura opcional de payload crudo del hook** (0.18.0+,
  `crates/keel-cli/src/gate.rs`) — `KEEL_GATE_DEBUG_RAW=<path>` (off por
  defecto) vuelca cada payload de hook recibido, verbatim, a JSONL antes de
  parsearlo — mitigación del riesgo admitido tras H-009 (la forma real del
  `tool_response` de un `Task` nunca se verificó contra una sesión viva; no
  existía forma de capturarla). Ver H-021 en "Cerrado/superado".
- **Entrega de `full` bajo demanda** (0.18.0+, `crates/keel-host/src/mcp.rs`
  + `crates/keel-engine/src/session.rs`) — `keel.skills.load` acepta
  `full: true` y entrega el contenido `full` de un skill directamente, sin
  necesitar 3 hallazgos de oscilación fingidos primero. Ver H-022 en
  "Cerrado/superado".

## 3. Roadmap activo

Cada ítem: problema, enfoque, criterio de aceptación + test, referencias.

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

- **H-022** (`full` de un skill solo alcanzable por escalamiento de
  oscilación P3, nunca bajo demanda) → RESUELTO (2026-08-10). Contexto: tras
  migrar 375 blueprints a skills en `keel-workflow`, `keel_reactive_notifier`
  quedó con un `full` real de 27,441 líneas — efectivamente inalcanzable, ya
  que el único mecanismo de escalamiento (`oscillating: bool` en
  `deliver_skills`, `crates/keel-engine/src/session.rs`) nunca se cableaba
  desde ningún camino de producción: el supervisor P3
  (`crates/keel-host/src/supervisor.rs`) solo imprime una sugerencia al
  operador, nunca llama a `deliver_skills`; el único call site real
  (`keel.skills.load`, `crates/keel-host/src/mcp.rs`) lo invocaba con
  `false` fijo. `oscillating: true` solo se ejercitaba en tests unitarios —
  mismo gap ya documentado para los 12 skills de Design/UI en
  `keel-workflow/MIGRATION_BACKLOG.md`. Investigado antes de diseñar: no
  había precedente de un parámetro "dame más" en ningún tool MCP del repo
  (`keel.rules.query`/`keel.agent.invoke` no tienen booleanos); `SessionState.
  loaded_skills` ya trackeaba el nivel entregado por sesión, así que no hizo
  falta tocar ledger/evidencia. Fix: `keel.skills.load`'s `inputSchema` ganó
  `full: boolean` (opcional, default `false` — compatible con cualquier
  caller existente que solo pasa `id`); `skills_load(id, full)` lo pasa a
  `deliver_skills`. Renombrado `oscillating: bool` → `escalate: bool` en
  `deliver_skills`/su doc (el nombre viejo presuponía que oscilación era la
  ÚNICA razón para escalar a `Full`; ahora hay una segunda razón legítima —
  pedido explícito — y la función no necesita saber POR QUÉ, solo QUE se
  pidió). Los 6 call sites de test en `tests-unit/session.rs` no necesitaron
  cambios (usan booleanos posicionales, el rename no rompe nada). Test nuevo
  end-to-end en `test/tests/mcp_stdio.rs`
  (`keel_skills_load_serves_full_content_on_explicit_request`, RED
  confirmado antes — `full` no existía en el schema — GREEN después):
  `keel.skills.load` sin `full` sigue sirviendo compact (regresión
  explícita); con `full: true` sirve el contenido `full`, etiquetado
  `(full)`. Verificado además contra el binario real instalado, con el caso
  real que motivó el pedido: `keel_reactive_notifier` en `keel-workflow`
  (recién migrado, H-012) — `keel mcp` por stdio, `keel.skills.load` sin
  `full` devuelve 11,679 caracteres `(compact)`; con `full: true` devuelve
  758,931 caracteres `(full)` con las 60 fuentes de ReactiveNotifier
  intactas. `keel-workflow` recompila/testea/lockea sin drift.
  **Explícitamente fuera de este ítem** (no se tocó): cablear el
  escalamiento automático real desde el supervisor P3 — requeriría
  señalización cross-proceso (el supervisor corre separado del servidor
  MCP), un ítem de motor más grande que "hacerlo alcanzable bajo demanda",
  que era el pedido concreto. `cargo test --workspace --locked`,
  `clippy -D warnings`, `fmt --check` verdes.
- **H-011** (`MCPProvider` sin lógica de conexión, `wire_convergence`
  hardcodeado a un solo servidor) → RESUELTO (2026-08-10). Motivación real:
  el usuario quiere autorar N MCPs de terceros (Linear, GitHub) UNA vez, con
  las keys en `.env`, sin instalarlos a mano en cada cliente. Investigado
  antes de diseñar: `wire_convergence` (`crates/keel-host/src/launch.rs`)
  construía la entrada `"keel"` inline y duplicada en las 3 ramas de
  cliente; `MCPProvider` ya recibía el bloque genérico `spec.config`/
  `spec.match` al compilar (mismo shape que `ModelExecutor`) pero NADA leía
  su `config` — ni siquiera `wire_convergence` iteraba
  `snapshot.components`. El propio doc-comment de `dotenv.rs` ya anticipaba
  el diseño literalmente ("Keel resolves `${VAR}` in `ModelExecutor.config.
  env` (and, once dispatched, MCP provider configs)"). Fix: refactor puro
  de `executor_env` (`crates/keel-runtime/src/executor.rs`) extrayendo la
  resolución `${VAR}` a `resolve_env_map` (sin cambio de comportamiento,
  tests preexistentes intactos); nuevo módulo `crates/keel-runtime/src/
  mcp_provider.rs` con `compiled_mcp_providers(components)` que resuelve
  cada componente `kind: mcp-provider` en un `McpServerSpec` (rechaza con
  error explícito un provider sin `config.command` o que reuse el id
  reservado `keel` — nunca desaparece en silencio); `wire_convergence`
  ahora construye `Vec<McpServerSpec>` = [entrada propia de "keel"] +
  proveedores configurados, y las 3 ramas de cliente (Claude `mcp.json`,
  Codex flags TOML repetidos, OpenCode `OPENCODE_CONFIG_CONTENT`) iteran el
  vector en vez de escribir un literal fijo. `wire_convergence` pasó a 8
  parámetros posicionales (clippy `too_many_arguments`) — resuelto
  agrupando el contexto de solo-lectura en un struct `ConvergenceContext`,
  no suprimiendo el lint. Alcance deliberadamente angosto: solo transporte
  stdio (`command`, sin `url`/SSE/HTTP — el caso real de Linear/GitHub es
  `npx`-style); cableado SIEMPRE incondicional, sin usar el `match`
  declarativo que `MCPProvider` ya tiene disponible (D-014) — `wire_convergence`
  corre al LANZAR la sesión, antes de que exista ningún "momento"/texto
  contra el cual rutear, así que no hay nada que condicionar todavía (esto
  corrige el enfoque original de este mismo documento, que proponía
  cableado por `match`). 8 tests nuevos (RED confirmado antes — el propio
  `wire_convergence` no compilaba con la firma nueva hasta implementar —
  GREEN después): 4 en `mcp_provider.rs` (resuelve comando+env, error sin
  comando, error id reservado, ignora componentes no-provider) + 4 en
  `launch.rs` (Claude/Codex/OpenCode cablean el provider junto a "keel" con
  `${VAR}` resuelto, y un provider mal configurado falla la convergencia con
  un error que lo nombra). Verificado además contra el binario real
  instalado: un workspace con `global/providers/linear.yaml` autorado +
  `keel compile` + `keel launch --client claude` con un `claude` falso en
  PATH que vuelca el `mcp.json` generado — ambas entradas (`keel` y
  `linear`) presentes, `${VAR}` resuelto correctamente desde el entorno.
  **Nota de seguridad de la propia verificación:** el primer intento usó el
  nombre convencional `LINEAR_API_KEY` en un `.env` de prueba en `/tmp`; la
  precedencia documentada de `dotenv.rs` (un export real del shell gana
  sobre el archivo) hizo que el `LINEAR_API_KEY` REAL ya exportado en el
  shell del usuario apareciera en el output — expuesto en el transcript de
  la sesión, no un bug de este fix. Corregido re-verificando con un nombre
  de variable exclusivo de test (`KEEL_H011_VERIFY_ONLY_VAR`) sin colisión
  posible; se le recomendó al usuario rotar la key real expuesta.
  **Pendiente sin verificar:** el nombre exacto del campo de entorno por
  servidor en el formato `local` de OpenCode se asumió `"environment"` (no
  hay forma de confirmarlo desde este código base ni desde esta sesión —
  ningún doc/test previo lo fijaba); si un `MCPProvider` con `env` falla en
  una sesión `keel opencode` real, ese es el primer lugar a revisar.
  `cargo test --workspace --locked` (248 tests), `clippy -D warnings`,
  `fmt --check` verdes.
- **H-021** (sin forma de verificar contra una sesión viva la forma real de
  un payload de hook) → RESUELTO (2026-08-10). Contexto: H-009 sintetiza
  `task.completed` a partir de `tool_response` de un `Task`, pero probando
  3 formas plausibles (`string` / `{result|output|text}` /
  `{content:[{text}]}`) autoconstruidas — nunca contra un payload real de una
  sesión `keel claude` en vivo. Investigado antes de proponer nada: confirmé
  por grep exhaustivo que `gate()` (`crates/keel-cli/src/gate.rs:50-53`) lee
  `stdin` y lo pasa directo a `parse()` sin loguearlo en ningún lado; no hay
  flag `--debug`, no hay `KEEL_DEBUG`; `keel observe` es un camino distinto
  (consume `Event` JSONL ya normalizado, no el payload crudo del cliente).
  Cero mecanismo de captura existía. Fix: `KEEL_GATE_DEBUG_RAW=<path>`
  (variable de entorno, no flag CLI — el argv de `keel gate` lo genera
  `wire_convergence` al lanzar la sesión; una env var se prende/apaga sin
  re-cablear nada) — cuando está seteada, `gate()` appendea una línea JSONL
  (`{ts, client, raw}`) con el payload crudo ANTES de parsear, best-effort
  (un fallo de escritura nunca rompe al cliente, mismo criterio que la
  escritura del ledger). Off por defecto: cero costo (un `std::env::var`
  chequeado), nunca se escribe nada a disco sin pedirlo explícitamente (un
  payload de hook puede llevar contenido sensible de prompt/tool). La lógica
  de captura se extrajo a una función pura `capture_raw_for_debug(path:
  Option<&str>, client, raw)` — NO lee `std::env::var` internamente — para
  que sea testeable sin depender de estado global de entorno (evita el
  problema clásico de tests en paralelo mutando la misma variable). 2 tests
  nuevos en `gate.rs` (RED confirmado antes — la función no existía, error
  de compilación — GREEN después): sin path no toca disco; con path,
  appendea JSONL (no sobreescribe) con `raw`/`client` preservados verbatim.
  Verificado además contra el binario real instalado: sin la variable, cero
  archivo creado; con la variable, el archivo captura el payload exacto
  recibido por stdin. `cargo test --workspace --locked` (223 tests),
  `clippy -D warnings`, `fmt --check` verdes. Pendiente: el usuario todavía
  no capturó un `PostToolUse Task` real con esta herramienta para confirmar
  o corregir las 3 formas asumidas en `task_result_text` (H-009) — este fix
  solo habilita esa verificación, no la reemplaza.
- **H-010** (capa de fases TDD/SDD) → RECLASIFICADO, no es un ítem de motor
  (2026-08-10). Al redactar este documento se planteó como un `Phase` enum
  NUEVO a autorar en el motor (`crates/keel-engine`), derivado de eventos del
  ledger — eso hubiera significado meter la taxonomía RED/GREEN/AUDIT/VERIFY
  de TDD/SDD (contenido de `keel-workflow`, autoría del operador) DENTRO del
  SDK. Corregido en conversación con el usuario: el motor ya expone TODO lo
  necesario para derivar fase sin ningún cambio de código —
  `event.recorded_evidence` (poblado antes de evaluar) viaja íntegro por
  stdin a cualquier `Tool` externo (`run_external`,
  `crates/keel-engine/src/tools.rs`, `serde_json::to_vec(event)`), así que
  una regla `on: [test.completed]`/`on: [task.completed]` con precondición
  `builtin:evidence.recorded` (el mismo patrón exacto que
  `require-red-before-write.yaml` y el fixture GO/NO-GO que probó H-009)
  alcanza para expresar RED→GREEN→AUDIT→VERIFY íntegramente como Reglas
  autoradas — nada que el motor no soporte hoy. Mismo patrón de resolución
  que H-008 (bash-write-guard): lo que parecía brecha de motor resultó ser
  puro trabajo de contenido. La autoría real (Rules `record-task-result`/
  `require-go-before-close`, y cualquier regla de secuencia RED→GREEN) es
  trabajo de `keel-workflow`, no de este repo — no trackeado acá.
- **H-009** (`gate.rs` no capturaba el resultado de un `Task`/subagente) →
  RESUELTO (2026-08-10). Problema: sin arm `"Task"` en el match de
  herramientas, un subagente completado (code-auditor, edu-revisor, cualquier
  revisor GO/NO-GO) caía al genérico `tool.requested` en `PostToolUse`, que
  se ignora por completo — nada sintetizaba evidencia utilizable por
  `evidence.recorded`, bloqueando el puerto fiel del patrón
  verify-before-close (gate de `Stop` que exige auditoría). Fix: nuevo
  `EventKind::TaskCompleted` (`task.completed`, extensión de capa puente —
  no uno de los 17 eventos reservados, documentado en el propio enum) + un
  arm `"Task"` en `parse_claude_code_hook` (`crates/keel-cli/src/gate.rs`):
  en `PostToolUse`, extrae el texto final crudo del subagente
  (`task_result_text`, tolera 4 formas distintas de `tool_response` — string
  plano, `{result|output|text}`, o bloques `{content:[{text}]}` estilo
  mensaje de asistente) y lo lleva VERBATIM como `content`, sin clasificar
  pass/fail en el motor — a diferencia de `test_outcome_content` (que sí
  clasifica FAILED/passed porque el exit code de un test-runner es una señal
  de verdad real), un `Task` no tiene exit code: el veredicto GO/NO-GO es
  puramente textual y por convención de cada agente, así que clasificarlo es
  responsabilidad de una regla autorada (`builtin:text.contains`), no del
  puente (coherente con el propio comentario del módulo: "the hook is pure
  TRANSPORT — no rule logic lives in it"). En `PreToolUse`, `Task` se
  comporta igual que cualquier otra tool no mapeada (`tool.requested`,
  observe-only) — sin cambios ahí. Efecto secundario encontrado y corregido
  en el mismo fix: `task.completed` no estaba en el enum `on:` de
  `schemas/rule.schema.json`/`ruletest.schema.json` (una lista JSON Schema
  separada del enum de Rust `EventKind`, que **no** se actualiza sola) — sin
  este segundo cambio, cualquier regla `on: [task.completed]` fallaba la
  compilación con "is not one of [...]"; detectado por el propio test de
  integración (RED genuino, no anticipado) al intentar `keel compile` con la
  regla de fixture. 4 tests unitarios nuevos en `gate.rs` (RED confirmado
  antes, GREEN después): Post con marcador GO/NO-GO en distintos formatos de
  `tool_response` se vuelve `task.completed` preservando el texto; Pre sigue
  siendo `tool.requested` observe-only; sin texto extraíble no sintetiza
  nada (mismo camino que un hook no reconocido). 1 test de integración
  end-to-end nuevo en `test/tests/gate_hook.rs`
  (`a_completed_task_subagent_captured_by_keel_gates_stop_on_its_go_no_go_verdict`):
  reproduce el patrón verify-before-close completo contra el binario
  compilado — `Stop` bloqueado sin auditoría, sigue bloqueado tras un
  `Task` con NO-GO, permitido recién después de un `Task` con GO, sin
  ningún evento nativo alimentado a mano. Verificado además contra el
  binario real instalado (`~/.local/bin/keel`) con los 5 pasos del mismo
  escenario por `keel gate` real vía stdin: exit 2 → exit 0 (captura,
  feedback-only) → exit 2 (NO-GO no desbloquea) → exit 0 (captura) → exit 0
  (GO desbloquea). `cargo test --workspace --locked` (218 tests), `cargo
  clippy --workspace --all-targets -D warnings`, `cargo fmt --check` todos
  verdes. `keel test`/`keel compile`/`keel lock --verify` en `keel-workflow`
  sin drift (mismo hash `sha256:5973a65a...` antes y después — esperado:
  `keel-workflow` no autora todavía ninguna regla `on: [task.completed]`,
  así que el snapshot compilado no cambia con este fix). Pendiente:
  `keel-workflow` no tiene aún una regla `record-task-result`/
  `require-go-before-close` real (equivalente de contenido a
  `record-test-result`/`require-red-before-write`) — eso es autoría de
  contenido en el repo hermano, fuera del alcance de este ítem (H-009 era
  puramente de motor, según su propio criterio de aceptación).
- **H-020** (ninguna regla distinguía "archivo fuera del workspace
  gobernado") → RESUELTO (2026-08-10). Reportado en vivo por el usuario
  trabajando NUI-4922: `keel claude` bloqueó con `global.require-red-
  before-write` el propio mecanismo interno de Plan Mode de Claude Code
  (que escribe a `~/.claude/plans/*.md`, fuera del workspace gobernado por
  completo), exigiendo evidencia RED para un archivo que no es código de
  producción ni parte del proyecto. Causa raíz confirmada: `CompiledScope::
  matches` (`snapshot.rs`) hace matching de glob sobre el string crudo de
  `event.file`, sin normalizar contra la raíz del workspace — y en ningún
  lado del motor se comparaba `event.file` contra `workspace_root` en
  absoluto, ni siquiera para reglas SIN `scope` declarado (que matchean
  todo por defecto). Dos arreglos, alcance distinto:
  1. **Inmediato, en `keel-workflow`** — agregado `**/.claude/**` al
     `scope.paths.exclude` de `require-red-before-write.yaml`. Destraba el
     caso puntual sin esperar al release del motor.
  2. **Estructural, en el motor** — nueva función
     `file_is_outside_workspace` (`crates/keel-engine/src/runtime.rs`),
     consultada al inicio de `evaluate_rule` (paso 0, antes del propio
     `scope`): si `event.file` es una ruta ABSOLUTA que no es subpath de
     `workspace_root` (también absoluto), NINGUNA regla dispara para ese
     evento — sin importar si la regla declara `scope` o no. Deliberadamente
     léxico (sin `canonicalize`, sin I/O): `workspace_root` no se
     canonicaliza en ningún lado de este código base, así que comparar
     rutas relativas de forma confiable no es posible — en ese caso
     ambiguo (ej. `--workspace .`) la función NO excluye (conservador: más
     vale evaluar de más que dejar pasar algo real). 4 tests nuevos en
     `runtime.rs` (RED confirmado antes, GREEN después): archivo fuera del
     workspace no dispara ninguna regla; archivo absoluto DENTRO del
     workspace sigue disparando igual que antes; ruta relativa (la forma
     que usa cada test preexistente del módulo) no se ve afectada.
     Verificado además contra el binario real: `/tmp/archivo-externo.dart`
     (con `--workspace` absoluto, la forma real en que se invoca en
     producción) ya no bloquea; el mismo archivo real dentro del workspace
     sigue bloqueando exit 2, sin regresión. `cargo test --workspace
     --locked`, `clippy -D warnings`, `fmt --check` verdes.
- **H-017** (entrega de contenido de Skill rota fuera de la raíz del
  workspace) → RESUELTO (2026-08-10). Reportado en vivo por el usuario: una
  sesión `keel claude` real en `keel-workflow` no pudo cargar `keel_tdd`
  ("the skill file is missing"). Causa raíz confirmada en 3 archivos:
  `compile.rs` valida la existencia del `compact` file contra la raíz de la
  CAPA (correcto) pero guardaba en el snapshot la ruta CRUDA sin resolver
  (relativa a la capa, ej. `"skills/keel_tdd_keel.md"`); `session.rs::
  render_skill` (el handler real detrás de `keel.skills.load` vía
  `mcp.rs`) resuelve el contenido contra la raíz del WORKSPACE, no de la
  capa — nunca podían coincidir para ninguna skill fuera de la raíz plana.
  Afectaba a TODAS las skills en `global/`/`platforms/`/`projects/`, no solo
  `keel_tdd`. Distinto del `Tool` (que sí funciona: el autor escribe la ruta
  completa relativa al workspace a mano en el YAML). Fix: `compile_layered`
  ahora recibe la raíz del workspace y re-ancla `compact`/`full` a esa raíz
  al compilar (`crates/keel-engine/src/compile.rs`), sin cambiar la
  convención de autoría (`compact: skills/x_keel.md`, relativo a la capa,
  sigue siendo lo que se escribe a mano). Verificado: `cargo test --workspace
  --locked` (incluye un caso de `test/tests/mcp_stdio.rs` que tenía la
  convención de ruta mal escrita, compensando el bug — corregido también),
  `cargo clippy -D warnings`, `cargo fmt --check`, y verificación mecánica
  end-to-end contra el binario real (`keel mcp` por stdio) entregando el
  contenido real de `keel_tdd` (5548 caracteres, sin placeholder) desde
  `keel-workflow`. Pendiente que el usuario reproduzca el caso original en
  una sesión `keel claude` real (no simulado por Claude).
- **H-018** (Claude nunca dispara `PostToolUse` → captura de evidencia
  RED/GREEN imposible en sesiones `keel claude`) → RESUELTO (2026-08-10).
  Reportado en vivo por el usuario, con diagnóstico propio muy detallado
  (corrido dentro de una sesión `keel claude` real): `require-red-before-
  write` bloqueaba TODO edit de producción para siempre, incluso después de
  correr un test real en rojo (`flutter test`, exit 1). Causa raíz
  confirmada en `crates/keel-host/src/launch.rs::wire_convergence`: el
  bloque `HookMethod::SettingsFileFlag` (Claude) solo registraba
  `PreToolUse`/`UserPromptSubmit`/`SessionStart`/`Stop` en el
  `settings.json` — **`PostToolUse` nunca se registraba para Claude**
  (sí para Codex, `HookMethod::ConfigOverrideFlags`, que ya lo tenía desde
  antes). La síntesis de evidencia RED/GREEN (`is_test_runner`/
  `test_outcome_content` en `gate.rs`) SOLO puede ocurrir en `PostToolUse`
  (es el único momento en que el exit code y la salida real del test
  existen) — sin el hook registrado, `keel gate` nunca se invocaba después
  de que el test corriera, así que el mecanismo (que sí funciona, y sí
  tiene tests) jamás se ejecutaba en una sesión Claude real. `keel observe`
  sí lo registra porque es un camino manual separado (passive mode,
  telemetría) — no sustituye al hook.
  Diagnóstico correcto en el síntoma y en el mecanismo general; la causa
  raíz real terminó siendo mucho más simple que la propuesta (no hacía
  falta un comando nuevo ni cambiar `record-test-result` — ese ya clasifica
  correctamente por exit code vía `test_outcome_content`, que emite su
  propio marcador `FAILED`/`passed` en mayúscula/minúscula fijo,
  independiente de cómo imprima `flutter`/`dart`; el `text.contains
  "FAILED"` de la regla compara contra ESE marcador, no contra la salida
  cruda). Fix: agregar `PostToolUse` al bloque de hooks de Claude en
  `wire_convergence`, mismo matcher catch-all que `PreToolUse`. Test de
  regresión nuevo (`claude_convergence_writes_a_posttooluse_hook_alongside_
  pretooluse`, RED confirmado antes del fix, GREEN después) — ningún test
  existente ejercitaba antes el `settings.json` real generado para Claude,
  por eso el gap pasó desapercibido pese a que el mecanismo interno ya
  tenía tests propios. Verificado: `cargo test --workspace --locked`,
  `clippy -D warnings`, `fmt --check`, todos verdes. Pendiente que el
  usuario reproduzca el ciclo TDD completo (test rojo → edit permitido →
  test verde → edit bloqueado de nuevo).
  **Corrección post-fix (mismo día):** el matcher catch-all que habilita
  `PostToolUse` también disparaba para `Edit`/`Write`/`MultiEdit`, y el
  parser ya sintetizaba ahí un segundo evento `file.edited` — idéntico
  bit-a-bit al de `PreToolUse` (`LedgerEntry` no distingue Pre de Post).
  Cada edición real escribía DOS filas en el ledger para la misma regla.
  Investigado antes de tocar nada: ningún consumidor de corrección depende
  de la segunda fila (`evidence.recorded` lee `DISTINCT`), pero sí afecta a
  dos reales: `rule_stats()` (`keel prune` reporta el doble de evaluaciones
  de las que hubo) y, más importante, `oscillations()` — el único
  consumidor del supervisor P3 — cuyo umbral de "3 intentos" se sesga
  (~1.5-2 intentos reales alcanzan para dispararlo en una regla `file.
  edited` no bloqueante, y un intento bloqueado aporta 1 fila contra 2 de
  uno exitoso: ni siquiera comparable). Ningún comentario/test/doc
  justificaba grabar `file.edited` en Post por mérito propio — D-016
  (`DECISIONES.md:419`) ya decía que esa captura quedaba diferida "para una
  fase posterior"; es un efecto lateral del catch-all agregado para Bash,
  no un diseño. Fix: `PostToolUse` + `Edit`/`Write`/`MultiEdit` (Claude) y
  `apply_patch`/`Edit`/`Write` (Codex) devuelven `None` desde el parser —
  mismo camino que una forma de hook no reconocida — sin tocar el brazo de
  `Bash` (la captura RED/GREEN de H-018 sigue intacta). 5 tests nuevos en
  `gate.rs` (RED confirmado antes, GREEN después): Post no genera evento en
  ninguno de los dos clientes, Pre sigue bloqueando igual que antes, Bash
  test-runner en Post no se tocó. Verificado además contra el binario real
  en `keel-workflow`: un `Write` por `PostToolUse` no agrega fila al ledger
  (conteo antes/después idéntico); el mismo `Write` por `PreToolUse` sigue
  bloqueando y agrega exactamente una fila. `cargo test --workspace
  --locked`, `clippy -D warnings`, `fmt --check` verdes.
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
  SUPERADO: la taxonomía RCCA (`Investigation/Planning/.../Delivery`,
  `crates/keel-runtime/src/phase.rs`) queda descartada, no reemplazada por
  código de motor — la fase TDD/SDD real de jflow (RED/GREEN/AUDIT/VERIFY)
  se deriva íntegra como Reglas de contenido en `keel-workflow` (ver H-010
  en "Cerrado/superado"), sin ningún `Phase` enum en el SDK.
  `PhaseController`/`RuntimeHost` siguen sin importarse en el camino de
  producción (`grep -rln "RuntimeHost" --include="*.rs" .` solo devuelve
  sus propios tests de crate) — código muerto, candidato a retirar (no
  trackeado como ítem propio: bajo riesgo, se puede borrar en cualquier
  limpieza).
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

- H-013 queda como único ítem activo sin dependencias (H-010 y H-011 ya no
  aplican acá, ver "Cerrado/superado").
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
