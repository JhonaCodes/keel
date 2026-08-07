> **CORRECCION (D-012, 2026-08-07).** Este documento describe, en partes, el
> diseno de "sesion propiedad de keel via API de proveedor" (RuntimeHost ->
> ModelExecutor -> API). Esa direccion fue REVERTIDA: **keel es un runtime
> PADRE que gobierna el ENTORNO DE EJECUCION del CLI del modelo y NO usa APIs de
> proveedor.** Donde este texto hable de llamar a la API del modelo, de `keel
> run` o de `keel configure executor`, esta OBSOLETO — manda `DECISIONES.md`
> (D-012 a-d) y el flujo real en `USO_INSTALACION.md`. La reescritura integral
> de este documento a la arquitectura de runtime-padre es trabajo pendiente
> registrado (no un descuido).

# Arquitectura del runtime

## Propiedad y limites

Keel gobierna el ciclo de intencion, contexto, razonamiento, plan, delegacion, accion, verificacion y entrega. El modelo produce propuestas; Keel decide que contexto se habilita, que operacion puede avanzar y que capability se ejecuta.

```text
entrada estructurada
  -> sesion + fase + snapshot inmutable
  -> resolver de componentes y contexto
  -> ModelExecutor
  -> respuesta normalizada (texto/tool call/agent request/artefacto)
  -> verification + policy + grants
  -> capability o AgentBroker
  -> ledger de evidencia
  -> transicion observable
```

La propiedad es del proceso de Keel. Un modelo que puede editar el cliente, invocar shell directamente o cambiar el snapshot queda fuera del modo gobernado.

## Vocabulario completo observado

### Recursos y capacidades

- `Skill`: conocimiento operativo compacto/full y ejemplos.
- `Knowledge`: fuente consultable con provenance.
- `Blueprint`: plantilla de trabajo, inputs, outputs, fases y requisitos.
- `Agent`: responsabilidad logica; no es un proceso ni un proveedor.
- `Workflow`: fases, transiciones y artefactos requeridos.
- `Tool`: funcion determinista local o externa.
- `MCPProvider`: capability externa normalizada; nunca autoridad de fases.
- `Hook`: trigger interno de eventos Keel.
- `Policy`/`Rule`: decision, deteccion, precondiciones, validacion y enforcement.

### Control y evidencia

- `ModelExecutor`: frontera con Claude, Codex u otro modelo.
- `ComponentRegistry`: indice de recursos declarados en el snapshot.
- `ContextResolver` y `CapabilityManager`: seleccion y grants de contexto/capabilities.
- `AgentBroker` y `AgentScheduler`: routing, aislamiento, limites, leases y cancelacion.
- `PhaseController`/guards: transiciones con condiciones observables.
- `EvidenceLedger`, `Receipt`, `Provenance` y `Attestation`: trazabilidad y limites de observabilidad.
- `Snapshot`, `Lock`, `Binding`, `Identity` y `RepositoryRegistry`: identidad y reproducibilidad de la configuracion efectiva.
- `Composition`, `Profile` y `Exception`: herencia monotona y relajaciones acotadas.
- `Scope`, `Constraint`, `Detector`, `Precondition` y `Validator`: aplicabilidad y evaluacion.

## Modelo de amenazas

La capa de prompt no es una frontera de seguridad. La seguridad practica depende de que el modelo no tenga acceso directo a filesystem, shell, Git, MCP o configuracion de proveedor, y de que cada capability pase por Keel. El plano local no protege contra un usuario con privilegios sobre el proceso anfitrion; ese limite se declara como advisory. Sandbox fuerte y CI son planos adicionales.

## Skills y contexto

Los archivos viven en `skills/` dentro del workspace Keel. El snapshot registra identidad, version y rutas; `RuntimeHost::from_snapshot` hidrata desde esa raiz. `skill.read` calcula el hash del contenido entregado y crea un receipt. `plan.submit`, `action.request`, `agent.invoke`, `phase.advance` y `delivery` se bloquean si falta un skill requerido.

El runtime puede garantizar disponibilidad, lectura protocolizada y version/hash. No puede demostrar comprension interna del modelo.

## Estado durable y fases

`RuntimeStore` conserva sesiones, component receipts, artifact receipts y phase transition receipts en SQLite. La evidencia es append-only; la fase efectiva se reconstruye desde la historia. Reabrir una sesion con otro snapshot, una secuencia de fases invalida o un guard que apunte a un artefacto no valido falla cerrado.

La secuencia base es Investigation, Planning, Implementation, Verification, Audit, Resolution, Acceptance y Delivery. El runtime valida el contenido del artefacto con JSON Schema y calcula su hash canonico antes de permitir la transicion. El siguiente paso arquitectonico es compilar workflows, contracts y schemas dentro del snapshot para que la API no reciba el schema desde el llamador.

## Integraciones

La integracion normativa es `Keel Runtime -> ModelExecutor -> proveedor`. Los CLIs interactivos, hooks y settings de proveedores no forman parte de la entrega final. MCP solo conecta una capability declarada, con permisos, policy, provenance y evidencia; no transporta el control de Keel.

## Proceso y entrada del producto

La primera implementacion usa un proceso efimero por sesion:

```text
keel run
  -> resuelve binding + lock + snapshot
  -> abre/restaura RuntimeStore
  -> construye RuntimeHost y ModelExecutor
  -> ejecuta el loop hasta delivery, cancelacion o error
  -> cierra executor y persiste evidencia
```

El proceso es soberano porque el modelo solo recibe operaciones y capabilities
mediadas, no porque permanezca como daemon. La persistencia permite reanudar; un
daemon posterior solo optimizaria latencia y sesiones concurrentes.

El CLI es la unica entrada de usuario. `init` crea y compila; `configure`
administra executors y secretos; `doctor --governed` valida snapshot, lock,
configuracion y store; `run` inicia o continua una sesion. Tarea y executor
quedan fijados en SQLite para que una reanudacion no cambie su identidad. Ningun
paso escribe configuracion de proveedores ni archivos de instrucciones.
