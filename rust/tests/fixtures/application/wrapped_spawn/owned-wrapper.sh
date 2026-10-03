#!/bin/sh
# Synthetic detached wrapper: no provider, network, home, or credentials.
printf '%s\n' "$$" > "$SYNTHETIC_WRAPPER_ROOT/pid"
printf '%s\n' "$@" > "$SYNTHETIC_WRAPPER_ROOT/argv"
printf '%s\n' "$MANY_AI_CLI_HUB_PORT" "$MANY_AI_CLI_USAGE_PROBE" "$MANY_AI_CLI_SUBSCRIPTION_LOGIN" "$SYNTHETIC_PRECEDENCE" > "$SYNTHETIC_WRAPPER_ROOT/environment"
if [ -n "${MANY_AI_CLI_INTERNAL_SPAWN_PROOF:-}" ]; then printf 'proof-present\n'; else printf 'proof-missing\n'; fi
printf 'stdout-append\n'
printf 'stderr-append\n' >&2
if [ "${SYNTHETIC_EXIT_EARLY:-0}" = 1 ]; then exit 23; fi
polls=0
while [ ! -f "$SYNTHETIC_WRAPPER_ROOT/stop" ]; do
    polls=$((polls + 1))
    if [ "$polls" -ge 2000 ]; then exit 91; fi
    sleep 0.01
done
exit 7
