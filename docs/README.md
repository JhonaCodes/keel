# Documentación de Keel

Este directorio separa la documentación estable del proyecto de las órdenes de trabajo activas.

> **Estado actual (D-012).** Keel es un runtime PADRE sobre el CLI del modelo,
> sin APIs de proveedor. Toda la documentación de este directorio está
> alineada con esa arquitectura (reescrita 2026-08-10) — no quedan banners de
> corrección pendiente.

## Documentación del proyecto

- [`DECISIONES.md`](DECISIONES.md): decisiones arquitectónicas — **empezá acá** (D-012 = runtime padre).
- [`USO_INSTALACION.md`](USO_INSTALACION.md): instalación y flujo real (`keel <cli>`, containment, MCP, agentes).
- [`AUTORIA.md`](AUTORIA.md): cómo crear cada tipo (Rule, Tool, Containment, Skill, Agent, ModelExecutor, RuleTest, Exception) — ejemplos copiables para humano o IA.
- [`DOCTRINA.md`](DOCTRINA.md): herramientas en frío antes que IA.
- [`CONTENCION_MULTIPLATAFORMA.md`](CONTENCION_MULTIPLATAFORMA.md): matriz macOS/Linux/Windows del anillo duro (Seatbelt/Landlock/WSL2) y plan de F2b.
- [`PROYECTO.md`](PROYECTO.md): descripción del proyecto.
- [`ARQUITECTURA_RUNTIME.md`](ARQUITECTURA_RUNTIME.md): arquitectura técnica.
- [`CONTRATOS_RUNTIME.md`](CONTRATOS_RUNTIME.md): operaciones, executors y scheduler.

## Órdenes de trabajo

Las órdenes de trabajo viven exclusivamente en [`planificacion/ordenes_trabajo/`](planificacion/ordenes_trabajo/). No deben mezclarse con la documentación estable.

- [`planificacion/README.md`](planificacion/README.md): reglas para cualquier LLM que trabaje con documentación.
- [`planificacion/ordenes_trabajo/PLAN_MAESTRO.md`](planificacion/ordenes_trabajo/PLAN_MAESTRO.md): secuencia de implementación, roadmap activo y estado honesto (incluye desviaciones, límites y deuda explícita).
- [`planificacion/ordenes_trabajo/PRUEBAS_Y_ACEPTACION.md`](planificacion/ordenes_trabajo/PRUEBAS_Y_ACEPTACION.md): criterios de aceptación y gates.

## Regla de navegación

Para entender el producto: leer `PROYECTO.md` y la arquitectura normativa. Para implementar trabajo: leer `planificacion/README.md` y la orden correspondiente. No crear documentos paralelos fuera de esta estructura.
