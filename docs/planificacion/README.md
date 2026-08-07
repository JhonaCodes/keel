# Planificacion canonica de Keel

Esta carpeta contiene las reglas de planificación y las órdenes de trabajo del runtime. La documentación estable del proyecto vive directamente en `docs/` y se indexa en [`docs/README.md`](../README.md).

## Regla para cualquier LLM

1. Leer este archivo y la orden correspondiente dentro de `ordenes_trabajo/` antes de proponer cambios.
2. Actualizar una fuente canonica existente; no crear copias con nombres como `future`, `phase`, `roadmap` o `implementation-plan`.
3. Separar hechos observados en codigo, decisiones, hipotesis y trabajo pendiente.
4. Cada decision debe citar una ruta del repositorio, una especificacion o una referencia externa verificable.
5. Cada trabajo debe tener criterio de aceptacion y una prueba asociada.
6. Los documentos historicos no son fuentes de verdad.
7. No adaptar la arquitectura a un proveedor especifico: Claude, Codex y otros son executors intercambiables.
8. No documentar hooks del proveedor ni MCP como mecanismo de gobierno de Keel.

## Documentación estable relacionada

- `../PROYECTO.md`: descripción completa del producto.
- `../ARQUITECTURA_RUNTIME.md`: arquitectura técnica.
- `../CONTRATOS_RUNTIME.md`: contratos ejecutables.
- `../DECISIONES.md`: decisiones cerradas.

## Órdenes de trabajo

- `ordenes_trabajo/PLAN_MAESTRO.md`: secuencia de implementación y estado honesto.
- `ordenes_trabajo/PRUEBAS_Y_ACEPTACION.md`: pruebas, gates y criterios de entrega.
- `ordenes_trabajo/HALLAZGOS.md`: desviaciones, límites y deuda explícita.
