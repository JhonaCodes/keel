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

## Composición por capas (section 7) — el modelo

Un mismo workspace tiene **capas** (section 8.5). Lo que pongas en `global/`
aplica a **todos** los proyectos; lo que pongas en `projects/<nombre>/` aplica
**solo** a ese proyecto (también existen `organizations/`, `platforms/`,
`teams/`, `profiles/`). Una regla marcada `locked` en una capa superior **no se
puede debilitar** desde una inferior: `keel compile` compone las capas y
verifica monotonicidad (section 7.4 — cobertura/sensibilidad/decisión/carga) y
falla con la dimensión exacta y la capa culpable. Una capa inferior solo puede
**endurecer** (o **reemplazar** donde la superior diga `overridable`); la única
vía gobernada para relajar un `locked` en un área acotada es un `Exception`
(dueño en la capa que declaró el lock, con razón, scope acotado y expiración).

Un repo se **liga** a su proyecto con `keel bind` (`.keel/project.yaml`), y
`keel compile` resuelve la cadena por identidad del repo (section 7.1). Un
workspace plano (solo `rules/`) es el caso de una sola capa — igual que antes.

### De cero (paso a paso)

```sh
# 1. binario
cargo build --release            # binario en target/release/keel

# 2. workspace con capas
WS=~/keel-workspace
mkdir -p "$WS"/{global/rules,projects/webapp/rules}
printf 'apiVersion: keel/v1alpha1\nkind: Workspace\nmetadata: { id: mi-ws }\nspec: {}\n' > "$WS/workspace.yaml"

# 3a. una regla GLOBAL, locked (aplica a todo; nadie puede debilitarla)
cat > "$WS/global/rules/no-raw.yaml" <<'YAML'
apiVersion: keel/v1alpha1
kind: Rule
metadata: { id: sec.no-raw, author: yo, adrRef: adr:ADR-1, reviewAfter: P6M }
spec:
  locked: true
  on: [file.edited]
  detect: { using: builtin:text.regex, with: { pattern: "rawQuery" } }
  enforcement:
    invalid: { decision: block, report: { message: "usa el query builder" } }
    valid:   { decision: allow }
YAML

# 3b. una regla SOLO del proyecto
cat > "$WS/projects/webapp/rules/no-todo.yaml" <<'YAML'
apiVersion: keel/v1alpha1
kind: Rule
metadata: { id: webapp.no-todo, author: yo, adrRef: adr:ADR-2, reviewAfter: P6M }
spec:
  on: [file.edited]
  detect: { using: builtin:text.contains, with: { text: "TODO" } }
  enforcement: { invalid: { decision: review }, valid: { decision: allow } }
YAML

# 4. ligar el proyecto y compilar (compone global + projects/webapp)
#    OJO: el último segmento de `project:<org>/<nombre>` debe ser el dir bajo projects/
keel bind --workspace "$WS" --project project:miorg/webapp --org org:local
keel compile --workspace "$WS"        # publica el snapshot COMPUESTO en .keel-state/

# 5. (demostración) si el proyecto intenta DEBILITAR la regla global locked,
#    compile FALLA con la dimensión y la capa culpable. Para relajar de forma
#    gobernada un área acotada, se autora un Exception en la capa dueña (global):
mkdir -p "$WS/global/exceptions"
cat > "$WS/global/exceptions/waiver.yaml" <<'YAML'
apiVersion: keel/v1alpha1
kind: Exception
metadata: { id: reports-waiver }
spec:
  rule: rule:sec.no-raw
  owner: global
  reason: "El módulo de reportes migra el próximo trimestre."
  scope: { paths: { include: ["src/reports/**"] } }
  expiry: "2027-01-01"
YAML
keel compile --workspace "$WS"        # levanta el lock SOLO en src/reports/**; intacto el resto
```

Enganchar el cliente y el resto (binario, uninstall) siguen igual — abajo.

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
**Keel cablea el hook solo** — no editás settings a mano (así funciona igual en
cualquier PC; la lógica vive en keel, no en un snippet pegado):
```sh
keel adapter claude-code --check              # preflight: rechaza reglas que el cliente no puede hacer cumplir
keel adapter claude-code --install            # escribe el hook en <workspace>/.claude/settings.json
keel adapter claude-code --install --global   # …o en ~/.claude/settings.json → gobierna sesiones DESDE CUALQUIER LUGAR
```
`--install` es **merge-safe** (hace backup `.keel-bak`, preserva toda tu config y
tus otros hooks) e **idempotente** (instalar dos veces no duplica). Abrí una
sesión **nueva** del cliente para que tome efecto (los settings se leen al iniciar).
A partir de ahí, cada acción del LLM (Bash / Edit / Write / MultiEdit / Stop) pasa
por `keel gate` **antes** de ejecutarse: exit 2 = bloqueado, exit 0 = permitido.
(`--print` sigue disponible si querés ver el bloque sin escribirlo.)

### (opcional) 6. CI — plano de cumplimiento
```sh
keel ci resolve && keel ci run      # CI evalúa el MISMO lock/hash (garantía real, section 5)
```

---

## Desinstalar (manual, SIN borrar el proyecto)

De menos a más agresivo. Los pasos 1–3 dejan `workspace.yaml`, `rules/`,
`tools/`, `skills/`, `agents/`, `tests/` y `.keel/` **intactos** en el repo.

### 1. Apagar el wrap (reversible al instante, no toca el repo)
```sh
keel adapter claude-code --uninstall            # quita SOLO el hook de keel del proyecto
keel adapter claude-code --uninstall --global   # …o del ~/.claude global
```
Keel quita **solo sus bloques** (los identifica por su comando `gate --client
claude-code`) y **deja intactos** tus otros hooks y toda tu config. El LLM deja
de pasar por el gate al iniciar la próxima sesión.

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

---

## Nota de autoría (para que tu regla SÍ dispare en Claude Code)

El adapter de Claude Code mapea cada acción a un evento:
- **Bash** → `command.requested` con el campo **`command`** (NO `content`).
- **Edit/Write/MultiEdit** → `file.edited` con **`content`** (lo que se escribe).
- **Stop** → `completion.requested`.

Consecuencia práctica: una regla que gobierna **comandos** debe detectar con
`builtin:command.classify` (lee `command`), **no** con `text.contains`/`text.regex`
(esos leen `content`, que en un Bash viene vacío → la regla no dispararía). Para
**ediciones** sí usás `text.*` sobre `content`. Ejemplo de regla de comando que
funciona en vivo (bloquea `rm`):

```yaml
spec:
  reversibility: irreversible
  on: [command.requested]
  detect: { using: builtin:command.classify, with: { families: ["rm"] } }
  enforcement:
    unknown: { decision: deny-pending-approval, report: { message: "rm irreversible — requiere aprobación humana" } }
    valid:   { decision: allow }
```
(Sin `validate`, el veredicto es `unknown`; en un irreversible eso escala a
`deny-pending-approval` → el cliente no ejecuta. Ver sección 4.7.)
