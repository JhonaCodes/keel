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
- [x] **PR2 · `docs/implementation-plan`** — este documento + `FUNCIONAMIENTO_INTERNO.md` + `ROADMAP.md`. PR #4 · **merged** `a12d77e`.
- [x] **PR3 · `test/harden-intervention-layers`** — #10 gaps de test (exit==2, completion-denegado, spawn real) + crate `test/`. PR #6 · **merged** `72ae1a8`.
- [x] **PR3b · `refactor/move-unit-tests-out-of-src`** — tests inline fuera de `src` vía `#[path]` (15 archivos). PR #8 · **merged** `ab4d119`.
- [x] **PR4 · `fix/surface-load-capabilities`** — #9: `capabilities` hecho honesto (surfaceado + documentado), NO removido (es forward-declaration de spec sección 11.3). PR #10 · **merged**.
- [x] **PR5 · `docs/finding-v1-deprecated-clarification`** — #11: `finding.v1` NO es gap; está **deprecado por diseño** (ADR-016). Corrección de registro.
- [x] **PR6 · `feat/repo-binding-lock`** — #2 lock + binding (`keel bind`/`keel lock`). PR #14 · **merged** `2cb8b12`.
- [x] **PR7 · `feat/compliance-ci-plane`** — #3 plano CI (`keel ci resolve`/`run` + `examples/ci/`). PR #16 · **merged** `103e5a8`.
- [x] **PR8 · `feat/adapter-capability-preflight`** — #5 preflight (`keel adapter --check`). PR #18 · **merged** `01a867f`.

**TODOS los PRs accionables están mergeados.** Quedan (por diseño, fuera de esta
iniciativa): Phase 0c (medición), broker/routing agentes + máquina de fases
(Phase 2), MCP gateway. (Monotonicidad D1–D4 §7.4: HECHA — ver #4.) Ver la sección
"Deliberadamente DIFERIDO".

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
2. Camino **completion DENEGADO** por blockers vivos (sección 12.3, `gate.rs:144-168`).
3. **Ejecución real** de un `AgentExecutor` end-to-end (hoy el invoke se
   registra-no-ejecuta y el auditor solo se prueba con stub, `audit.rs:150-156`).
**Por qué / para qué:** endurece las tres capas de intervención sin ampliar
alcance ni tocar features; sube la confianza antes de modificar el runtime.

### PR4 — Hacer honesto el campo `capabilities` (#9)  · DESVÍO JUSTIFICADO
**Qué:** en vez de **eliminarlo** (como decía el plan inicial), se **surfacea** en
la evidencia del ledger (`branch_detail`) y se **documenta** como forward-declaration.
**Por qué / para qué:** al investigar, `load.capabilities` resultó ser una
declaración deliberada del **ejemplo canónico del núcleo (sección 11.3)** y del futuro
sección 9 (economía de contexto), análoga a `invoke.agent` ("recorded, not executed").
Eliminarlo divergiría de la spec/DSL. La cura del "stub silencioso" es hacerlo
**visible + documentado** (integrar, no deprecar), no borrarlo. PR8 (#5) es OTRO
concepto de capabilities (del adapter), no reintroduce éste.

### PR5 — `finding.v1`: NO es un gap, deprecado por diseño (#11)  · CORRECCIÓN
**Qué:** corrección de registro en `ROADMAP.md`/este doc. NO se crea ningún
`finding.v1`.
**Por qué / para qué:** `finding.v1` está **ausente a propósito** — ADR-016 lo
**deprecó** en favor de SARIF como formato normativo (`sarif.rs:7`: "finding.v1
is deprecated and does NOT exist in this code"; spec :1139,:1637). Añadirlo
revivería un formato muerto (contra "no deprecado en código nuevo"). SARIF ya
emite todos los findings. El "absent" de STATUS:105 es correcto, no una falta.

### PR6 — Binding de repo + lock (#2)
**Qué:** `.keel/project.yaml` (binding del repo) + `keel.lock` (hash canónico
fijado) + comandos `keel bind` / `keel lock`; reusa la única autoridad de hash
(`hash.rs`) y un nuevo `keel-engine/src/lock.rs` + `schemas/project.schema.json`.
**Por qué / para qué:** sin lock/binding no existe plano CI y `locked` no llega a
ser garantía (inv 4/9, sección 8.6, ADR-007). El repo debe contener solo binding/lock,
no estado local.

### PR7 — Plano CI de cumplimiento (#3)
**Qué:** `keel ci resolve` / `keel ci run` reusando el engine sobre el lock, y un
step `keel` en el workflow. Depende de PR6.
**Por qué / para qué:** es donde `locked` **se vuelve garantía** (plano de
cumplimiento, sección 5.2): el job falla antes de ejecutar si el binding/lock no
resuelve. El plano local sigue siendo cooperativo (sección 5.1).

### PR8 — Manifiesto de capacidades + preflight del adapter (#5)
**Qué:** manifiesto de capacidades del adapter + **preflight** en compile que
**rechaza** una policy bloqueante que el cliente no puede honrar.
**Por qué / para qué:** evita la falsa promesa de seguridad (inv 8, sección 12.1):
declarar un `block` que el cliente nunca aplicará. Reintroduce, con consumidor
real, la idea de capacidades que PR4 quitó como stub mudo.

---

## Deliberadamente DIFERIDO (no se codea en esta iniciativa) — con justificación

Estos faltantes del inventario **no** se implementan ahora, por orden de la spec:

- **#1 Phase 0c — experimento de enforcement.** El **harness ya está construido**
  (`keel-measure` + dataset sintético v0, `test/src/measure.rs`,
  `datasets/phase0c/`): corre passive vs enforce y agrega el ledger en un reporte
  con el delta. Lo que queda es la **medición real** — correr sesiones reales y
  comparar violaciones-a-revisión con/sin `keel gate`, más la línea base honesta —,
  que no es código de producto y es el *gate de crecimiento* que la spec pone antes
  de Phase 2. Lo corre el usuario cuando decida.
- **#4 Monotonicidad de composición D1–D4 (sección 7.4).** ✅ HECHO: existe la
  composición por capas (§8.5 + §7.1) y `composition::compose` verifica D1–D4
  contra cada ancestro `locked`, rechazando todo debilitamiento
  (`MonotonicityViolation`). El lattice D3 vive en `keel-core`.
- **#6 Broker/routing de agentes** y **#8 Máquina de fases completa sección 6.2.**
  Son **Phase 2**. La spec los gate detrás de Phase 0c. Implementarlos antes de
  medir sería prematuro. (Recordatorio: hoy el `invoke.agent` de una regla solo
  se registra, nunca se ejecuta — `runtime.rs:227-233`; el único spawn real es
  `keel audit` manual.)
- **#7 MCP gateway (ADR-005).** ⏭ diferido; sin MCP en el alcance actual.

Cuando Phase 0c muestre delta material, esta lista se revisa y se promueven #6/#8
(y lo que corresponda) a una nueva iniciativa.
