# syntax=docker/dockerfile:1
#
# The session keeper: a real logged-in Chromium that keeps the Gemini session
# alive and hands its cookies to the sidecar.
#
# Microsoft's Playwright image rather than building a browser install by hand: it
# ships Chromium with the ~90 shared libraries headless Chrome needs, all matched
# to the Playwright version. Pinned by digest like every other image here.
#
# Re-pin by asking the registry and then *proving the answer resolves*, because a
# digest fetched with the wrong Accept headers is a different manifest's digest
# and fails the build with a bare "not found":
#
#   H='Accept: application/vnd.oci.image.index.v1+json, \
#      application/vnd.docker.distribution.manifest.list.v2+json'
#   D=$(curl -sI -H "$H" \
#       https://mcr.microsoft.com/v2/playwright/python/manifests/v1.62.0-noble \
#       | awk '/^docker-content-digest/{print $2}' | tr -d '\r')
#   curl -so /dev/null -w '%{http_code}\n' -H "$H" \
#       "https://mcr.microsoft.com/v2/playwright/python/manifests/$D"   # want 200
#
# The multi-arch index rather than the -amd64 tag, so buildx resolves the right
# architecture itself.
FROM mcr.microsoft.com/playwright/python:v1.62.0-noble@sha256:aa81288e738725378becba5b3e06cb0f3a7f012a610e87e8d767a090ea3f740d

# No apt layer at all: R2 is reached with boto3 from requirements.txt, because
# `awscli` has no installation candidate on this image's Ubuntu base and enabling
# a repository to get it would be a worse trade than one pinned pip package.
WORKDIR /app
COPY requirements.txt ./
RUN pip install --no-cache-dir -r requirements.txt
COPY session_keeper.py ./

# Non-root, like the others. The image's own `pwuser` already owns the browser
# and its caches, so reusing it avoids re-chowning them.
USER pwuser
ENV HOME=/tmp PYTHONUNBUFFERED=1

# No port, no healthcheck: this serves nothing and answers nothing. Whether it is
# doing its job is visible where it matters -- the sidecar's /health, which the
# admin page reads.
CMD ["python", "session_keeper.py"]
