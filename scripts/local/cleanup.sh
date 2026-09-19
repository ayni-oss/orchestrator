#!/usr/bin/env bash
set -euo pipefail

readonly repository_root="$(git rev-parse --show-toplevel)"
context="$($repository_root/scripts/local/require-docker-desktop-context.sh)"
readonly context

# Deletes only resources owned by this overlay; it intentionally leaves the
# namespace and any retained future evidence untouched.
kubectl --context "$context" delete -k "$repository_root/deploy/overlays/docker-desktop" --ignore-not-found
