# Pruebas y aceptacion

## Evidencia automatizada actual

- Black-box CLI: init, configure mock, doctor, run, resume completado y
  continuacion real desde una sesion interrumpida.
- `run` rechaza un snapshot recompilado que no coincide con el lock.
- Requirements de Workflow consumidos mediante skill receipt SQLite.
- Lecturas generales de componentes y bloqueo por consumo pendiente.
- Artefactos, transiciones, restauracion y snapshot drift fail-closed.
- Capability no concedida sin side effect, write confinado al workspace.
- Rules evaluadas antes de capabilities con side effects.
- Routing Agent -> executor mediante broker y scheduler.
- Parsing normalizado Anthropic/OpenAI para texto y tool calls.
- Hash, lock, composicion monotona, tools deterministas y CI existentes.

## Gates por cambio

```text
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets -- -D warnings
git diff --check
```

## Criterios pendientes para version estable

- Workflow y Contract controlan la maquina y schemas sin fallback interno.
- Scheduler verifica depth, fan-out, ciclos, tokens, coste y cancelacion.
- Broker aisla componentes/capabilities del hijo y registra budgets/usage.
- MCPProvider tiene contract tests de discovery/call/policy/provenance.
- Hooks internos no pueden modificar snapshot ni saltar policy.
- Ledger reconstruye model call, lectura, capability, delegacion y transicion.
- Installer prueba artefactos firmados en macOS y Linux.
- Runner fuerte aisla filesystem, procesos, red y secretos.

No se acepta reintroducir integraciones de cliente como alternativa.
