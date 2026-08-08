# Hallazgos vigentes

## H-001 Workflow parcialmente compilado

Los documentos Workflow se validan, hashean y pueden declarar requirements y
capabilities. La maquina de ocho fases (`PhaseController`, `phase.rs`) esta
implementada y probada, pero **no corre en el camino de produccion hoy** —
ni `keel-host` ni `keel-cli` la importan (`grep -rln "RuntimeHost"
--include="*.rs" .` solo devuelve sus propios tests de crate). Solo se
ejecuta dentro de `crates/keel-runtime/tests/phase_controller.rs`. Por eso
cambiar `config.phases` no altera ningun comportamiento observable de una
sesion real, no solo "todavia no reemplaza" la maquina interna — hoy no hay
ninguna maquina de fases en el camino real que reemplazar.

## H-002 Contracts no son autoridad completa

Los Contract forman parte del snapshot, pero el schema de artefacto usado por el
vertical CLI sigue siendo interno. Debe resolverse `contract_id` desde workflow.

## H-003 Scheduler incompleto

La cola persiste estados, claims y lease. Faltan limites por proyecto, depth,
fan-out, ciclos, tokens, coste, prioridades y cancelacion cascada.

## H-004 Broker no resuelve credenciales del hijo

El broker demuestra routing Agent -> executor y valida output. La seleccion
automatica de un driver configurado localmente para el hijo aun debe integrarse
con el registro de executors del CLI.

## H-005 MCP y hooks internos sin dispatcher

Ambos kinds se compilan como componentes. MCP no se expone directamente al
modelo, pero falta implementar transports y hooks internos declarativos.

## H-006 Evidencia fragmentada

Receipts, artefactos y transiciones viven en `runtime.sqlite`; evaluaciones de
rules conservan el ledger anterior. Falta una vista append-only unificada de
model calls, capability decisions, delegaciones, usage y costes.

## H-007 Distribucion aun local

`install.sh` instala desde el checkout. No existen todavia releases firmados,
self-update ni rollback remoto.
