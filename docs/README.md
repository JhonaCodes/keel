# Documentación de Keel

Este directorio separa la documentación estable del proyecto de las órdenes de trabajo activas.

> **Estado actual (D-012).** Keel es un runtime PADRE sobre el CLI del modelo,
> sin APIs de proveedor. Para el modelo y el flujo REALES, la fuente de verdad
> hoy es [`DECISIONES.md`](DECISIONES.md) (D-012) + [`USO_INSTALACION.md`](USO_INSTALACION.md)
> + [`DOCTRINA.md`](DOCTRINA.md). Los documentos marcados abajo con "(banner
> D-012)" conservan el diseño anterior por API con una corrección al tope; su
> reescritura integral es trabajo pendiente registrado en PRUEBAS_Y_ACEPTACION.

## Documentación del proyecto

- [`DECISIONES.md`](DECISIONES.md): decisiones arquitectónicas — **empezá acá** (D-012 = runtime padre).
- [`USO_INSTALACION.md`](USO_INSTALACION.md): instalación y flujo real (`keel <cli>`, containment, MCP, agentes).
- [`DOCTRINA.md`](DOCTRINA.md): herramientas en frío antes que IA.
- [`PROYECTO.md`](PROYECTO.md): descripción del proyecto (banner D-012).
- [`ARQUITECTURA_RUNTIME.md`](ARQUITECTURA_RUNTIME.md): arquitectura técnica (banner D-012).
- [`CONTRATOS_RUNTIME.md`](CONTRATOS_RUNTIME.md): operaciones, executors y scheduler (banner D-012).
- [`RCCA_reference_architecture_v0_9_1.md`](RCCA_reference_architecture_v0_9_1.md): especificación normativa RCCA/Keel (banner D-012).

## Órdenes de trabajo

Las órdenes de trabajo viven exclusivamente en [`planificacion/ordenes_trabajo/`](planificacion/ordenes_trabajo/). No deben mezclarse con la documentación estable.

- [`planificacion/README.md`](planificacion/README.md): reglas para cualquier LLM que trabaje con documentación.
- [`planificacion/ordenes_trabajo/PLAN_MAESTRO.md`](planificacion/ordenes_trabajo/PLAN_MAESTRO.md): secuencia y estado de implementación.
- [`planificacion/ordenes_trabajo/PRUEBAS_Y_ACEPTACION.md`](planificacion/ordenes_trabajo/PRUEBAS_Y_ACEPTACION.md): criterios de aceptación y gates.
- [`planificacion/ordenes_trabajo/HALLAZGOS.md`](planificacion/ordenes_trabajo/HALLAZGOS.md): desviaciones, riesgos y límites pendientes.

## Regla de navegación

Para entender el producto: leer `PROYECTO.md` y la arquitectura normativa. Para implementar trabajo: leer `planificacion/README.md` y la orden correspondiente. No crear documentos paralelos fuera de esta estructura.
