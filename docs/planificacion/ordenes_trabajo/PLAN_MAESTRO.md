> **CORRECCION (D-012, 2026-08-07).** Este documento describe, en partes, el
> diseno de "sesion propiedad de keel via API de proveedor" (RuntimeHost ->
> ModelExecutor -> API). Esa direccion fue REVERTIDA: **keel es un runtime
> PADRE que gobierna el ENTORNO DE EJECUCION del CLI del modelo y NO usa APIs de
> proveedor.** Donde este texto hable de llamar a la API del modelo, de `keel
> run` o de `keel configure executor`, esta OBSOLETO — manda `DECISIONES.md`
> (D-012 a-d) y el flujo real en `USO_INSTALACION.md`. La reescritura integral
> de este documento a la arquitectura de runtime-padre es trabajo pendiente
> registrado (no un descuido).

# Plan maestro - cierre del runtime gobernado

## Objetivo

Keel posee la sesion y el ciclo cognitivo. Los proveedores son
`ModelExecutor`; recursos, operaciones, capabilities, agentes, fases y evidencia
se resuelven dentro del runtime.

## Implementado

### M0 - vertical CLI

- Prueba black-box `init -> configure -> doctor -> run -> resume`.
- `init` publica snapshot y lock, crea configuracion mock y store SQLite.
- `run` inicia o continua una sesion gobernada sin configuracion manual; tarea,
  executor y snapshot quedan fijados para reanudacion fail-closed.

### M1 - vocabulario y snapshot

- Kinds compilados: Rule, Tool, Skill, Agent, Blueprint, Knowledge, Workflow,
  Contract, Hook, MCPProvider, ModelExecutor, AgentRoutingPolicy y Policy.
- Registry abierto, requirements por fase, contenido, provenance y hash dentro
  del snapshot/lock.
- Referencias ausentes fallan durante compilacion.

### M2 - contexto y loop

- Lectura generalizada de skills y componentes con receipts persistentes.
- Requirements pendientes bloquean plan, action, agent, transition y delivery.
- Cada fase llama al executor en un loop acotado, despacha operaciones Keel y
  reinyecta sus resultados antes de aceptar el artefacto.

### M3 - capabilities

- `CapabilityManager` concede explicitamente filesystem, shell o Git.
- Paths quedan confinados al workspace.
- Las rules se evaluan en modo enforce antes del side effect.
- Una capability ausente o denegada falla cerrado.

### M4 - agentes

- `AgentBroker` resuelve Agent -> ModelExecutor y valida output contract.
- Scheduler SQLite con estados, limite de concurrencia, claims y leases.
- `agent.invoke` esta conectado al loop, resuelve la configuracion local del
  executor hijo y devuelve `AgentResult` al padre.
- Test cross-provider logico padre Claude -> agente Codex.

### M5 - executors y configuracion

- Drivers HTTP para Anthropic Messages y OpenAI Responses.
- `configure executor add/list/test/remove/default`.
- Secretos por referencia de entorno o Keychain/Secret Service.
- Mock determinista obligatorio para CI y demostracion.

### M6 - reemplazo

- Eliminada toda la integracion dependiente del cliente, los executors por
  comando, paquetes de contexto, tests y datasets asociados.
- Scaffold sin carpetas de proveedor, con recursos gobernados y executor mock.
- Instalador funcional desde checkout para macOS/Linux.

## Trabajo restante para version estable

1. **Workflow compilado:** reemplazar la maquina fija por fases/transiciones y
   contracts resueltos completamente desde el snapshot.
2. **Scheduler completo:** maxDepth, fan-out, ciclos, tokens, coste, prioridades,
   recuperacion tras crash y cancelacion cascada.
3. **AgentBroker completo:** aislar componentes/capabilities del hijo, aplicar
   timeout/cancelacion y registrar budgets/usage reales.
4. **MCPProvider:** implementar transports, discovery, normalizacion de tools,
   secret refs, policy pre/post y provenance.
5. **Hooks internos:** dispatcher de eventos con acciones declaradas, sin poder
   modificar snapshot o saltar policy.
6. **Ledger unificado:** registrar requests/responses hasheados, capability
   decisions, delegaciones, usage, costes y cierre de sesion.
7. **Distribucion:** releases firmados macOS/Linux, checksum, self-update,
   rollback y pruebas desde artefacto empaquetado.
8. **Aislamiento fuerte:** runners sandbox por plataforma para procesos y MCP.

## Orden obligatorio

1. Workflow/contracts.
2. Ledger y usage.
3. Scheduler/broker completo.
4. MCPProvider y hooks internos.
5. Sandbox.
6. Packaging firmado.

Cada punto se implementa con prueba RED, test de integracion y prueba black-box.
No se reintroducen mecanismos de cliente ni codigo legacy.
