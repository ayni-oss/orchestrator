#!/bin/sh
set -eu

# NATS parses environment references as configuration expressions, not opaque
# strings. Quote and escape each Secret-backed value in memory before it sees it.
# No credential values enter configuration files, logs or command arguments.
quote_nats_string() {
  printf '%s' "$1" | LC_ALL=C awk '
    BEGIN {
      RS="\0"
      for (position=1; position<32; position++) controls[sprintf("%c",position)]=position
      controls[sprintf("%c",127)]=127
      printf "\""
    }
    {
      for (position=1; position<=length($0); position++) {
        character=substr($0,position,1)
        if (character == "\\") printf "\\\\"
        else if (character == "\"") printf "\\\""
        else if (character in controls) printf "\\x%02x", controls[character]
        else printf "%s",character
      }
    }
    END { printf "\"" }
  '
}

: "${WORKFLOW_ADMIN_PASSWORD:?broker password is missing}"
: "${WORKFLOW_APPLICATION_PASSWORD:?broker password is missing}"
: "${WORKFLOW_EXECUTOR_PASSWORD:?broker password is missing}"
: "${WORKFLOW_ACCEPTANCE_PASSWORD:?broker password is missing}"
export WORKFLOW_ADMIN_PASSWORD="$(quote_nats_string "$WORKFLOW_ADMIN_PASSWORD")"
export WORKFLOW_APPLICATION_PASSWORD="$(quote_nats_string "$WORKFLOW_APPLICATION_PASSWORD")"
export WORKFLOW_EXECUTOR_PASSWORD="$(quote_nats_string "$WORKFLOW_EXECUTOR_PASSWORD")"
export WORKFLOW_ACCEPTANCE_PASSWORD="$(quote_nats_string "$WORKFLOW_ACCEPTANCE_PASSWORD")"
exec nats-server "$@"
