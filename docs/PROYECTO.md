# Keel — documentación completa del proyecto

## Qué es Keel

Keel es un runtime PADRE local que gobierna el ciclo cognitivo de agentes de IA envolviendo el ENTORNO DE EJECUCIÓN del CLI del modelo (D-012) — no su API. El modelo propone intenciones, planes, tool calls y delegaciones; Keel resuelve contexto, verifica requisitos, autoriza capacidades, controla fases y registra evidencia.

Keel no es un hook de Claude, un plugin de Codex ni un servidor MCP de terceros — es el proceso que envuelve a esos CLIs. Claude, Codex y otros proveedores son CLIs locales gobernadas vía PTY (`keel claude`, `keel codex`, `keel launch --client generic`); Keel intercepta sus comandos antes de que existan como proceso, no llama a la API HTTP de ningún proveedor.

## Flujo principal

```text
keel claude/codex
  -> PTY passthrough (keel-host)
  -> comando del modelo hijo
  -> shim de PATH + puente de hook por cliente (keel gate)
  -> evaluate_event contra reglas compiladas del snapshot
  -> bloqueo (exit 2, nunca llega a existir como proceso) o permiso
  -> sandbox del SO (Containment: Seatbelt/Landlock) como anillo duro adicional
  -> servidor MCP (keel-host/src/mcp.rs) entrega skills/agentes/reglas bajo demanda
  -> evidencia y receipt
  -> transición de fase (derivada de evidencia acumulada)
```

El runtime es dueño del estado. La frase del modelo "voy a leer este skill" no es una lectura. Solo `skill.read` ejecutado por Keel crea un receipt y habilita una operación posterior.

## Vocabulario del sistema

### Recursos y capacidades

- `Skill`: conocimiento operativo compacto/full y ejemplos, con `match{terms,context,autoload}` para enrutado declarativo (D-014).
- `Knowledge`: fuente consultable con provenance, versionada como cadena de hashes encadenados (`keel knowledge append/verify`).
- `Agent`: responsabilidad lógica ejecutada por un CLI local distinto (cross-model), invocada vía `keel.agent.invoke` con `outputSchema` validado.
- `Workflow`: fases, transiciones y artefactos requeridos.
- `Tool`: función determinista local o externa.
- `MCPProvider`: proveedor externo de capabilities; no gobierna fases ni policies.
- `Hook`: trigger interno de eventos Keel.

### Gobierno, estado y evidencia

- `Policy` y `Rule`: decisiones, restricciones, detección, precondiciones y enforcement.
- `ModelExecutor`: trait que ejecuta un comando LOCAL (`CliModelExecutor` para un CLI real, `MockModelExecutor` para tests) — sin frontera HTTP hacia ningún proveedor.
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

Los recursos viven dentro del workspace de Keel, en capas de composición (`global/`, `platforms/<tech>/` — D-015 —, `projects/<name>/`, `exceptions/`):

```text
keel-workspace/
├── skills/
├── knowledge/
├── agents/
├── workflows/
├── policies/
├── rules/
├── tools/
├── hooks/
├── providers/
└── executors/
```

`executors/` contiene specs de `ModelExecutor` para comandos LOCALES (p. ej. `command: [claude, -p]`, `command: [codex, exec, -]`) — no drivers de API de proveedor; ese camino fue eliminado (D-012).

No se copian skills, policies o knowledge a directorios, archivos de instrucciones ni configuracion del proveedor.

## Integración y seguridad

La integración canónica es:

```text
keel claude/codex -> PTY -> shim de PATH + broker (evaluate_event) -> sandbox Containment (Seatbelt/Landlock) -> servidor MCP (skills/agentes/reglas)
```

Keel nunca llama a la API HTTP de un proveedor de modelo. MCP solo puede aparecer como capability declarada, validada y registrada por Keel.

El anillo duro del sistema operativo (`kind: Containment`, Seatbelt en macOS ya shippeado — D-012.a) es el límite de seguridad real contra un modelo que intenta actuar fuera de lo declarado; Linux (Landlock) tiene cobertura parcial documentada en `CONTENCION_MULTIPLATAFORMA.md`. CI con el mismo lock y snapshot (`keel ci resolve|run`) es el plano complementario para verificación reproducible.

## Estado de implementación

Existe un baseline gobernado end-to-end accesible desde `keel-cli`: `init` compila y fija snapshot/lock, `bind` asocia el repo a un proyecto, `compile`/`test`/`lock` validan el workspace, `doctor --governed` lo verifica read-only, y `claude`/`codex`/`launch --client generic` lanzan el CLI hijo gobernado vía PTY. El loop consume componentes requeridos vía el servidor MCP, media capabilities, aplica rules antes del side effect (shim + hook bridge + sandbox), captura evidencia RED/GREEN desde el exit code real de tests, valida artefactos y persiste receipts/transiciones en SQLite.

Para el roadmap activo (qué falta y por qué), consultar [`USO_INSTALACION.md`](USO_INSTALACION.md) y [`planificacion/ordenes_trabajo/PLAN_MAESTRO.md`](planificacion/ordenes_trabajo/PLAN_MAESTRO.md).
