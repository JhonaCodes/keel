# Keel - estado operativo

## Baseline gobernado disponible

- `keel init --executor mock` crea un workspace ejecutable y transaccional a
  nivel de producto: snapshot, binding, lock, configuracion y store.
- `keel doctor --governed` verifica snapshot, lock, executor y store.
- `keel run --task` inicia y `--resume` continua una sesion propiedad de Keel;
  tarea y executor quedan fijados en el store.
- Los requisitos de componentes se compilan y se consumen con receipts.
- Las fases producen artefactos validados y persistidos antes de avanzar.
- Tool calls se despachan dentro del loop de Keel; las capabilities no
  concedidas o denegadas por rules no producen side effects.
- Claude y Codex se conectan por drivers HTTP Anthropic/OpenAI.
- `agent.invoke` resuelve agentes logicos a executors configurados y usa
  scheduler SQLite.
- El camino de intercepcion dependiente del cliente fue eliminado junto con sus
  pruebas, datasets y ejemplos.

## Limites pendientes antes de una version estable

- Los drivers reales tienen tests de serializacion/parsing, pero los smoke tests
  requieren credenciales del operador.
- El scheduler implementa concurrencia, persistencia, claim y renovacion de
  lease; faltan budgets economicos, profundidad, fan-out y cancelacion cascada.
- `MCPProvider` y hooks internos se compilan como componentes, pero aun no tienen
  transport/dispatcher de produccion.
- El workflow ejecutable inicial conserva la secuencia canonica de ocho fases;
  la definicion compilada todavia no reemplaza esa maquina interna.
- El instalador actual construye un checkout fuente; releases firmados y rollback
  remoto requieren el pipeline de distribucion.

Por estas limitaciones, el baseline es operativo y testeable, pero la iniciativa
completa M0-M6 sigue abierta. La fuente de verdad del trabajo restante es
[`docs/planificacion/ordenes_trabajo/PLAN_MAESTRO.md`](docs/planificacion/ordenes_trabajo/PLAN_MAESTRO.md).
