#!/usr/bin/env bash

# Build one project image and print its immutable local reference.
dev_build_image() {
  local component="${1:?component is required}"
  local requested_image_ref="${2:-}"
  case "$component" in
    control-plane | control-plane-dev | worker | web | web-dev) ;;
    *) echo "component must be control-plane, control-plane-dev, worker, web, or web-dev" >&2; return 2 ;;
  esac

  local dockerfile="$DEV_REPOSITORY_ROOT/deploy/images/$component/Dockerfile"
  local revision full_revision source_status source_dirty build_image_ref image_ref image_id content_id
  revision="$(git -C "$DEV_REPOSITORY_ROOT" rev-parse --short=12 HEAD)" || return
  full_revision="$(git -C "$DEV_REPOSITORY_ROOT" rev-parse HEAD)" || return
  source_status="$(git -C "$DEV_REPOSITORY_ROOT" status --porcelain)" || return
  source_dirty=false
  if [[ -n "$source_status" ]]; then
    source_dirty=true
  fi
  build_image_ref="${requested_image_ref:-local/ayni-orchestrator-$component:build-git-${revision}-$$}"

  # The worker launcher must use the node architecture so mandatory kernel
  # sandbox syscalls execute natively. Managed tools may still be emulated.
  # Callers capture stdout with $(...), where Bash may disable errexit.
  # Propagate failures explicitly rather than emitting an unusable image tag.
  docker build \
    --file "$dockerfile" \
    --label "org.opencontainers.image.revision=$full_revision" \
    --label "org.opencontainers.image.source.dirty=$source_dirty" \
    --tag "$build_image_ref" \
    "$DEV_REPOSITORY_ROOT" >&2 || return

  image_ref="$requested_image_ref"
  if [[ -z "$image_ref" ]]; then
    image_id="$(docker image inspect "$build_image_ref" --format '{{.Id}}')" || return
    if [[ ! "$image_id" =~ ^sha256:[a-f0-9]{64}$ ]]; then
      echo "built image $build_image_ref did not report a valid content ID" >&2
      return 1
    fi
    content_id="${image_id#sha256:}"
    image_ref="local/ayni-orchestrator-$component:git-${revision}-${content_id:0:12}"
    docker image tag "$build_image_ref" "$image_ref" || return
    docker image remove "$build_image_ref" >/dev/null || return
  fi
  docker image inspect "$image_ref" \
    --format 'image={{.Id}} platform={{.Os}}/{{.Architecture}} revision={{index .Config.Labels "org.opencontainers.image.revision"}}' >&2 || return
  printf '%s\n' "$image_ref"
}

# Import a host image into Docker Desktop Kubernetes' containerd store.
dev_load_image() {
  local image_ref="${1:?image reference is required}"
  local node_name="desktop-control-plane"
  local runtime_image_ref="docker.io/$image_ref"
  local node_id node_labels platform
  node_id="$(docker inspect "$node_name" --format '{{.Id}}')"
  node_labels="$(docker inspect "$node_name" --format '{{index .Config.Labels "io.x-k8s.kind.cluster"}} {{index .Config.Labels "io.x-k8s.kind.role"}}')"
  if [[ "$node_labels" != "desktop control-plane" ]]; then
    echo "Docker Desktop node $node_name is not the expected kind control-plane node" >&2
    return 1
  fi
  # Export the concrete manifest, not an index selected using the node's CPU.
  # Managed runtimes may use a different CPU architecture from the native launcher.
  platform="$(docker image inspect "$image_ref" --format '{{.Os}}/{{.Architecture}}')"
  docker save --platform "$platform" "$image_ref" \
    | docker exec -i "$node_id" ctr --namespace k8s.io images import --platform "$platform" - >&2
  if ! docker exec "$node_id" ctr --namespace k8s.io images list --quiet | grep -Fxq "$runtime_image_ref"; then
    echo "Docker Desktop node did not retain image $image_ref" >&2
    return 1
  fi
  echo "loaded image=$image_ref node=$node_name" >&2
}

# Pin the manifest loaded into the local node and retain its digest alias for Jobs.
dev_pin_worker_image() {
  local image_ref="${1:?worker image is required}"
  local loaded_ref="docker.io/$image_ref" digest pinned
  digest="$(docker exec desktop-control-plane ctr --namespace k8s.io images list "name==$loaded_ref" | awk 'NR==2 {print $3}')"
  if [[ ! "$digest" =~ ^sha256:[a-f0-9]{64}$ ]]; then
    echo "loaded worker manifest digest is unavailable" >&2
    return 1
  fi
  pinned="${loaded_ref%:*}@$digest"
  docker exec desktop-control-plane ctr --namespace k8s.io images tag --force "$loaded_ref" "$pinned" >&2
  printf '%s\n' "$pinned"
}

# Remove only obsolete project tags, never cache, volumes, or unrelated images.
dev_prune_images() (
  local control_plane_image_ref="${1:?control-plane image is required}"
  local worker_image_ref="${2:?worker image is required}"
  local web_image_ref="${3:?web image is required}"
  local node_id temporary_directory keep runtime_ref local_ref image_ref
  local pinned_worker="${4:?pinned runtime worker is required}"
  local pinned_connection_test="${5:-}"
  node_id="$(docker inspect desktop-control-plane --format '{{.Id}}')"
  temporary_directory="$(mktemp -d)"
  trap 'rm -rf "$temporary_directory"' EXIT
  keep="$temporary_directory/keep"

  printf '%s\n%s\n%s\n' "$control_plane_image_ref" "$worker_image_ref" "$web_image_ref" > "$keep"
  kubectl --context docker-desktop get pods --all-namespaces \
    -o jsonpath='{range .items[*]}{range .spec.initContainers[*]}{.image}{"\n"}{end}{range .spec.containers[*]}{.image}{"\n"}{end}{end}' \
    | sed 's#^docker.io/##' | grep '^local/ayni-orchestrator-' >> "$keep" || true
  printf '%s\n' "${pinned_worker#docker.io/}" >> "$keep"
  [[ -z "$pinned_connection_test" ]] || printf '%s\n' "${pinned_connection_test#docker.io/}" >> "$keep"
  sort -u "$keep" -o "$keep"

  while IFS= read -r runtime_ref; do
    local_ref="${runtime_ref#docker.io/}"
    if ! grep -Fxq "$local_ref" "$keep"; then
      docker exec "$node_id" ctr --namespace k8s.io images remove "$runtime_ref" >/dev/null
      echo "removed obsolete node image $local_ref" >&2
    fi
  done < <(docker exec "$node_id" ctr --namespace k8s.io images list --quiet | grep '^docker.io/local/ayni-orchestrator-' || true)

  while IFS= read -r image_ref; do
    if ! grep -Fxq "$image_ref" "$keep"; then
      docker image remove "$image_ref" >/dev/null
      echo "removed obsolete host image $image_ref" >&2
    fi
  done < <(docker image list --format '{{.Repository}}:{{.Tag}}' | grep '^local/ayni-orchestrator-' || true)
)
