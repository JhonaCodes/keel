# Doctrina — herramientas en frío antes que IA

> Regla operativa que hace explícita la tesis de Keel (sección 4.4): *"la regla
> declara; la tool implementa; la tool es código"*. Un proceso decidible se
> resuelve con una tool determinista (0 tokens), no con el modelo.

## Principio

Antes de resolver algo con IA, preguntá: **¿es decidible en frío?** — es decir,
¿puede una función pura / un detector / un script dar la respuesta sin ambigüedad
y sin consumir tokens? Si la respuesta es sí, **es una tool, no una llamada al
modelo**. El modelo se reserva para lo que genuinamente requiere juicio
(evaluación semántica, sección 6.4), y aun ahí su veredicto es advisory y auditado
(sección 4.7).

Por qué importa en Keel: cada token gastado en algo decidible es coste sin
garantía (el modo de falla probabilístico que el sistema existe para eliminar).
Un detector `builtin:text.contains` cuesta microsegundos y 0 tokens; pedirle al
modelo "¿este comando borra datos?" cuesta tokens y puede fallar en silencio.

## Triage de viabilidad (antes de gastar tokens)

Para cada paso de un flujo, clasificá:

1. **Determinista** → tool/builtin (regex, clasificador de comando, parser AST,
   chequeo de env, comparación de hash). Cero tokens. Es la mayoría de los gates.
2. **Determinista pero sin tool aún** → escribir la tool (y agregarla al catálogo
   para reuso general), no improvisar con el modelo. Una tool nueva se amortiza
   en el primer reuso.
3. **Genuinamente semántico** → evaluador L3 (`keel audit`), con `outputSchema`,
   límite de tokens (inv 13) y veredicto advisory (nunca autoriza un
   irreversible, sección 4.7).

Regla de cierre: **si dudás entre script e IA para un proceso repetible y
decidible, es script.** Construí la herramienta en frío; reservá el modelo para
el juicio que no se puede codificar.

## Relación con Phase 2

La activación de capabilities, el broker de agentes y el scheduler
([`PHASE2_INITIATIVE.md`](PHASE2_INITIATIVE.md)) NO cambian esta doctrina: el
runtime, el compilador, el lock y el scheduling son **código determinista**; el
modelo solo entra donde la spec lo marca semántico. Machine learning en procesos
específicos (p. ej. clasificar intención de un comando) es una alternativa
determinista-entrenada a considerar antes que una llamada LLM — nota de
investigación, no build (ver PHASE2_INITIATIVE, sección ML).
