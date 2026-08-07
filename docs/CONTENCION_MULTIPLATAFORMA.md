# Contención multiplataforma

Keel gobierna en DOS niveles. Solo el segundo depende del sistema operativo.

## Nivel 1 — interposición de comandos (shims)

Un comando gobernado pasa por un shim → el broker → `evaluate_event`. Es lo que
decide `rm x.md` ANTES de que exista como proceso. Gobierna la **superficie de
PATH**; una invocación por ruta absoluta (`/bin/rm`) la evade por construcción —
por eso existe el Nivel 2.

Estado hoy: la implementación es **Unix** (shims `#!/bin/sh`, socket Unix,
`exec`). macOS y Linux la comparten; Windows nativo no (ver abajo).

## Nivel 2 — sandbox del SO (el anillo duro)

Cierra el bypass por ruta absoluta a nivel kernel. Cada SO enchufa su mecanismo
por el trait `SandboxProvider`. Regla de honestidad transversal: **sin provider
disponible, el nivel baja a shims CON BANNER — nunca se finge contención de
kernel que no existe.**

| SO | Mecanismo | `denyUnlink` por glob (ej. `*.md`) | `denyWriteOutside` | `denyNetwork` | Estado |
|---|---|---|---|---|---|
| **macOS** | Seatbelt (`sandbox-exec` + SBPL, match por **regex**) | ✅ exacto | ✅ | ✅ | Hecho |
| **Linux** | Landlock (allow-list por path + permisos) | ❌ **no** (ver nota) | ✅ | ✅ (kernel ≥6.7) | Pendiente (F2b) |
| **Windows** | — | — | — | — | No soportado (usar WSL2) |

### Nota crítica: Landlock ≠ Seatbelt

Seatbelt matchea paths por **regex**, así que expresa `denyUnlink: ["**/*.md"]`
de forma exacta. **Landlock NO tiene globs ni filtro por extensión**: es una
lista blanca de jerarquías de path con un bitmask de permisos. Con Landlock se
puede:

- **`denyWriteOutside`** — conceder escritura solo bajo el workspace; todo lo de
  afuera queda denegado por el kernel. Es su punto fuerte.
- **`denyNetwork`** — restringir connect/bind TCP (Landlock ABI 4, kernel ≥6.7).

Pero **NO** se puede "denegar el borrado de `*.md` pero permitir `*.txt`": no hay
forma de discriminar por extensión. Retirar `REMOVE_FILE` del workspace entero
bloquearía TAMBIÉN el `.txt` que la regla permite — sobre-bloqueo que
contradice la política. Por eso, en Linux, una regla `denyUnlink` selectiva por
extensión queda **enforced solo por la capa de shims** (Nivel 1, evadible por
ruta absoluta); el Nivel 2 en Linux aporta confinamiento de escritura/red, no el
glob de unlink. Esto es una diferencia REAL de capacidad entre kernels, no una
carencia de implementación, y debe decirse explícitamente (banner + este doc).

### Windows

Windows no tiene un equivalente liviano por-proceso a Seatbelt/Landlock para
denegar operaciones de archivo por patrón. Además, hoy el wrapper de keel es
Unix (sockets Unix, `exec`, termios, shims `sh`), así que **Windows nativo no
está soportado en absoluto** (D-011). Caminos:

1. **WSL2 (recomendado).** El CLI corre dentro de WSL2 → es Linux → aplica
   Landlock y el resto del runtime sin cambios. Es la respuesta para Windows hoy.
2. **AppContainer + token restringido** (lo de Edge/UWP): kernel-level pero
   basado en ACLs/capabilities, no en globs de deny — mapear `denyUnlink` a ACLs
   es con pérdida. Iniciativa futura, solo si hay demanda.
3. **Puerto nativo completo:** PTY con ConPTY, shims `.exe`/`.bat` (no hay
   `exec`), sockets. Trabajo grande, gated por demanda.

## Plan de F2b (Linux Landlock) — para ejecutar EN una máquina Linux

Registrado explícitamente porque no se puede compilar/verificar desde macOS:

1. Dep Linux-only `landlock` bajo `[target.'cfg(target_os="linux")'.dependencies]`.
2. `LinuxLandlock` implementa `SandboxProvider`; `available()` prueba que
   Landlock esté presente y ENFORCED (no solo compilado) en el kernel; si no,
   `None` → shims + banner.
3. Aplicar el ruleset en el HIJO antes de `exec` (via `pre_exec`) — requiere una
   ruta de spawn que exponga `pre_exec` (portable-pty no lo da directo: usar un
   pequeño wrapper `keel-sandbox` que aplique Landlock y luego `exec`, análogo a
   cómo macOS antepone `sandbox-exec`).
4. Traducir `Containment`: `denyWriteOutside` → grant de escritura solo bajo el
   workspace; `denyNetwork` → restricción de red si ABI ≥4; `denyUnlink` glob →
   **no** enforced por Landlock, se anota en el banner que es shim-only en Linux.
5. Verificación: job ubuntu del CI compila+corre; test tolerante a runners sin
   Landlock (aserta bloqueo O banner de degradación). El test macOS del glob de
   `.md` sigue siendo `cfg(target_os="macos")`.
