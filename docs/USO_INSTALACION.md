# Instalacion, configuracion y uso

## Instalar desde el repositorio

```bash
./install.sh
export PATH="$HOME/.local/bin:$PATH"
```

El instalador soporta macOS y Linux, compila con el lock del repositorio e
instala `keel` en `$HOME/.local/bin`. Puede cambiarse el prefijo:

```bash
./install.sh --prefix /ruta/controlada
```

## Crear un workspace operativo

```bash
keel init ~/keel-workspace --executor mock --json
keel doctor --workspace ~/keel-workspace --governed --json
keel run --workspace ~/keel-workspace --task "Revisar el proyecto" --json
```

No hay pasos de edicion manual entre `init` y `run`. El workspace contiene los
recursos de Keel y `.keel-state/` contiene snapshot, configuracion local y SQLite.

## Configurar Claude o Codex

CI usa referencias a entorno:

```bash
keel configure executor add claude \
  --workspace ~/keel-workspace \
  --provider anthropic --model <modelo> \
  --credential-env ANTHROPIC_API_KEY

keel configure executor add codex \
  --workspace ~/keel-workspace \
  --provider openai --model <modelo> \
  --credential-env OPENAI_API_KEY
```

Una maquina local puede entregar la clave por stdin:

```bash
printf '%s' "$ANTHROPIC_API_KEY" | keel configure executor add claude \
  --workspace ~/keel-workspace \
  --provider anthropic --model <modelo> --api-key-stdin
```

Keel guarda esa clave en Keychain de macOS o Secret Service de Linux. Solo la
referencia queda en `runtime-config.json`.

## Administrar executors

```bash
keel configure executor list --workspace ~/keel-workspace --json
keel configure executor test claude --workspace ~/keel-workspace --json
keel configure executor default claude --workspace ~/keel-workspace
keel configure executor remove codex --workspace ~/keel-workspace
```

## Reanudar

```bash
keel run --workspace ~/keel-workspace --resume <session-id> --json
```

Keel recupera la tarea, el executor y la fase guardados, y continua el loop. La
sesion falla si el snapshot ya no coincide con el hash inicial o si se intenta
reemplazar el executor fijado.

## Desarrollo y verificacion

```bash
cargo test --workspace --locked
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

Los releases binarios firmados, self-update y rollback remoto siguen siendo
trabajo de distribucion; el instalador desde checkout es funcional hoy.
