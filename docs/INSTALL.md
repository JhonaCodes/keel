# Keel — instalar y desinstalar (preservando el proyecto)

> Hoy la instalación es **manual / desde fuente** (no hay instalador firmado aún
> — STATUS section 9, es la "installation story" de Phase 1). Eso es a propósito
> deseable para tu caso: lo durable (rules/tools/skills/agents + binding+lock)
> **vive en el repo del proyecto y se versiona**; desinstalar solo quita lo
> "instalado" (binario + hook + estado efímero) y **nunca toca ese contenido**.

## Qué es cada cosa (el modelo mental)

| Pieza | Dónde vive | ¿Se versiona? | ¿Sobrevive un uninstall? |
|---|---|---|---|
| **Binario `keel`** | PATH (global, `~/.cargo/bin`) | no | se quita en uninstall |
| **Workspace autorado** (`workspace.yaml`, `rules/`, `tools/`, `skills/`, `agents/`, `tests/`) | repo del proyecto | **sí** | **SÍ — intacto** |
| **Binding + lock** (`.keel/project.yaml`, `.keel/keel.lock`) | repo del proyecto | **sí** (inv 4) | **SÍ — intacto** |
| **Estado de runtime** (`.keel-state/`: snapshot, ledger, sesiones) | dentro del workspace | **no** (gitignored) | opcional borrar; se regenera con `keel compile` |
| **Hook del cliente** (bloque en `.claude/settings.json`) | config del cliente | según tu proyecto | se quita en uninstall (apaga el wrap) |

Regla clave: **el "proyecto con todos los datos" (skills/tools/rules/agents + `.keel/`) es contenido versionado del repo.** Desinstalar Keel = quitar el binario + el hook + `.keel-state/`. El repo queda igual; reinstalar es recompilar.

---

## Instalar

### 1. El binario
```sh
# desde el repo de Keel:
cargo install --path crates/keel-cli      # instala `keel` en ~/.cargo/bin
#   o, sin instalar global:
cargo build --release                     # binario en target/release/keel
keel --version
```

### 2. El workspace (donde viven tus rules/tools/skills) — vive en TU repo
```sh
keel workspace init ~/mi-proyecto/.keel-workspace   # estructura, SIN reglas default
#   crea: workspace.yaml, rules/, tools/, tests/, fixtures/ (+ .example) y un .gitignore
#   escribí tus reglas en rules/ (plantilla rules/rule.yaml.example)
```
(O si el proyecto ya tiene su workspace Keel versionado, saltá este paso.)

### 3. Compilar el snapshot
```sh
cd <workspace>
keel compile     # atómico: staging → RuleTests → publica en .keel-state/ (ignorado)
keel doctor      # chequeo read-only de que todo quedó sano
```

### 4. Binding + lock (lo que SÍ se versiona en el repo, inv 4)
```sh
keel bind        # deriva project:org/repo del remote git → .keel/project.yaml
keel lock        # fija el hash del snapshot → .keel/keel.lock
git add .keel/ workspace.yaml rules/ tools/ skills/ agents/ tests/   # versionar el proyecto
```

### 5. Enganchar el cliente (esto ACTIVA el wrap del LLM)
```sh
keel adapter claude-code --check    # preflight: rechaza reglas que el cliente no puede hacer cumplir
keel adapter claude-code --print    # imprime el bloque de hooks para .claude/settings.json
```
Pegá ese bloque en `.claude/settings.json`. A partir de ahí, cada acción del LLM
(Bash / Edit / Write / MultiEdit / Stop) pasa por `keel gate` **antes** de
ejecutarse: exit 2 = bloqueado, exit 0 = permitido.

### (opcional) 6. CI — plano de cumplimiento
```sh
keel ci resolve && keel ci run      # CI evalúa el MISMO lock/hash (garantía real, section 5)
```

---

## Desinstalar (manual, SIN borrar el proyecto)

De menos a más agresivo. Los pasos 1–3 dejan `workspace.yaml`, `rules/`,
`tools/`, `skills/`, `agents/`, `tests/` y `.keel/` **intactos** en el repo.

### 1. Apagar el wrap (reversible al instante, no toca el repo)
Quitá el bloque `"hooks"` de Keel de `.claude/settings.json`. El LLM deja de
pasar por el gate inmediatamente. Nada más cambia.

### 2. Borrar el estado efímero (opcional)
```sh
rm -rf <workspace>/.keel-state/     # snapshot + ledger + sesiones
```
Se regenera con `keel compile`. Perdés solo la **historia del ledger**; reglas y
config quedan intactas.

### 3. Quitar el binario
```sh
cargo uninstall keel-cli            # si lo instalaste con cargo install
#   o borrá target/release/keel / el symlink del PATH
```

### Lo que NUNCA borra un uninstall (tu requerimiento)
`workspace.yaml`, `rules/`, `tools/`, `skills/`, `agents/`, `tests/`,
`.keel/project.yaml`, `.keel/keel.lock` — **todo eso es contenido versionado del
repo y sobrevive.** Reinstalar = repetir Instalar pasos 1, 3 y 5 (el workspace y
el binding ya están en el repo).

> Nota: un `keel uninstall`/`project detach` automatizado es parte de la
> installation story de Phase 1 (STATUS section 9). Hoy es manual — y por diseño
> nunca toca el contenido autorado.
