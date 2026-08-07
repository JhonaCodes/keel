# Pruebas y aceptacion

Keel es un runtime PADRE sobre el CLI del modelo (ver `docs/DECISIONES.md`,
D-012). La aceptacion se define contra ese modelo, no contra una sesion por API.

## Prueba canonica del dueno

En un workspace con una regla global que prohibe borrar `.md` pero permite
`.txt`:

```bash
keel init ~/keel-workspace --json
# (autorar la regla global.no-delete-md + su tool)
keel compile --workspace ~/keel-workspace
keel launch --client generic --workspace ~/keel-workspace -- /bin/sh -c "rm notes.md"   # BLOQUEADO: exit 2 + packet, el archivo sobrevive
keel launch --client generic --workspace ~/keel-workspace -- /bin/sh -c "rm notes.txt"  # PERMITIDO
```

Cubierto por `test/tests/host_launch.rs`
(`a_governed_rm_is_decided_before_it_exists_as_a_process`).

## Evidencia automatizada por fase

- **F1 contencion (shims + broker):** un comando gobernado se decide ANTES de
  existir como proceso; `.md` bloqueado con packet + evidencia en el ledger,
  `.txt` permitido (`host_launch.rs`).
- **F2 anillo duro (sandbox del SO):** `/bin/rm` por ruta absoluta lo refusa el
  kernel; el modo `--containment shims` reabre el bypass CON BANNER
  (`the_os_sandbox_blocks_an_absolute_path_bypass`, macOS). Perfil SBPL
  unit-tested.
- **F3 convergencia (MCP):** un cliente MCP de juguete hace initialize →
  skills.list → skills.load por stdio y recibe el contenido gobernado; el
  receipt queda en el store (`mcp_stdio.rs`).
- **F4 agentes CLI:** `keel.agent.invoke` enruta a un executor CLI local y
  valida su salida contra el `outputSchema` (invariante 12); el camino API HTTP
  ya no existe (`cargo tree | grep reqwest` vacio; `governed_cli.rs`).
- **F5 direccion cognitiva:** una oscilacion (misma regla 3x/sesion) superficie
  `[keel] suggestion` al operador; `--no-suggest` la silencia; NUNCA se escribe
  en el stream del modelo (`host_launch.rs`).
- Hash, lock, composicion monotona, tools deterministas y RuleTests existentes.

## Gates por cambio

```text
cargo fmt --all -- --check
cargo test --workspace --locked      # incluye el job macOS para el sandbox
cargo clippy --workspace --all-targets -- -D warnings
git diff --check
```

## Criterios pendientes para version estable

- Sandbox Linux (Landlock) tras el trait `SandboxProvider` (hoy Linux degrada a
  shims, dicho explicitamente).
- Envolver los subprocesos de agente (`keel.agent.invoke`) en el shim+sandbox
  completo (hoy confinados por cwd/env).
- Reescritura integral de la spec y de PROYECTO/ARQUITECTURA/CONTRATOS a la
  arquitectura de runtime-padre (hoy con banner de correccion apuntando a
  D-012).
- Releases firmados y rollback remoto (pipeline de distribucion).

No se reintroduce el camino API de proveedor: el executor no-mock es un CLI
local.
