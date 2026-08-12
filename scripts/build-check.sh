#!/usr/bin/env bash
# Build the container images before pushing, on a host that actually has Docker.
#
# Why this exists: this dev machine has no container runtime and no sudo to install
# one, so `RUN` steps in a Dockerfile were only ever exercised by CI. Three
# consecutive CI failures came from that blind spot -- a base digest that did not
# resolve, then `awscli` having no installation candidate on the Playwright base --
# each costing ~12 minutes to learn something a build says in two.
#
# bulky-server has Docker and is reachable over Tailscale SSH, so the context is
# piped to it and built there. Nothing is pushed, tagged into the deploy, or left
# running: this only answers "does it build".
#
#   scripts/build-check.sh            # both images
#   scripts/build-check.sh keeper     # just one
#
# It deliberately does not run from the pre-push hook. A keeper build pulls a
# ~1.9GB base and takes minutes, which is the wrong tax on every push -- run it
# when a Dockerfile or a requirements file changed.
set -euo pipefail

HOST="${BUILD_HOST:-root@bulky-server}"
TARGETS=("${@:-sidecar keeper}")
# Word-split a single "sidecar keeper" argument into two targets.
read -r -a TARGETS <<< "${TARGETS[*]}"

cd "$(dirname "$0")/.."

for target in "${TARGETS[@]}"; do
  if [ ! -f "$target/Dockerfile" ]; then
    echo "build-check: no $target/Dockerfile" >&2
    exit 1
  fi

  echo "build-check: building $target on $HOST"
  # The context goes over stdin as a tarball, so nothing has to be copied to the
  # host first and nothing is left behind there. `--rm` and the prune below keep
  # the host from accumulating layers from these throwaway builds.
  if tar -cz -C "$target" . \
      | ssh "$HOST" "docker build --rm -t son-buildcheck-$target -f Dockerfile - " \
      2>&1 | tail -25
  then
    echo "build-check: $target OK"
  else
    echo "build-check: $target FAILED" >&2
    exit 1
  fi
done

echo "build-check: cleaning up on $HOST"
# Only the images this script made, and only dangling layers -- never a blanket
# prune, which would evict the images production is running from.
ssh "$HOST" '
  for t in son-buildcheck-sidecar son-buildcheck-keeper; do
    docker image rm -f "$t" >/dev/null 2>&1 || true
  done
  docker image prune -f >/dev/null 2>&1 || true
' || true

echo "build-check: done"
