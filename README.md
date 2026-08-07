# Keel

Keel es un runtime PADRE local: se ejecuta POR ENCIMA del CLI del modelo
(Claude Code, Codex, u otro), lo contiene en el entorno que fabrica y gobierna
sus acciones de forma determinista — antes de que ocurran, y de un modo que el
modelo no puede desconfigurar. Keel NO usa APIs de los proveedores de modelos
(ver `docs/DECISIONES.md`, D-012).

## Instalacion local

```bash
./install.sh
export PATH="$HOME/.local/bin:$PATH"
keel --version
```

Instala `keel` y `keel-shim` (viajan juntos).

## Inicio rapido

```bash
keel init ~/keel-workspace --json
keel doctor --workspace ~/keel-workspace --governed --json
keel claude --workspace ~/keel-workspace     # o: keel codex, o keel launch --client generic -- <cmd>
```

`init` crea el workspace, binding, snapshot, lock y store SQLite. No crea ni
modifica configuracion de proveedores: keel gobierna el ENTORNO del CLI, no
habla su API.

## Como gobierna (tres planos)

```text
keel <cli>
  -> PTY: el CLI corre interactivo, sin modificar
  -> P1 shims: comando gobernado -> broker -> evaluate_event(Enforce)
        block => exit 2 + ContextPacket (nunca llega a existir como proceso)
  -> P1 sandbox del SO: perfil generado del `kind: Containment` (anillo duro)
  -> P2 MCP: el modelo consulta/carga skills y agentes A TRAVES de keel
  -> P3 supervisor: sugiere al operador ante oscilacion (sin interferir el modelo)
```

El enforcement (P1) nunca depende de la cooperacion del modelo ni de la
convergencia (P2). Los agentes son executors CLI locales: una sesion en un
modelo puede pedir una auditoria que corre en otro, sin API.

## Workspace

Soporta `rules`, `tools`, `skills`, `agents`, `containment`, `blueprints`,
`knowledge`, `workflows`, `contracts`, `hooks`, `policies` y `executors`
(comandos CLI locales). Los componentes se validan, hashean y compilan en el
snapshot inmutable; el lock los fija (`keel lock --verify` detecta drift).

## Desarrollo

```bash
cargo test --workspace --locked
cargo clippy --workspace --all-targets -- -D warnings
```

Documentacion: [`docs/USO_INSTALACION.md`](docs/USO_INSTALACION.md) y
[`docs/DECISIONES.md`](docs/DECISIONES.md). Orden de trabajo y limites:
[`docs/planificacion/ordenes_trabajo/PLAN_MAESTRO.md`](docs/planificacion/ordenes_trabajo/PLAN_MAESTRO.md).
