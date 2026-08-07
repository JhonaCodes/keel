# Keel

Keel es un runtime agnostico que posee la sesion, el contexto, las fases, las
capabilities, la delegacion y la evidencia de agentes de IA. Claude, Codex y
otros modelos se conectan mediante `ModelExecutor`; no gobiernan el runtime.

## Instalacion local

```bash
./install.sh
export PATH="$HOME/.local/bin:$PATH"
keel --version
```

## Inicio rapido

```bash
keel init ~/keel-workspace --executor mock --json
keel doctor --workspace ~/keel-workspace --governed --json
keel run --workspace ~/keel-workspace --task "Revisar la arquitectura" --json
```

`init` crea el workspace, binding, snapshot, lock, configuracion mock y store
SQLite. No crea ni modifica configuracion de proveedores.

## Proveedores reales

```bash
keel configure executor add claude \
  --workspace ~/keel-workspace \
  --provider anthropic --model <modelo> \
  --credential-env ANTHROPIC_API_KEY

keel configure executor add codex \
  --workspace ~/keel-workspace \
  --provider openai --model <modelo> \
  --credential-env OPENAI_API_KEY

keel configure executor test claude --workspace ~/keel-workspace
keel configure executor default claude --workspace ~/keel-workspace
keel run --workspace ~/keel-workspace --task "Implementar el cambio"
```

Para almacenamiento local, use `--api-key-stdin`; Keel guarda la credencial en
Keychain de macOS o Secret Service de Linux. El valor no entra al workspace,
snapshot, lock ni logs.

## Ciclo gobernado

```text
Keel Runtime
  -> carga snapshot y requisitos de la fase
  -> entrega skills, knowledge y blueprints mediante receipts
  -> llama al ModelExecutor seleccionado
  -> despacha tool calls solo por CapabilityManager
  -> aplica rules antes del side effect
  -> valida y registra el artefacto de fase
  -> avanza o falla cerrado
```

El workspace soporta `rules`, `tools`, `skills`, `agents`, `blueprints`,
`knowledge`, `workflows`, `contracts`, `hooks`, `providers`, `policies` y
`executors`. Los componentes se validan, hashean y compilan en el snapshot.
Un modelo puede solicitar `agent.invoke`; Keel resuelve el agente logico, el
executor hijo configurado y el lease del scheduler antes de ejecutarlo.

## Desarrollo

```bash
cargo test --workspace --locked
cargo clippy --workspace --all-targets -- -D warnings
```

Documentacion: [`docs/README.md`](docs/README.md). Orden de trabajo y limites:
[`docs/planificacion/ordenes_trabajo/PLAN_MAESTRO.md`](docs/planificacion/ordenes_trabajo/PLAN_MAESTRO.md).
