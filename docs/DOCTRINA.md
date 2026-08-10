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
3. **Genuinamente semántico** → un agente gobernado vía `keel.agent.invoke`
   (executor CLI local), con `outputSchema` validado (invariante 12) y veredicto
   advisory (nunca autoriza un irreversible, sección 4.7).

Regla de cierre: **si dudás entre script e IA para un proceso repetible y
decidible, es script.** Construí la herramienta en frío; reservá el modelo para
el juicio que no se puede codificar.

## Relación con el runtime padre (D-012)

Esta doctrina es independiente de cómo keel se relaciona con el CLI del modelo.
En el runtime padre, el enforcement determinista vive DONDE el modelo no puede
tocarlo: el broker de shims evalúa reglas (`evaluate_event`, 0 tokens) y el
sandbox del SO impone el anillo duro. El modelo solo entra donde la spec lo
marca semántico, y su salida se valida contra un contrato antes de confiarse.
El compilador, el lock, el snapshot y el scheduling son **código determinista**.

La misma lógica gobierna QUÉ se le entrega al modelo, no solo qué se le
bloquea: el enrutado de skills/reglas/agentes es determinista y auditable
(`match{terms,context,autoload}`, derivación de términos por el compilador —
D-014), no bolsa-de-palabras ni semántica difusa. La entrega en sí ocurre en
cada momento relevante (`SessionStart`, edición de archivo, comando, prompt),
no solo al inicio de la sesión (D-013, D-016) — "tener una regla disponible"
y "que se entregue en el momento correcto" son dos garantías distintas, y
ambas son código determinista, no criterio del modelo.
