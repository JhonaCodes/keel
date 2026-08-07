# Contratos del runtime

## Operaciones

```text
session.start
component.list
skill.read
knowledge.read
blueprint.read
plan.submit
action.request
agent.invoke
phase.advance
delivery
session.close
```

La autorizacion de una operacion es una funcion del estado de sesion, snapshot, fase, receipts, grants, policy y artefactos. El texto del modelo no cambia ese estado.

## `skill.read`

Solicitud minima:

```json
{
  "operation": "skill.read",
  "skill_id": "flutter.state-management",
  "variant": "compact",
  "session_id": "session-123",
  "phase": "planning",
  "reason": "blueprint requirement"
}
```

Respuesta minima:

```json
{
  "skill_id": "flutter.state-management",
  "version": "1.2.0",
  "content_hash": "sha256:...",
  "content": "...",
  "receipt_id": "receipt-01...",
  "required": true,
  "session_id": "session-123",
  "phase": "planning",
  "reason": "blueprint requirement",
  "read_at": "2026-08-07T00:00:00Z"
}
```

El receipt se registra de forma append-only en SQLite. Al reabrir la sesion, Keel restaura los componentes consumidos y rechaza la sesion si el snapshot no coincide. La fase solicitada debe coincidir con la fase real del runtime; el modelo no puede falsificarla. Una promesa textual no cuenta y produce `REQUIRED_COMPONENT_READ` al intentar avanzar.

## Fases y artefactos

```text
investigation -> planning -> implementation -> verification
              -> audit -> resolution -> acceptance -> delivery
```

Cada transicion exige un artefacto valido: Investigation Report, Solution Contract, Implementation Record, Evidence Report, Audit Report, Resolution Record y Acceptance Record. Cada tipo solo puede registrarse en su fase propietaria, por lo que una fase futura no puede precargarse. Keel valida el contenido mediante JSON Schema, calcula el hash canonico y registra tanto el artifact receipt como el transition receipt antes de cambiar el estado en memoria.

Las transiciones no pueden saltar fases. Al restaurar, Keel verifica el orden y que cada transición apunte al artefacto valido que la habilito; una historia manipulada falla cerrada.

Estado actual: los schemas son entregados a la API de validacion del runtime. Sigue pendiente resolverlos exclusivamente desde contratos compilados en el snapshot.

## ModelExecutor

El executor recibe `ModelRequest` y devuelve `ModelResponse` normalizados. Expone proveedor/modelo, completion y cancelacion. No decide skills, policies, fases, capabilities, shell, filesystem, MCP ni agentes.

El `session_id` del request debe coincidir con el `RuntimeHost`; un request cruzado se rechaza antes de alcanzar al executor.

Implementados: `MockModelExecutor`, Anthropic Messages y OpenAI Responses. Los
drivers HTTP traducen mensajes, tools, texto y tool calls al contrato normalizado.
Los CLIs interactivos no son runtime canonico ni quedan como modo alternativo.
Los smoke tests de proveedores reales requieren una credencial del operador.

## AgentScheduler

Contrato objetivo: cada tarea tiene id, sesion, proyecto, parent, profundidad, agent id, executor id, presupuesto, estado y lease. Estados: `pending`, `claimed`, `running`, `completed`, `failed`, `cancelled`. El scheduler debe limitar concurrencia y evitar claims duplicados mediante transaccion SQLite y lease recuperable.

Estado actual: la cola SQLite puede ser durable o en memoria, aplica un limite
global, hace claim transaccional, renueva leases y recupera tareas cuyo lease
expira. Todavia no modela limites por proyecto/sesion, profundidad, fan-out,
grafo, budgets, prioridades ni cancelacion cascada.

El agente hijo recibe solo contexto, skills, capabilities, credenciales y presupuesto declarados. La herencia implicita esta prohibida.

## CLI gobernado

```text
keel init <workspace> --executor mock [--json]
keel configure executor add|list|test|remove|default
keel doctor --workspace <workspace> --governed [--json]
keel run --workspace <workspace> --task <text> [--executor <id>] [--json]
keel run --workspace <workspace> --resume <session-id> [--json]
```

`init` deja snapshot, lock, configuracion mock y store validos. `run` solo acepta
executors resueltos por configuracion Keel, exige que lock y snapshot coincidan,
crea la identidad de sesion y emite estado, fase, snapshot y executor. Tarea y
executor quedan persistidos. `resume` continua desde la fase durable, rechaza
snapshot drift y no permite sustituir el executor fijado.

Codigos de salida actuales: `0` completado y `1` error, denegacion o sesion no
terminada. Codigos diferenciados para denegacion/aprobacion quedan pendientes.

## Secretos

Un executor contiene `secret-ref`, nunca el valor. Localmente la referencia
apunta a Keychain/Secret Service; en CI apunta a una variable de entorno
declarada. Resolver un secreto es una operacion interna no visible al modelo y
su valor se excluye de serializacion, logs, errors, snapshot, lock y ledger.
