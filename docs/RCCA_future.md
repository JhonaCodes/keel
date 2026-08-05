# RCCA — Extensiones organizacionales a escala

**Estado:** material diferido desde la especificación núcleo v0.9
**Condición de activación:** este documento se detalla y promueve a especificación cuando la Fase 0 demuestre delta material y las Fases 1–2 produzcan datos de operación (ADR-020). Hasta entonces, registra intención y restricciones de diseño, no contratos.

El modelo de composición y la semántica de monotonicidad de `locked` (núcleo, sección 7) son idénticos en todos los perfiles. Lo que este material añade es **distribución, firma, administración e identidad**, no semántica nueva de reglas.

---

## 1. Perfil Enterprise

Sobre el perfil Team del núcleo, la organización añade:

- Control Plane remoto como fuente de verdad distribuida;
- catálogo firmado de packages, rules y workflows;
- políticas bloqueantes administradas centralmente;
- permisos y roles;
- distribución de paquetes;
- auditoría central;
- ejecución obligatoria en el plano de cumplimiento.

Apagar un equipo local no elimina configuración: la fuente de verdad permanece en el workspace versionado o en el Control Plane.

## 2. Control Plane

```text
RCCA Control Plane
├── organizaciones
├── políticas
├── paquetes
├── firmas
└── auditoría central
```

Restricciones de diseño ya decididas:

- el Control Plane no introduce una segunda semántica: publica los mismos objetos del DSL, firmados;
- el flujo editar → validar → persistir → compilar → publicar snapshot es idéntico al local;
- la caída del Control Plane degrada a last-known-good local, nunca a sesión sin gobierno.

## 3. Identidad de desarrollador y permisos por persona

El núcleo resuelve por identidad de repositorio y declara explícitamente que no expresa permisos por persona. Esta extensión añade: roles (quién puede aprobar un objeto `Exception`, quién certifica un workflow, quién administra el catálogo), integración con el proveedor de identidad de la organización, y separación empleado/contractor. Requisito: los permisos por persona operan en el plano de cumplimiento; el plano local no gana capacidad de resistir elusión por incorporar identidad.

## 4. Atestación local fuerte

Firma de evidencia con claves fuera del alcance del usuario del runtime, verificación server-side de la cadena de evidencia, y detección de manipulación de binding/lock desde infraestructura no controlada por el desarrollador. Proyecto propio, con su propio modelo de amenaza; el núcleo lo excluye deliberadamente (sección 5.1 del núcleo).

## 5. Certificación de workflows equivalentes

```text
Nui standard workflow ─┐
                       ├── Production Readiness Contract
Progressive RCCA flow ─┘
```

Un workflow alternativo se certifica contra: benchmarks; tasa de findings; evidencia; coste; reproducibilidad; políticas corporativas.

Qué se unifica obligatoriamente: políticas no negociables; contratos de salida; schemas de evidencia; capabilities permitidas; reglas de seguridad; criterios de aceptación; entrega. No es obligatorio unificar los pasos internos del workflow.

Personalización permitida por profile: modelo; cliente; workflow autorizado; TDD/SDD si ambos están permitidos; nivel de autonomía; presentación. No modificable: seguridad bloqueante; arquitectura locked; evidencia obligatoria; permisos corporativos; requisitos de delivery.

## 6. Registro corporativo de repositorios

La organización mantiene un registro firmado (`repositories.yaml` con firma). Si un repositorio corporativo elimina o altera su binding, el plano de cumplimiento: bloquea la integración; exige regeneración del lock; reporta configuración inválida. El plano local solo degrada y registra (núcleo, 13.3). El LLM no interviene en esta decisión.

## 7. Panel web

Consola local o remota: ver configuración efectiva; editar definiciones; validar schemas; diff antes de aplicar; compilar; revisar conflictos y violaciones de monotonicidad; inspeccionar sesiones; findings y evidencia por clase de origen; probar tools; comprobar adapters, executors y bindings; inspeccionar sesiones hijas y costes de delegación; cancelar una AgentInvocation; rollback al último snapshot válido.

El panel no mantiene una segunda fuente de verdad:

```text
editar → validar → persistir en workspace/control plane → compilar → publicar snapshot
```

## 8. CI de referencia (se promueve al núcleo en Fase 2)

```yaml
name: RCCA Audit
on:
  pull_request:
  push:
    branches: [main]
jobs:
  rcca:
    runs-on: ubuntu-latest
    permissions:
      contents: read
      pull-requests: write
    steps:
      - uses: actions/checkout@v4
      - name: Install RCCA
        run: rcca-install
      - name: Authenticate
        env:
          RCCA_TOKEN: ${{ secrets.RCCA_TOKEN }}
        run: rcca auth login --token "$RCCA_TOKEN"
      - name: Validate binding and lock
        run: rcca ci resolve
      - name: Run configured RCCA workflow
        run: rcca ci run
      - name: Upload evidence
        if: always()
        uses: actions/upload-artifact@v4
        with:
          name: rcca-results
          path: .artifacts/rcca/
```

CI no depende del daemon ni del MCP local del desarrollador: inicia su runtime efímero y resuelve capabilities según configuración. Si un workflow invoca agentes secundarios, CI los resuelve desde el mismo lock, y el job falla antes de ejecutar si: el executor fijado no está disponible; su versión no cumple el lock; no produce output estructurado; falta una credencial; la policy de datos prohíbe el proveedor; el sandbox requerido no puede aplicarse.

## 9. Economía de contexto (referencia)

Tres niveles de carga: nada al inicio; compact bajo demanda; full solo ante oscilación o solicitud. El runtime no expone cientos de tools simultáneamente: activa solo las capabilities aplicables al estado actual. Un agente hijo recibe contexto independiente y acotado; ni la conversación del padre se copia por defecto, ni el resultado completo del hijo se añade al contexto del padre: se devuelve el artefacto validado y una síntesis adecuada a la fase.
