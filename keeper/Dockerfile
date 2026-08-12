# syntax=docker/dockerfile:1
#
# The session keeper: a real logged-in Chromium that keeps the Gemini session
# alive and hands its cookies to the sidecar.
#
# Microsoft's Playwright image rather than building a browser install by hand: it
# ships Chromium with the ~90 shared libraries headless Chrome needs, all matched
# to the Playwright version. Pinned by digest like every other image here.
FROM mcr.microsoft.com/playwright/python:v1.62.0-noble-amd64@sha256:c906992c927a2875cb6998f7e412a6177f4852391bf37294de11e352ad29a478

# awscli for the two R2 operations this does (fetch the profile, put it back).
RUN apt-get update \
    && apt-get install -y --no-install-recommends awscli \
    && rm -rf /var/lib/apt/lists/*

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
