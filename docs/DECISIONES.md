# Decisiones

> **Correccion (D-012).** Las decisiones D-001, D-005 y D-008 fueron escritas
> cuando el producto se penso como una sesion propiedad de keel que llama a las
> APIs de los proveedores (`RuntimeHost -> ModelExecutor -> API`). Esa direccion
> fue REVERTIDA: keel es un runtime PADRE que gobierna el ENTORNO DE EJECUCION
> del CLI del modelo y NO usa APIs de proveedor. Donde D-001/D-005/D-008 hablen
> de `ModelExecutor -> provider API` o de "sesion gobernada por API", manda
> D-012 (y sus sub-decisiones a-d). El camino API HTTP y sus comandos (`keel
> run`, `keel configure executor`) fueron eliminados; el executor no-mock es un
> CLI local (`CliModelExecutor`). D-002/D-003/D-004/D-006/D-007/D-009 siguen
> vigentes tal cual.

## D-001 Runtime soberano

Keel es propietario del loop cognitivo gobernado. El flujo canonico es:

```text
RuntimeHost -> resolver/snapshot -> ModelExecutor -> respuesta normalizada
RuntimeHost -> verifica -> capability/agent broker -> evidencia -> siguiente fase
```

Los hooks de Claude Code, configuraciones del cliente y MCP no son el plano de control.

Evidencia: `crates/keel-runtime/src/lib.rs`, `crates/keel-runtime/src/executor.rs` y la seccion 6.2 de `docs/RACC_reference_architecture_v0_9_1.md`.

> **Nota de estado (verificar antes de confiar en este parrafo).** `RuntimeHost`
> y su `PhaseController` (`crates/keel-runtime/src/{lib,phase}.rs`) existen y
> estan probados, pero HOY no estan conectados al camino de produccion: ni
> `keel-host` ni `keel-cli` los importan (`grep -rln "RuntimeHost"
> --include="*.rs" .` solo devuelve sus propios tests de crate y `lib.rs`). El
> enforcement real que corre en `keel gate`/el broker es
> `keel_engine::runtime::evaluate_event`, sin concepto de fases. Este D-001
> describe el diseño de `RuntimeHost`, no lo que se ejecuta hoy cuando un
> usuario corre `keel claude`.

## D-002 Recursos propiedad de Keel

Skills, knowledge, blueprints, agents, workflows, policies, rules, tools, hooks
internos, providers y executors se resuelven desde el workspace/snapshot de
Keel. Nada se copia a configuracion o archivos de instrucciones del proveedor.

## D-003 Lectura observable

Una frase del modelo no es evidencia. Solo `skill.read` ejecutado por Keel satisface un requisito. El receipt contiene sesion, fase, version, hash y motivo.

## D-004 Componentes no son toda la arquitectura

El registro conserva tambien identidad, binding, lock, composicion, excepciones, perfiles, registro de repositorios, scopes, constraints, capabilities, detectores, precondiciones, validadores, packets, fases, guards, attestation y evidence ledger. Se modelan segun su responsabilidad, no se fuerzan a ser todos `Component`.

## D-005 Estado previo no es un modo del producto

La implementacion basada en eventos de cliente fue eliminada. No existe un modo
compatible paralelo a la sesion gobernada.

## D-006 Estado cognitivo durable

Receipts, artefactos y transiciones se registran append-only en `RuntimeStore`.
Una sesion queda ligada al hash del snapshot, tarea y executor con los que
inicio; reabrirla con otro snapshot o cambiar su executor falla cerrado.

## D-007 Las fases se prueban, no se declaran

El runtime mantiene la fase efectiva y valida cada transicion contra un artefacto con JSON Schema. La fase incluida en una operacion del modelo debe coincidir con la fase real y no puede modificarla. Los schemas se moveran al snapshot cuando aterricen `Workflow` y `Contract` compilados.

## D-008 Reemplazo sin codigo deprecado

El runtime soberano reemplazo el camino de integracion de cliente y no lo
mantiene como modo legacy. La eliminacion se realizo despues de dejar verde el
E2E gobernado.

Se eliminan comandos, parser, escritura de settings, tests, scaffold y
documentacion asociados. Logica reusable solo permanece si tiene un consumidor
real dentro de `RuntimeHost`, `CapabilityManager` o el plano CI.

## D-009 Proceso efimero primero

`keel run` crea un proceso propietario de la sesion y persiste su estado. Un
daemon no es requisito de soberania: es una optimizacion de latencia/operacion y
se difiere hasta medir la necesidad. Esta decision reduce IPC, instalacion y
recuperacion durante la primera entrega sin devolver control al proveedor.

## D-010 Configuracion automatica y secretos externos

Despues del bootstrap del binario, toda configuracion se realiza mediante Keel.
Los secretos locales se guardan en Keychain de macOS o Secret Service de Linux;
CI usa referencias a variables de entorno. Los valores no entran al workspace,
snapshot, lock ni ledger.

## D-011 Plataformas iniciales

El instalador publicado cubre macOS y Linux (Unix). El anillo duro (sandbox del
SO) está implementado en macOS (Seatbelt) y pendiente en Linux (Landlock, F2b);
sin provider disponible, el nivel baja a shims CON BANNER. **Windows nativo NO
está soportado** — el wrapper es Unix (sockets Unix, `exec`, termios, shims
`sh`); el camino para Windows es **WSL2** (allí es Linux). El detalle por
plataforma, incluida la diferencia real Landlock-no-tiene-globs vs Seatbelt, y
el plan de F2b, viven en [`CONTENCION_MULTIPLATAFORMA.md`](CONTENCION_MULTIPLATAFORMA.md).

## D-012 Runtime padre sobre el entorno de ejecucion del CLI

Keel es un runtime PADRE local: `keel <cli>` (por ejemplo `keel claude`,
`keel codex`) lanza el CLI del modelo como proceso HIJO dentro de un entorno
que Keel fabrica, y evalua cada comando gobernado ANTES de que exista como
proceso (anillo interior, seccion 5.3). El punto de gobierno NO son los hooks
del cliente: esos viven en la configuracion del propio cliente y el modelo
puede editarlos — desconfigurables = no hay enforcement. El punto de gobierno
es la contencion que Keel construye alrededor del hijo:

```text
keel <cli> -> PTY passthrough (el hijo corre interactivo, sin modificar)
           -> PATH shims -> keel-shim -> broker (socket) -> evaluate_event(Enforce)
           -> allow: exec del binario real / block: exit 2 + ContextPacket
```

Consecuencias:

- El camino por API HTTP de los proveedores (drivers Anthropic/OpenAI,
  `keel run --task`, API keys) se ELIMINA: no se usan APIs de los LLM; Keel
  gobierna su entorno de ejecucion directa. (Esta decision corrige la
  direccion de D-001/D-005/D-008, escritas cuando el producto se penso como
  sesion por API; su reescritura completa acompana la restauracion de la spec.)
- Keel es el punto unico de convergencia: los modelos consultan sus skills y
  agentes A TRAVES de Keel (plano de convergencia, MCP local — fase siguiente),
  nunca desde su propia configuracion.
- La contencion por interposicion de PATH gobierna la superficie de PATH; una
  invocacion por ruta absoluta la evade por construccion. Esa es tarea del
  plano de sandbox del SO, y el preflight (invariante 8) es honesto al
  respecto: no promete un `block` que la contencion activa no puede honrar.

### D-012.a Anillo duro: sandbox del SO desde `kind: Containment`

El anillo duro es un `kind: Containment` (subdir `containment/` por capa, p.ej.
`global/containment/`) que declara SOLO lo que el kernel puede imponer: borrado
de archivos por glob (`denyUnlink`), escritura fuera del workspace
(`denyWriteOutside`), red (`denyNetwork`). Compone por UNION entre capas
(restricciones solo suman) y **entra al hash del snapshot** (`Snapshot::
with_containment`), asi que `keel lock --verify` detecta su drift — no hay
contencion "fuera del artefacto".

- macOS: perfil SBPL generado y aplicado con `sandbox-exec` (`keel-host::
  sandbox::seatbelt`). El CLI esta deprecado pero la tecnologia (Seatbelt) es
  la que usan Bazel/Nix/Chromium hoy; se PRUEBA disponibilidad antes de
  confiar en ella.
- Linux (Landlock) es un provider posterior tras el mismo trait
  `SandboxProvider`; hasta que aterrice, Linux degrada a shims.
- Regla de honestidad: la contencion NUNCA se degrada en silencio. Sin
  provider disponible, o con `--containment shims`, el nivel efectivo baja a
  shims CON BANNER, y la garantia del kernel deja de aplicar (el bypass por
  ruta absoluta vuelve a ser posible, dicho explicitamente). El sandbox impone
  exactamente lo declarado, ni mas ni menos.

Evidencia: `crates/keel-host/src/sandbox.rs`, `schemas/containment.schema.json`,
`crates/keel-engine/src/snapshot.rs` (`CompiledContainment`),
`test/tests/host_launch.rs` (`the_os_sandbox_blocks_an_absolute_path_bypass`).

### D-012.b Convergencia: los modelos consultan skills/agentes A TRAVES de keel

Keel es el punto UNICO donde convergen los modelos: en vez de que cada CLI lea
sus skills/agentes de SU propia configuracion, el hijo se los pide a KEEL por un
endpoint MCP (`keel mcp`, stdio JSON-RPC 2.0 servido a mano — cero deps) que
keel cablea al lanzar la sesion (config efimera por proceso). Tools: `keel.
skills.list`, `keel.skills.load` (entrega el contenido al contexto y registra el
receipt en el store — la evidencia es de keel, no la palabra del modelo, D-003),
`keel.rules.query` (que reglas aplican, advisory, NUNCA bloquea) y
`keel.agent.invoke` (resuelto por completo, no un stub — ver D-012.c). Cada cliente declara en el
`AdapterManifest` COMO se inyecta el MCP (claude `--mcp-config`, codex `-c
mcp_servers…`) y COMO se le anuncia al modelo que esta gobernado
(`--append-system-prompt` o linea al PTY); `generic` no asume flags (convergencia
opt-in). **La convergencia (P2) NO es enforcement**: si el hijo ignora o borra la
config, no se rompe nada — los anillos duros (shims, sandbox) son independientes y
siempre activos; P1 nunca depende de P2. `deliver_skills`/`SessionStore`
(recuperados de git) implementan la economia de contexto (compact→full por
oscilacion).

Evidencia: `crates/keel-host/src/mcp.rs`, `crates/keel-host/src/launch.rs`
(`wire_convergence`), `crates/keel-engine/src/{session,adapter}.rs`,
`test/tests/mcp_stdio.rs`.

### D-012.c Sin API de proveedor: executors y agentes son CLIs locales

Keel NUNCA habla una API de proveedor. El camino HTTP (drivers Anthropic/OpenAI,
`keel run`, `keel configure executor`, keychain, `reqwest`) fue ELIMINADO. El
unico executor no-mock es `CliModelExecutor`: keel corre un COMANDO LOCAL,
escribe el prompt por stdin y toma stdout como respuesta, confinado al workspace
(`cwd=root`, `env_clear` + solo `PATH` → un agente no hereda secretos del
entorno). Un `kind: ModelExecutor` declara `config.command` (p.ej.
`[codex, exec, --json]`); `keel init` ya no toma `--executor`.

`keel.agent.invoke` (MCP) resuelve el Agent → su executor CLI → lo corre via
`AgentScheduler` (lease) → valida la salida contra el `outputSchema` declarado
(invariante 12) ANTES de confiar en ella → devuelve. Esto habilita agentes
TRANSVERSALES entre modelos: una sesion en claude pide una auditoria que corre
en codex (u otro CLI), determinista y sin API. La evidencia es de keel.

Evidencia: `crates/keel-runtime/src/executor.rs` (`CliModelExecutor`,
`executor_command`), `crates/keel-host/src/mcp.rs` (`agent_invoke`),
`test/tests/mcp_stdio.rs` (`agent_invoke_routes_to_a_local_cli_executor…`).

### D-012.d Direccion cognitiva sin interferir: sugerir al OPERADOR

Keel es el padre que AYUDA sin interferir en el razonamiento del modelo. El
supervisor (P3) observa el ledger en vivo y, ante una senal DETERMINISTA de que
el modelo esta atascado, SUPERFICIE una sugerencia al OPERADOR en el transcript
compartido. **NO escribe en el stream de entrada del modelo**: dirigir sus
tokens directamente seria interferir su razonamiento — justo lo que el dueno
pidio no hacer. El humano ve la senal y decide; los anillos de enforcement
(shims, sandbox) no dependen de este plano.

La senal en v1 es OSCILACION (seccion 6.5): la misma regla bloqueando en la
misma ubicacion 3 veces en una sesion. Cada oscilacion se superficie UNA vez
(sin fastidiar), rate-limited. Flag `--no-suggest` la silencia; el enforcement
sigue igual.

Evidencia: `crates/keel-host/src/supervisor.rs`, `crates/keel-host/src/launch.rs`
(spawn/teardown del hilo), `test/tests/host_launch.rs`
(`the_supervisor_surfaces_an_oscillation_and_no_suggest_silences_it`).

Evidencia: `crates/keel-host/**` (pty/broker/shims/launch), `crates/keel-shim`,
`crates/keel-engine/src/{packet,adapter}.rs`, `test/tests/host_launch.rs`.

### D-012.e Puente de hook: visibilidad de las tools internas del cliente

El wrapper ve comandos (shims) y OS (sandbox), pero NO las tools internas del
cliente (el `Write`/`Edit` de Claude Code). El puente lo cierra: keel instala,
por sesion y desde afuera, un hook PreToolUse que reenvia la tool call a
`keel gate`; keel la evalua con el MISMO motor y responde por exit code (2 =
bloquear ANTES de que corra, PreToolUse es pre-accion; PostToolUse = feedback,
exit 0, sin falsa promesa). El hook es TRANSPORTE puro; la logica de regla no
vive en el.

Por que esto NO reintroduce el error viejo (hook editable = unica defensa):
- Es un COMPLEMENTO: los anillos duros (shims, sandbox) no dependen de el; si
  el cliente no soporta hooks, keel sigue gobernando comandos.
- El **sandbox lo BLINDA**: el perfil deniega SIEMPRE que el hijo escriba
  `<workspace>/.keel-state` (donde vive el settings efimero del hook, los shims,
  el snapshot y el ledger). El modelo no puede desactivar el hook porque no
  puede escribir su config. Con hook y sin `Containment` declarado, keel
  sintetiza uno vacio solo para aplicar ese blindaje.
- Con el puente, `skill.loaded` puede gatear `file.edited` → keel FUERZA un
  skill incluso ante una escritura interna del modelo.

Especifico por cliente via `AdapterManifest.hook` (claude: `--settings` con un
PreToolUse que llama a `keel gate --client claude-code`). Codex: pendiente.

Evidencia: `crates/keel-cli/src/gate.rs`, `crates/keel-host/src/launch.rs`
(wire del hook), `crates/keel-engine/src/adapter.rs` (`HookInjection`),
`crates/keel-host/src/sandbox.rs` (blindaje `.keel-state`),
`test/tests/gate_hook.rs`.
