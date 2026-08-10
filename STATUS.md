# Keel - estado operativo

> Este documento describe la arquitectura vigente (D-012, `docs/DECISIONES.md`):
> Keel es un runtime padre que gobierna un CLI de modelo por PTY passthrough +
> interposición de comandos, no un consumidor de APIs HTTP de proveedor. Ese
> camino (drivers Anthropic/OpenAI, `keel run --task`, `keel configure`) fue
> removido (`c7ddcd8`); si ves referencias a él en documentación más vieja,
> están obsoletas.

## Baseline gobernado disponible

- `keel init <path>` crea un workspace ejecutable: capas de composición
  (`global/`, `projects/<name>/`, `exceptions/`, `tools/`, `tests/`),
  snapshot, lock y binding — listo para `keel compile` sin reglas por
  defecto (Keel no envía reglas propias a propósito).
- `keel doctor --governed` verifica snapshot, lock y estado del store de
  forma read-only.
- `keel claude` / `keel codex` / `keel launch --client generic -- <cmd>`
  lanzan el CLI cliente como hijo gobernado: PTY passthrough + interposición
  de comandos — un comando bloqueado nunca llega a existir como proceso.
- **P1 — interposición determinista (dos anillos):** reglas YAML compiladas
  se evalúan vía `keel_engine::runtime::evaluate_event` en dos canales — el
  shim de PATH (`keel-host/src/broker.rs`) y el puente de hook por cliente
  (`keel gate`, `keel-cli/src/gate.rs`). Preconditions builtin disponibles:
  `env.present`, `flag.present`, `skill.loaded`, `evidence.recorded` (bloquea
  una acción hasta que exista evidencia de un evento pasado en la sesión,
  p. ej. "no hay commit sin RED previo"). El shim de PATH solo cubre un
  conjunto FIJO de comandos por defecto (`DEFAULT_SHIM_COMMANDS`:
  `rm`, `unlink`, `mv`, `git`, `dd`, `shred`) — una regla `command.classify`
  sobre otra familia compila y pasa su `RuleTest` sin aviso, pero nunca
  dispara en una sesión real (ver `docs/AUTORIA.md`). El segundo anillo de
  P1 es el anillo duro del SO: `kind: Containment` se aplica de verdad —
  sandbox real (Seatbelt/Landlock) envolviendo el argv del cliente
  (`keel-host/src/sandbox.rs`, `launch.rs`), no solo declarado en schema.
  `--containment shims` permite optar por el anillo más débil
  explícitamente. Ninguno de los dos anillos de P1 depende de P2/P3.
- **P2 — convergencia MCP:** `keel-host/src/mcp.rs` implementa un servidor
  JSON-RPC 2.0 real por stdio (`keel.skills.list`, `keel.skills.load`,
  `keel.rules.query`, `keel.agent.invoke`) que el cliente lanzado consume
  para descubrir skills/agentes gobernados — no es un stub. La convergencia
  NO es enforcement: si el hijo ignora la config del MCP, P1 sigue activo.
- **P3 — dirección cognitiva (supervisor):** `keel-host/src/supervisor.rs`
  detecta oscilación (misma regla+ubicación repetida) y sugiere al operador
  sin interferir con el enforcement (`--no-suggest` la apaga).
- **Memoria versionada (`kind: Knowledge`):** `keel knowledge append|verify`
  hace crecer una cadena de hashes encadenados (Merkle-log) anclada en
  `.keel/keel.lock` como `knowledge_checkpoints` — crecer no dispara drift
  en `keel lock --verify`, pero una reescritura retroactiva sí es detectada.
- `keel test` corre los RuleTests del workspace contra un snapshot en
  staging (compuerta de compilación, sección 15.1).
- `keel lock` / `keel lock --verify` fijan y verifican la resolución
  (`.keel/keel.lock`), compartida entre entorno local y CI.
- `keel ci resolve|run` — plano de compliance: el mismo motor corriendo en
  CI sobre el lock fijado.
- `keel bind` — asocia el repo a un proyecto/workspace (`.keel/project.yaml`,
  invariante 4: el repo solo guarda binding + lock, nunca las definiciones).
- `keel explain <ev_id>` / `keel prune` / `keel observe` — trazabilidad de
  evidencia (con salida SARIF opcional), telemetría de ciclo de vida de
  reglas, y modo pasivo de evaluación sin bloqueo.
- `keel use <workspace>` — registra un workspace por defecto para
  `keel <cliente>` sin pasar `--workspace` cada vez.

## Límites pendientes / brechas conocidas

Detalle completo, criterio de aceptación y test de cada uno en
[`PLAN_MAESTRO.md`](docs/planificacion/ordenes_trabajo/PLAN_MAESTRO.md) —
esta lista es solo un puntero, no se duplica el contenido:

- `Hook` se compila pero sin transport/dispatcher propio — ver H-005 (backlog).
- `Workflow`/`Contract`/`Policy`/`AgentRoutingPolicy` se parsean y validan
  pero sin lógica de evaluación dedicada — ver H-002 (Contract, backlog).
- `RuntimeHost`/`Phase`/`PhaseController` no están en el camino de
  producción — ver H-001 (superado). La fase TDD/SDD real se deriva como
  contenido (Rules) en `keel-workflow`, no en el motor — no es una brecha
  del motor, ver `PLAN_MAESTRO.md` sección "Cerrado/superado" (H-010).
- Instalador es build-from-source, sin distribución firmada — ver H-007.
- `kind: Blueprint` fue eliminado del todo (no deprecado): 0 componentes
  activos tras la migración de `keel-workflow` a `Skill` — ver H-012.

Por estas limitaciones, el baseline es operativo y testeable de punta a
punta (ver `examples/starter-workspace/` para un workspace de arranque con
reglas reales), pero la iniciativa completa sigue abierta. La fuente de
verdad del trabajo restante es
[`docs/planificacion/ordenes_trabajo/PLAN_MAESTRO.md`](docs/planificacion/ordenes_trabajo/PLAN_MAESTRO.md).
