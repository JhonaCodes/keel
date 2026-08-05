# Reactive state access (full)

## Why the rule exists
Direct `.notifier.data` reads bypass the reactive graph: the widget does not
rebuild when state changes, and the notifier's internal representation leaks
into consumers, freezing refactors.

## Approved patterns
1. Projection: `ref.watch(provider.select((s) => s.field))` — rebuilds only
   when the projected value changes.
2. Whole-state watch: `ref.watch(provider)` when the widget genuinely renders
   most of the state.
3. One-shot read in callbacks: `ref.read(provider)` (the VALUE, never the
   notifier's internals).

## Forbidden
- `ref.read(provider.notifier).data` — reaches into the implementation.
- Caching the notifier instance to poll its fields.

If a use case does not fit the patterns above, stop and request a human
review instead of working around the rule.
