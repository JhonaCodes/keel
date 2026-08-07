# Keel — documentación completa del proyecto

## Qué es Keel

Keel es un runtime agnóstico para gobernar el ciclo cognitivo de agentes de IA. El modelo propone intenciones, planes, tool calls y delegaciones; Keel resuelve contexto, verifica requisitos, autoriza capacidades, controla fases y registra evidencia.

Keel no es un hook de Claude, un plugin de Codex ni un servidor MCP. Claude, Codex y otros proveedores son `ModelExecutor` intercambiables.

## Flujo principal

```text
sesión + snapshot
  -> intención y fase
  -> ComponentRegistry / ContextResolver
  -> skills, knowledge, blueprints, policies y agentes
  -> ModelExecutor
  -> respuesta normalizada
  -> verificación y autorización
  -> capability o AgentBroker
  -> evidencia y receipt
  -> transición de fase
```

El runtime es dueño del estado. La frase del modelo “voy a leer este skill” no es una lectura. Solo `skill.read` ejecutado por Keel crea un receipt y habilita una operación posterior.

## Vocabulario del sistema

### Recursos y capacidades

- `Skill`: conocimiento operativo compacto/full y ejemplos.
- `Knowledge`: fuente consultable con provenance.
- `Blueprint`: patrón de trabajo, requisitos, inputs y outputs.
- `Agent`: responsabilidad lógica que puede ser ejecutada por otro modelo.
- `Workflow`: fases, transiciones y artefactos requeridos.
- `Tool`: función determinista local o externa.
- `MCPProvider`: proveedor externo de capabilities; no gobierna fases ni policies.
- `Hook`: trigger interno de eventos Keel.

### Gobierno, estado y evidencia

- `Policy` y `Rule`: decisiones, restricciones, detección, precondiciones y enforcement.
- `ModelExecutor`: frontera normalizada hacia Claude, Codex u otro modelo.
- `ComponentRegistry`: índice abierto de recursos declarados.
- `ContextResolver` y `CapabilityManager`: selección y grants.
- `AgentBroker` y `AgentScheduler`: routing, aislamiento, límites y leases.
- `PhaseController` y guards: transiciones verificables.
- `Snapshot`, `Lock`, `Binding`, `Identity` y `RepositoryRegistry`: reproducibilidad e identidad.
- `Composition`, `Profile` y `Exception`: herencia monotónica y excepciones acotadas.
- `Scope`, `Constraint`, `Detector`, `Precondition` y `Validator`: aplicabilidad y evaluación.
- `Receipt`, `Provenance`, `Attestation` y `EvidenceLedger`: trazabilidad y límites de observabilidad.

La lista no es cerrada: los nuevos conceptos deben clasificarse por responsabilidad y registrarse en el snapshot sin copiarse a un proveedor.

## Workspace

Los recursos viven dentro del workspace de Keel:

```text
keel-workspace/
├── skills/
├── knowledge/
├── blueprints/
├── agents/
├── workflows/
├── policies/
├── rules/
├── tools/
├── hooks/
├── providers/
└── executors/
```

No se copian skills, policies, blueprints o knowledge a directorios, archivos
de instrucciones ni configuracion del proveedor.

## Integración y seguridad

La integración canónica es:

```text
Keel Runtime -> ModelExecutor -> API/SDK del proveedor -> respuesta normalizada
```

La implementacion de cliente anterior fue eliminada. MCP solo puede aparecer como capability declarada, validada y registrada por Keel.

El runtime local no protege contra un usuario que controla el proceso anfitrión. Para una garantía fuerte se necesitan sandbox del sistema operativo y/o CI con el mismo lock y snapshot.

## Estado de implementación

Existe un baseline gobernado end-to-end accesible desde `keel-cli`: `init`
compila y fija snapshot/lock, `configure` administra executors, `doctor` valida el
workspace y `run` inicia o reanuda una sesion propiedad de Keel. El loop consume
componentes requeridos, llama drivers Anthropic/OpenAI o el mock, media
capabilities, aplica rules antes del side effect, valida artefactos y persiste
receipts/transiciones en SQLite.

Antes de una version estable faltan workflows/contracts que sustituyan la
maquina de fases interna, budgets y grafos completos del scheduler, transports
MCP gobernados, hooks internos ejecutables, ledger unificado de model calls y
aislamiento fuerte del sistema operativo.

Para el estado operativo y las próximas tareas, consultar [`USO_INSTALACION.md`](USO_INSTALACION.md) y [`planificacion/ordenes_trabajo/PLAN_MAESTRO.md`](planificacion/ordenes_trabajo/PLAN_MAESTRO.md).
