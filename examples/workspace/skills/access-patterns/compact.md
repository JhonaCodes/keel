# Reactive state access (compact)

Never read implementation data through the notifier instance. Access state
through the provider-facing API: watch a projection (`select`) instead of
reaching into `.notifier.data`. Reads stay reactive and the ownership
boundary of the state holder is preserved.
