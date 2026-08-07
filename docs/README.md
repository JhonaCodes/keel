# Documentación de Keel

Este directorio separa la documentación estable del proyecto de las órdenes de trabajo activas.

## Documentación del proyecto

- [`PROYECTO.md`](PROYECTO.md): descripción completa de Keel, propósito, arquitectura, ciclo cognitivo, componentes, workspace, integraciones y límites.
- [`ARQUITECTURA_RUNTIME.md`](ARQUITECTURA_RUNTIME.md): arquitectura técnica del runtime soberano y modelo de amenazas.
- [`CONTRATOS_RUNTIME.md`](CONTRATOS_RUNTIME.md): operaciones, `skill.read`, executors y scheduler.
- [`DECISIONES.md`](DECISIONES.md): decisiones arquitectónicas cerradas.
- [`USO_INSTALACION.md`](USO_INSTALACION.md): estado operativo real y experiencia objetivo sin configuracion manual.
- [`RCCA_reference_architecture_v0_9_1.md`](RCCA_reference_architecture_v0_9_1.md): especificación normativa RCCA/Keel.

## Órdenes de trabajo

Las órdenes de trabajo viven exclusivamente en [`planificacion/ordenes_trabajo/`](planificacion/ordenes_trabajo/). No deben mezclarse con la documentación estable.

- [`planificacion/README.md`](planificacion/README.md): reglas para cualquier LLM que trabaje con documentación.
- [`planificacion/ordenes_trabajo/PLAN_MAESTRO.md`](planificacion/ordenes_trabajo/PLAN_MAESTRO.md): secuencia y estado de implementación.
- [`planificacion/ordenes_trabajo/PRUEBAS_Y_ACEPTACION.md`](planificacion/ordenes_trabajo/PRUEBAS_Y_ACEPTACION.md): criterios de aceptación y gates.
- [`planificacion/ordenes_trabajo/HALLAZGOS.md`](planificacion/ordenes_trabajo/HALLAZGOS.md): desviaciones, riesgos y límites pendientes.

## Regla de navegación

Para entender el producto: leer `PROYECTO.md` y la arquitectura normativa. Para implementar trabajo: leer `planificacion/README.md` y la orden correspondiente. No crear documentos paralelos fuera de esta estructura.
