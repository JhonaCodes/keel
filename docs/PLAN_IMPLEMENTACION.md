# Keel — Plan de implementación de los faltantes (tracker vivo)

> Documento **persistente** de la iniciativa que resuelve los faltantes de
> conformidad de Keel. No se pierde por contexto: aquí vive el porqué y el para
> qué de cada cambio, la división por PRs, y qué se difiere deliberadamente.
> Fuentes: [`STATUS.md`](../STATUS.md) (matriz de conformidad),
> [`ROADMAP.md`](ROADMAP.md) (pendientes priorizados) y
> [`FUNCIONAMIENTO_INTERNO.md`](FUNCIONAMIENTO_INTERNO.md) (cómo funciona hoy).

**Idioma:** documentación en español; código, comentarios, commits, PRs e issues
en inglés (consistencia con el repo, Apache/EN).

**Proceso:** un PR por ítem, rama `type/desc`, `CI verde` (job `build + test`)
antes de mergear, merge squash automático tras CI verde, sin borrar ramas,
commits sin `Co-Authored-By`, formato `type: short` + `Closes #N`.

---

## Estado de los PRs

- [x] **PR1 · `ci/base-workflow`** — CI base (build+test requerido, fmt+clippy advisory). Issue #1 · PR #2 · **merged** `a93f78e`.
- [ ] **PR2 · `docs/implementation-plan`** — este documento + `FUNCIONAMIENTO_INTERNO.md` + `ROADMAP.md`.
- [ ] **PR3 · `test/harden-intervention-layers`** — #10 gaps de test.
- [ ] **PR4 · `fix/remove-unconsumed-capabilities`** — #9 stub silencioso.
- [ ] **PR5 · `feat/finding-v1-schema`** — #11 schema de findings.
- [ ] **PR6 · `feat/repo-binding-lock`** — #2 lock + binding.
- [ ] **PR7 · `feat/compliance-ci-plane`** — #3 plano CI de cumplimiento (depende de PR6).
- [ ] **PR8 · `feat/adapter-capability-preflight`** — #5 preflight del adapter.

---

## Qué hace cada PR y por qué

### PR1 — CI base (habilitador)
**Qué:** `.github/workflows/ci.yml` con job `build + test` (requerido) y
`fmt + clippy` (advisory, `continue-on-error`).
**Por qué / para qué:** red de seguridad de corrección para que el merge
automático nunca aterrice un árbol roto. Lint queda advisory porque el baseline
aún no está `fmt`/`clippy` limpio y un reformat de todo el repo no pertenece a
este PR; un chore posterior puede promover lint a bloqueante.
**Nota de gate:** el repo no tiene branch protection, así que el gate lo aplica
el operador esperando el check `build + test` verde antes de mergear cada PR.

### PR2 — Documentación de planificación (contexto persistente)
**Qué:** `docs/PLAN_IMPLEMENTACION.md` (este archivo), más `FUNCIONAMIENTO_INTERNO.md`
(gráfico) y `ROADMAP.md` (pendientes), ya redactados.
**Por qué / para qué:** que la planificación y su justificación no se pierdan por
contexto; una sola fuente de verdad para seguir la iniciativa entre sesiones.

### PR3 — Endurecer tests de las 3 capas (#10)
**Qué:** tests que faltaban sobre lo YA implementado:
1. Assert **directo** de `exit == 2` para un evento inner-ring que viola (hoy solo
   indirecto en `gate.rs:178-182`).
2. Camino **completion DENEGADO** por blockers vivos (§12.3, `gate.rs:144-168`).
3. **Ejecución real** de un `AgentExecutor` end-to-end (hoy el invoke se
   registra-no-ejecuta y el auditor solo se prueba con stub, `audit.rs:150-156`).
**Por qué / para qué:** endurece las tres capas de intervención sin ampliar
alcance ni tocar features; sube la confianza antes de modificar el runtime.

### PR4 — Eliminar el campo `capabilities` no consumido (#9)
**Qué:** quitar `load.capabilities` del DSL (`rule.rs`), del schema
(`rule.schema.json`) y de la compilación (`compile.rs`/`snapshot.rs`).
**Por qué / para qué:** hoy se parsea/compila/guarda pero **nunca se consume** en
runtime — un stub silencioso. Principio del proyecto: eliminar/integrar, nunca
deprecar ni dejar no-ops mudos. Volverá cableado a un consumidor real cuando se
diseñe el enforcement de capacidades (relacionado con PR8).

### PR5 — Schema `finding.v1` + emisión (#11)
**Qué:** `schemas/finding.v1.schema.json` y emitir el finding.v1 junto al SARIF
actual.
**Por qué / para qué:** formaliza el contrato de findings además de SARIF
(§11.6/ADR-016), para que un consumidor pueda validar la forma sin depender solo
del envoltorio SARIF.

### PR6 — Binding de repo + lock (#2)
**Qué:** `.keel/project.yaml` (binding del repo) + `keel.lock` (hash canónico
fijado) + comandos `keel bind` / `keel lock`; reusa la única autoridad de hash
(`hash.rs`) y un nuevo `keel-engine/src/lock.rs` + `schemas/project.schema.json`.
**Por qué / para qué:** sin lock/binding no existe plano CI y `locked` no llega a
ser garantía (inv 4/9, §8.6, ADR-007). El repo debe contener solo binding/lock,
no estado local.

### PR7 — Plano CI de cumplimiento (#3)
**Qué:** `keel ci resolve` / `keel ci run` reusando el engine sobre el lock, y un
step `keel` en el workflow. Depende de PR6.
**Por qué / para qué:** es donde `locked` **se vuelve garantía** (plano de
cumplimiento, §5.2): el job falla antes de ejecutar si el binding/lock no
resuelve. El plano local sigue siendo cooperativo (§5.1).

### PR8 — Manifiesto de capacidades + preflight del adapter (#5)
**Qué:** manifiesto de capacidades del adapter + **preflight** en compile que
**rechaza** una policy bloqueante que el cliente no puede honrar.
**Por qué / para qué:** evita la falsa promesa de seguridad (inv 8, §12.1):
declarar un `block` que el cliente nunca aplicará. Reintroduce, con consumidor
real, la idea de capacidades que PR4 quitó como stub mudo.

---

## Deliberadamente DIFERIDO (no se codea en esta iniciativa) — con justificación

Estos faltantes del inventario **no** se implementan ahora, por orden de la spec:

- **#1 Phase 0c — experimento de enforcement.** Es una **medición** (correr
  sesiones reales y comparar violaciones-a-revisión con/sin `keel gate`), no
  código. Es el *gate de crecimiento* que la spec pone antes de Phase 2. La
  infraestructura que lo mide (`declared` vs `effective` en el ledger) ya existe.
  Lo corre el usuario cuando decida.
- **#4 Monotonicidad de composición D1–D4 (§7.4).** ⏭ YAGNI: solo aplica cuando
  exista una **segunda capa de autoridad**. Hoy hay una sola. El lattice ya está
  listo en `keel-core` (`composition_stub()` documentado).
- **#6 Broker/routing de agentes** y **#8 Máquina de fases completa §6.2.**
  Son **Phase 2**. La spec los gate detrás de Phase 0c. Implementarlos antes de
  medir sería prematuro. (Recordatorio: hoy el `invoke.agent` de una regla solo
  se registra, nunca se ejecuta — `runtime.rs:212-219`; el único spawn real es
  `keel audit` manual.)
- **#7 MCP gateway (ADR-005).** ⏭ diferido; sin MCP en el alcance actual.

Cuando Phase 0c muestre delta material, esta lista se revisa y se promueven #6/#8
(y lo que corresponda) a una nueva iniciativa.
