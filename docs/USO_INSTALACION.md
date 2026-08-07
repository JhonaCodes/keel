# Instalacion, configuracion y uso

Keel es un runtime PADRE local: se ejecuta POR ENCIMA del CLI del modelo
(Claude Code, Codex, u otro), lo contiene en el entorno que keel fabrica y
gobierna sus acciones de forma determinista. Keel NO usa APIs de los proveedores
de modelos (ver `DECISIONES.md`, D-012).

## Instalar desde el repositorio

```bash
./install.sh
export PATH="$HOME/.local/bin:$PATH"
```

El instalador soporta macOS y Linux, compila con el lock del repositorio e
instala DOS binarios en `$HOME/.local/bin`: `keel` y `keel-shim` (el shim de
interposicion viaja junto a keel; una instalacion sin el es una instalacion
rota, fail-closed). Puede cambiarse el prefijo:

```bash
./install.sh --prefix /ruta/controlada
```

## Crear un workspace operativo

```bash
keel init ~/keel-workspace --json
keel doctor --workspace ~/keel-workspace --governed --json
```

`init` scaffoldea las capas de composicion (`global/`, `projects/<name>/`, ...),
compila el snapshot, fija el lock y abre el store. `doctor --governed` verifica
que el snapshot cargue, que el lock coincida con el snapshot publicado y que el
store abra. No hay pasos de edicion manual entre `init` y lanzar el CLI. El
workspace contiene los recursos de Keel y `.keel-state/` contiene snapshot, lock
y SQLite.

## Ejecutar un CLI gobernado

```bash
keel claude                 # o: keel codex
keel launch --client generic -- /bin/sh -c "<comando>"   # cualquier CLI
```

`keel <cli>` lanza el CLI como proceso HIJO bajo un PTY (pasa interactivo, sin
modificar) dentro del entorno que keel fabrica. Un comando gobernado (rm, git,
...) pasa por un shim → el broker de keel → `evaluate_event` en modo Enforce:
si una regla lo deniega, **el comando nunca llega a existir como proceso** (exit
2 + ContextPacket en stderr); si no, se ejecuta el binario real.

Opciones:

```text
keel launch --client <id> [opciones] -- <cmd> [args...]
  --workspace <path>       # prioridad 1 (si no: KEEL_WORKSPACE; si no: walk-up a workspace.yaml)
  --containment full|shims # full (default) = shims + sandbox del SO; shims = solo interposicion
  --no-suggest             # desactiva las sugerencias del supervisor (P3); el enforcement sigue igual
  --task "..."             # tarea inicial, pasada al CLI segun su adapter
  --session <id>           # reanudar la identidad de sesion keel
```

## El anillo duro: contencion del SO

La interposicion de PATH gobierna la superficie de PATH; una ruta absoluta
(`/bin/rm`) la evade. Para el anillo que el hijo no puede evadir, declara un
`kind: Containment` (subdir `global/containment/`) con lo que el kernel puede
imponer:

```yaml
apiVersion: keel/v1alpha1
kind: Containment
metadata: { id: global.hard.protect-docs }
spec:
  denyUnlink: ["**/*.md"]     # no se pueden borrar, ni con /bin/rm
  denyWriteOutside: true      # escrituras confinadas al workspace
  denyNetwork: false
```

El Containment entra al hash del snapshot (drift detectable por `keel lock
--verify`) y genera el perfil del sandbox del SO (macOS Seatbelt; Linux Landlock
es trabajo posterior). Si no hay provider disponible o usas `--containment
shims`, el nivel baja a shims CON BANNER — nunca en silencio.

## Convergencia: skills y agentes a traves de keel

Al lanzar, keel cablea su endpoint MCP en el hijo. El modelo descubre y carga
sus skills A TRAVES de keel (no desde su propia config):

- `keel.skills.list` — catalogo de skills gobernadas + estado de carga.
- `keel.skills.load` — entrega el contenido al contexto y registra el receipt.
- `keel.rules.query` — que reglas aplican a un comando/ruta (advisory).
- `keel.agent.invoke` — corre un agente gobernado (posiblemente en OTRO modelo,
  via un executor CLI local) y devuelve su salida validada contra su
  `outputSchema`. Transversal entre modelos, determinista, sin API.

Un `kind: ModelExecutor` declara un COMANDO local:

```yaml
apiVersion: keel/v1alpha1
kind: ModelExecutor
metadata: { id: auditor-cli, version: 1.0.0 }
spec:
  config:
    command: [codex, exec, --json]   # keel corre esto; prompt por stdin, stdout = respuesta
```

Si el hijo ignora o borra la config MCP, no se rompe nada: la convergencia no es
enforcement; los anillos duros (shims, sandbox) son independientes y siempre
activos.

## Direccion cognitiva

El supervisor observa el ledger en vivo y, ante una senal determinista de que el
modelo esta atascado (oscilacion: la misma regla bloqueando 3 veces en la
sesion), SUPERFICIE una sugerencia al operador en el transcript. NO escribe en el
stream del modelo: keel ayuda sin interferir su razonamiento. `--no-suggest` la
silencia.

## Desarrollo y verificacion

```bash
cargo test --workspace --locked
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

Los releases binarios firmados, self-update y rollback remoto siguen siendo
trabajo de distribucion; el instalador desde checkout es funcional hoy.
