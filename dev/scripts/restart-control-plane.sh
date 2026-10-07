#!/usr/bin/env sh
set -eu

# Restart only the binary supervised by PID 1. Signalling the container's PID 1
# discards its writable source and compiled binary when Kubernetes recreates it.
matched=false
for process in /proc/[0-9]*; do
  [ "${process##*/}" != 1 ] || continue
  [ -r "$process/cmdline" ] || continue
  command=$(tr '\000' '\n' < "$process/cmdline" | head -n 1)
  if [ "$command" = /opt/source/target/debug/ayni-orchestrator-control-plane ]; then
    kill -TERM "${process##*/}"
    matched=true
  fi
done
[ "$matched" = true ] || { echo 'No supervised control-plane process found.' >&2; exit 1; }
