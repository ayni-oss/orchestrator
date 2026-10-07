#!/usr/bin/env sh
set -eu

# Restart only the binary supervised by PID 1. Signalling the container's PID 1
# discards its writable source and compiled binary when Kubernetes recreates it.
case "$(tr '\000' '\n' < /proc/1/cmdline)" in
  *'while :; do'*'/opt/source/target/debug/ayni-orchestrator-control-plane &'*) ;;
  *) echo 'Control plane is not running under the development supervisor.' >&2; exit 1 ;;
esac

# Concurrent live updates can arrive during the supervisor's restart delay.
# Wait at most five seconds for its next child, tolerating a child exit race.
attempt=0
while [ "$attempt" -le 10 ]; do
  for process in /proc/[0-9]*; do
    [ "${process##*/}" != 1 ] || continue
    [ -r "$process/cmdline" ] || continue
    command=$(tr '\000' '\n' < "$process/cmdline" 2>/dev/null | head -n 1)
    if [ "$command" = /opt/source/target/debug/ayni-orchestrator-control-plane ]; then
      if kill -TERM "${process##*/}" 2>/dev/null; then
        exit 0
      fi
    fi
  done
  [ "$attempt" -lt 10 ] || break
  attempt=$((attempt + 1))
  sleep 0.5
done
echo 'No supervised control-plane process found within five seconds.' >&2
exit 1
