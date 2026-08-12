#!/usr/bin/env bash
# Runs on bulky-server, fed to `bash -s` over SSH stdin by the CI deploy step with
# the secrets already exported into this shell. Never executed locally, and never
# passed as an SSH argument -- on a command line these values sit in the host's
# process list for the length of the deploy.
#
# It exists as a file rather than a heredoc in the workflow so it can be read,
# shellchecked and changed without touching YAML quoting rules.
set -euo pipefail

COMPOSE="docker compose -f /opt/son/docker-compose.yml"
SERVICE=son

# What is running right now, so there is something to go back to. Read before the
# pull, because the pull is what makes the new image available and the `up` is what
# makes it live.
#
# `.Config.Image` on the container rather than the compose file's ${SON_IMAGE}: the
# file has already been overwritten by the scp above with the new one.
prev_image="$($COMPOSE ps -q "$SERVICE" 2>/dev/null \
  | xargs -r docker inspect --format '{{.Config.Image}}' 2>/dev/null || true)"

$COMPOSE pull
# --remove-orphans is load-bearing here, not hygiene: this compose file went from
# four services to one, and without it the old son-app/gemini/keeper/cloudflared
# containers keep running beside the merged one -- two cloudflared processes sharing
# one tunnel token, which is the "two origins on one tunnel" failure that once had
# Cloudflare load-balancing between an old stack and a new one.
$COMPOSE up -d --remove-orphans

# Wait for the container to say it is serving, and put the old one back if it never
# does.
#
# Without this a broken image replaced a working container and the site stayed down
# until someone pushed again -- the deploy reported success either way, because
# `docker compose up` succeeds when the container *starts*, which is a much weaker
# claim than "the site answers". It matters more now than it did: one container
# means one thing to get wrong, and four processes under a supervisor is exactly the
# kind of change that builds cleanly and does not come up.
#
# The healthcheck this reads is a TCP connect to the app's port and nothing else --
# deliberately not "is Gemini screening working", which depends on Google cookies
# that expire and repair themselves on their own schedule and would roll back a
# perfectly good deploy (see the Dockerfile).
deadline=$((SECONDS + 180))
status=starting
while [ "$SECONDS" -lt "$deadline" ]; do
  status="$($COMPOSE ps -q "$SERVICE" \
    | xargs -r docker inspect --format '{{if .State.Health}}{{.State.Health.Status}}{{else}}none{{end}}' \
    2>/dev/null || echo missing)"
  case "$status" in
    healthy) echo "deploy: healthy after $SECONDS s"; break ;;
    # No healthcheck in the image at all: nothing to wait for, and waiting three
    # minutes to conclude that would be its own kind of wrong.
    none) echo "deploy: image declares no healthcheck; not gating on it"; break ;;
    unhealthy)
      # Keep looking: a cold start can report unhealthy for its first interval
      # before the app has bound its port, and start-period only suppresses the
      # *failure count*, not the status string.
      ;;
  esac
  sleep 5
done

if [ "$status" != healthy ] && [ "$status" != none ]; then
  echo "deploy: still '$status' after 180s -- rolling back" >&2
  # The last 60 lines of a container that never came up are the only diagnosis
  # anyone gets, and they are gone the moment it is replaced.
  $COMPOSE logs --tail 60 "$SERVICE" >&2 || true
  if [ -n "$prev_image" ]; then
    echo "deploy: restoring $prev_image" >&2
    SON_IMAGE="$prev_image" $COMPOSE up -d --force-recreate "$SERVICE"
  else
    echo "deploy: nothing was running before this, so there is nothing to restore" >&2
  fi
  exit 1
fi

# Screening is reported, never gated on. A site serving with screening down is the
# documented degraded mode (uploads are held for review, not published), so this is
# a line in the deploy log rather than a failure -- but a silent one would mean
# nobody notices the seed cookies have expired.
accounts="$($COMPOSE exec -T "$SERVICE" python -c \
  'import json,urllib.request;print(json.load(urllib.request.urlopen("http://127.0.0.1:8099/health",timeout=10)).get("accounts",0))' \
  2>/dev/null || echo unknown)"
echo "deploy: gemini accounts usable: $accounts (0 or unknown => uploads are held, see /admin)"
