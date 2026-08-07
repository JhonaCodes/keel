# Guía de autoría — cómo crear cada tipo

Referencia práctica para autorar los componentes de un workspace de keel — por
un humano o por cualquier IA. Cada `kind` va en su carpeta por convención;
`keel init` ya deja un README + un `.example` en cada una (el loader IGNORA los
`.example`: nada se activa hasta renombrarlo a `<name>.yaml`).

Sobre-escribe siempre el modelo mental: **la regla declara; la tool implementa;
la tool es código.** Un proceso decidible en frío es una tool determinista (0
tokens), no una llamada al modelo.

Flujo mínimo tras autorar cualquier cosa:

```bash
keel compile --workspace <ws>   # valida schema + corre RuleTests + publica el snapshot
keel lock --workspace <ws>      # fija el lock al snapshot (drift detectable con --verify)
```

Las capas viven en `global/` (aplica a todo), `projects/<name>/` (solo ese
proyecto) y demás (sección 8.5). Los ejemplos usan `global/`; podés autorar lo
mismo bajo `projects/app/`.

---

## Rule — la regla que gobierna una acción

Carpeta: `global/rules/<name>.yaml`. Decide sobre un EVENTO (p.ej.
`command.requested`, `file.edited`). El veredicto viene de un `validate` (una
tool), y `enforcement` mapea el veredicto a una decisión.

```yaml
apiVersion: keel/v1alpha1
kind: Rule
metadata: { id: global.no-delete-md, author: jhonacode, adrRef: adr:ADR-001, reviewAfter: P6M }
spec:
  reversibility: irreversible          # borrar no se deshace; un `unknown` escala a humano
  on: [command.requested]              # eventos que disparan la regla
  validate: { using: tool:no-delete-md }   # la tool que da el veredicto (valid/invalid/unknown)
  enforcement:
    invalid: { decision: block, report: { message: "deleting .md files is forbidden" } }
    valid:   { decision: allow }
```

- **Obligatorio en metadata:** `id`, `author`, `adrRef`, `reviewAfter` (ADR-023).
- **`on`:** uno o varios de los 18 event kinds; el anillo interior
  (`command.requested`) es pre-acción (bloqueo real).
- **`decision`:** `allow` < `review` < `block` < `deny-pending-approval`.
- **Opcionales:** `scope: { paths: { include: ["src/**"] } }` (limita por ruta),
  `detect` (prefiltro barato), `preconditions` (estado del mundo), `locked: true`
  (una capa inferior solo puede fortalecerla).
- **Forzar un skill (o agente) para un trabajo:** una precondición
  `builtin:skill.loaded` bloquea la acción hasta que la sesión haya cargado ese
  skill por keel — así keel NO sugiere, OBLIGA. El packet le dice al modelo cuál
  cargar (`keel.skills.load`); tras cargarlo, reintenta y pasa. Ejemplo: exigir
  `web-guide` antes de un `git`:

  ```yaml
  spec:
    on: [command.requested]
    detect: { using: "builtin:command.classify", with: { families: ["git"] } }
    preconditions:
      - using: "builtin:skill.loaded"
        with: { id: web-guide }
        onFail: block          # deny | block | review
    enforcement:
      valid: { decision: allow }
  ```

  (Preconditions builtin: `env.present`, `flag.present`, `skill.loaded`.) Nota:
  esto gobierna COMANDOS que keel ve por los shims; una escritura interna del
  cliente que no pasa por un comando no dispara la regla.
- **Trampa:** los detectores builtin (`text.regex`/`text.contains`) miran el
  CONTENIDO, no el string del comando. Para decidir sobre un comando por su
  texto, usá una tool externa (abajo).

---

## Tool — el código que decide (validate/detect/precondition)

Carpeta: `global/tools/<name>.yaml` + el script. keel corre el `command`, le
pasa el EVENTO como JSON por stdin, e interpreta la salida según `output`.

```yaml
apiVersion: keel/v1alpha1
kind: Tool
metadata: { id: no-delete-md, version: 0.1.0 }
spec:
  command: [sh, global/tools/no-delete-md.sh]   # relativo a la raíz del workspace
  timeoutMs: 5000
  output: exit-code        # exit 0 = valid | exit 1 = invalid | otro = unknown
                           # (también: verdict-json | sarif)
```

El script (referenciado arriba). Recibe el evento por stdin; decide por exit code:

```sh
#!/bin/sh
payload="$(cat)"
cmd="$(printf '%s' "$payload" | sed -n 's/.*"command"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')"
first="$(printf '%s' "$cmd" | awk '{print $1}')"
case "${first##*/}" in
  rm|unlink)
    if printf '%s' "$cmd" | grep -qiE '\.md($|[^a-zA-Z0-9])'; then exit 1; fi ;;  # .md -> block
esac
exit 0   # todo lo demás -> allow
```

- La ref desde una regla es `tool:<id>`. `chmod +x` no hace falta si invocás con
  `[sh, ...]`.
- Contrato exit-code: **0=allow, 1=block, cualquier otro=unknown** (fail-safe).

---

## Containment — el anillo duro del SO (kernel)

Carpeta: `global/containment/<name>.yaml`. Declara SOLO lo que el kernel puede
imponer, sin importar el PATH. Entra al hash del snapshot; genera el perfil del
sandbox del SO (macOS Seatbelt; Linux Landlock pendiente).

```yaml
apiVersion: keel/v1alpha1
kind: Containment
metadata: { id: global.hard.protect-docs }
spec:
  denyUnlink: ["**/*.md"]   # no se pueden borrar, ni con /bin/rm (glob exacto en macOS)
  denyWriteOutside: true    # escrituras confinadas al workspace
  denyNetwork: false
```

- Compone por UNIÓN entre capas (restricciones solo suman).
- **Cobertura por SO:** ver `CONTENCION_MULTIPLATAFORMA.md`. En Linux el glob de
  `denyUnlink` NO es kernel-hard (Landlock no tiene globs); queda shim-only.

---

## Skill — conocimiento que keel entrega al modelo

Carpetas: `global/skills/<name>.yaml` + los `.md` de contenido. El modelo la
carga vía `keel.skills.load` (MCP); keel registra el receipt.

```yaml
apiVersion: keel/v1alpha1
kind: Skill
metadata: { id: access-patterns, version: 0.1.0 }
spec:
  compact: global/skills/access-patterns_keel.md   # variante corta (primera entrega)
  full: global/skills/access-patterns-full_keel.md # opcional: variante completa (escala en oscilación)
  examples:                                        # opcional: pares para el exemplar del packet
    - ["raw SQL query", "use the query builder"]
```

- **CONDICIÓN (enforced en compile):** los archivos de contenido de una skill
  DEBEN terminar en `_keel.md`. Un `compact`/`full` que no lo cumpla es un error
  de compilación (`SkillNaming`). El sufijo hace legible la procedencia —
  entregado POR keel — dondequiera que se lea el contenido.
- El `.md` es texto libre; keel lo entrega tal cual al contexto.
- Una regla puede pedir cargarla: `enforcement.invalid.load.skills: ["skill:access-patterns"]`.

---

## ModelExecutor — un CLI local como "modelo" para agentes

Carpeta: `global/executors/<name>.yaml`. keel corre el `command`, le pasa el
prompt por stdin y toma stdout como respuesta. **NO es una API de proveedor**
(D-012): es un comando local.

```yaml
apiVersion: keel/v1alpha1
kind: ModelExecutor
metadata: { id: auditor-cli, version: 1.0.0 }
spec:
  config:
    command: [codex, exec, --json]   # o [claude, -p], o un script propio
```

---

## Agent — una responsabilidad enrutada a un executor

Carpeta: `global/agents/<name>.yaml`. El modelo lo invoca vía
`keel.agent.invoke`; keel lo corre por el scheduler y valida su salida contra el
`outputSchema` antes de confiar en ella (transversal entre modelos).

```yaml
apiVersion: keel/v1alpha1
kind: Agent
metadata: { id: auditor }
spec:
  role: audit                              # audit | review | implement
  executor: executor:auditor-cli           # el ModelExecutor que lo corre
  objective: Audit the diff for issues.
  outputSchema: global/agents/verdict.schema.json   # opcional: valida la salida (invariante 12)
```

El schema (JSON Schema estándar):

```json
{ "type": "object", "required": ["verdict"],
  "properties": { "verdict": { "type": "string" }, "note": { "type": "string" } } }
```

---

## RuleTest — prueba una regla, gate del compile

Carpeta raíz `tests/<name>.yaml` (o `projects/<name>/tests/`). `keel compile`
las corre y NO publica si fallan. Compara la decisión DECLARADA.

```yaml
apiVersion: keel/v1alpha1
kind: RuleTest
metadata: { id: global.no-delete-md.blocks-md }
spec:
  target: rule:global.no-delete-md
  event: { kind: command.requested, command: rm notes.md }
  expect: { fired: true, verdict: invalid, decision: block, origin: deterministic }
```

Autorá siempre al menos un caso block y uno allow por regla.

---

## Exception — relajar una regla `locked`, acotada

Carpeta: `global/exceptions/<name>.yaml`. La ÚNICA vía gobernada para relajar una
regla `locked`, dentro de un scope y con vencimiento; se registra como decisión
humana.

```yaml
apiVersion: keel/v1alpha1
kind: Exception
metadata: { id: reports-waiver }
spec:
  rule: rule:global.no-raw-queries       # la regla locked que se relaja
  owner: global                          # DEBE ser la capa que la lockeó
  reason: "Legacy reporting migrates next quarter."
  scope: { paths: { include: ["src/reports/**"] } }   # el lock se levanta SOLO acá
  expiry: "2027-01-01"                   # una exception vencida no hace nada
```

---

## Checklist de autoría

1. Poné el archivo en la carpeta del `kind` (renombrado, sin `.example`).
2. `keel compile` — si el schema o un RuleTest falla, corregí (el error apunta al
   campo exacto).
3. `keel lock` — fija el snapshot.
4. Para reglas nuevas: agregá su RuleTest (block + allow) antes de confiar en ella.
