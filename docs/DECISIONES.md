# Decisiones

## D-001 Runtime soberano

Keel es propietario del loop cognitivo gobernado. El flujo canonico es:

```text
RuntimeHost -> resolver/snapshot -> ModelExecutor -> respuesta normalizada
RuntimeHost -> verifica -> capability/agent broker -> evidencia -> siguiente fase
```

Los hooks de Claude Code, configuraciones del cliente y MCP no son el plano de control.

Evidencia: `crates/keel-runtime/src/lib.rs`, `crates/keel-runtime/src/executor.rs` y la seccion 6.2 de `docs/RCCA_reference_architecture_v0_9_1.md`.

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

El instalador publicado cubre macOS y Linux. Windows no se documenta como
soportado hasta contar con almacenamiento seguro, rutas, packaging y pruebas E2E
equivalentes.

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
  plano de sandbox del SO (fase siguiente), y el preflight (invariante 8) es
  honesto al respecto: no promete un `block` que la contencion activa no puede
  honrar.

Evidencia: `crates/keel-host/**` (pty/broker/shims/launch), `crates/keel-shim`,
`crates/keel-engine/src/{packet,adapter}.rs`, `test/tests/host_launch.rs`.
